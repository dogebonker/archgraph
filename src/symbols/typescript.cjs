'use strict';
const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
function collect(input) {
  const root = fs.realpathSync(input.root);
  const candidates = input.depcruise ? [] : [path.join(root, 'package.json')];
  if (input.depcruise) {
    const locations = input.depcruise.includes(path.sep) ? [input.depcruise] : (process.env.PATH || '').split(path.delimiter).map(dir => path.join(dir, input.depcruise));
    for (const location of locations) if (fs.existsSync(location)) { candidates.unshift(fs.realpathSync(location)); break; }
  }
  let ts;
  for (const candidate of candidates) {
    try { ts = createRequire(candidate)('typescript'); break; } catch (error) { if (error.code !== 'MODULE_NOT_FOUND') throw error; }
  }
  if (!ts || ![ts.createProgram, ts.readConfigFile, ts.parseJsonConfigFileContent].every(fn => typeof fn === 'function')) throw new Error('TypeScript compiler API unavailable; install TypeScript in the repository or beside dependency-cruiser');
  const files = new Map(input.files.map(file => [path.resolve(root, file), file]));
  const relative = filename => files.get(path.resolve(filename));
  let options = { module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler ?? ts.ModuleResolutionKind.NodeJs };
  const configPath = path.join(root, 'tsconfig.json');
  if (fs.existsSync(configPath)) {
    const config = ts.readConfigFile(configPath, ts.sys.readFile);
    if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, '\n'));
    const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, root);
    const errors = parsed.errors.filter(error => error.code !== 18003);
    if (errors.length) throw new Error(errors.map(error => ts.flattenDiagnosticMessageText(error.messageText, '\n')).join('\n'));
    options = { ...options, ...parsed.options };
  }
  const program = ts.createProgram({ rootNames: [...files.keys()], options: { ...options, noEmit: true, allowJs: true, skipLibCheck: true } });
  const checker = program.getTypeChecker();
  const result = { exports: [], imports: [], limitations: [] };
  const limitations = new Set();
  function target(symbol) {
    const seen = new Set();
    while (symbol && symbol.flags & ts.SymbolFlags.Alias && !seen.has(symbol)) { seen.add(symbol); symbol = checker.getAliasedSymbol(symbol); }
    const declarations = symbol?.declarations || [];
    const targets = declarations.map(declaration => relative(declaration.getSourceFile().fileName)).filter(Boolean);
    if (new Set(targets).size !== 1 || !symbol || !declarations.length) return null;
    return { file: targets[0], name: symbol.getName() };
  }
  for (const [filename, file] of files) {
    const source = program.getSourceFile(filename);
    if (!source) { limitations.add(`Cannot parse ${file}`); continue; }
    if (source.parseDiagnostics.length) limitations.add(`Syntax errors in ${file}; symbol coverage is incomplete`);
    const module = checker.getSymbolAtLocation(source);
    if (module) for (const symbol of checker.getExportsOfModule(module)) {
      const resolved = target(symbol);
      if (resolved) result.exports.push({ file, name: symbol.getName(), target: resolved });
      else limitations.add(`Unresolved export ${file}:${symbol.getName()}`);
    }
    function add(specifier, bindings) {
      const resolved = ts.resolveModuleName(specifier.text, filename, options, ts.sys).resolvedModule;
      const to = resolved && relative(resolved.resolvedFileName);
      if (!to) {
        if (!resolved || specifier.text.startsWith('.') || resolved && path.resolve(resolved.resolvedFileName).startsWith(root + path.sep) && !resolved.isExternalLibraryImport) limitations.add(`Import outside scanned symbols: ${file} -> ${specifier.text}`);
        return;
      }
      for (const [name, node, wildcard] of bindings) {
        const resolvedTarget = wildcard ? null : target(checker.getSymbolAtLocation(node));
        result.imports.push({ from: file, to, name, target: resolvedTarget, wildcard });
        if (wildcard) limitations.add(`Namespace import or re-export ${file} -> ${to} is conservative`);
        else if (!resolvedTarget) limitations.add(`Unresolved import ${file}:${name}`);
      }
    }
    function visit(node) {
      if (ts.isBinaryExpression(node) && node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment && node.operatorToken.kind <= ts.SyntaxKind.LastAssignment) {
        let base = node.left;
        while (ts.isPropertyAccessExpression(base) || ts.isElementAccessExpression(base)) base = base.expression;
        if (ts.isIdentifier(base) && ['module', 'exports'].includes(base.text)) limitations.add(`CommonJS property exports in ${file}; exports are incomplete`);
      }
      if (ts.isImportDeclaration(node) && ts.isStringLiteralLike(node.moduleSpecifier)) {
        const bindings = [];
        const clause = node.importClause;
        if (clause?.name) bindings.push(['default', clause.name, false]);
        if (clause?.namedBindings) {
          if (ts.isNamespaceImport(clause.namedBindings)) bindings.push(['*', clause.namedBindings.name, true]);
          else for (const binding of clause.namedBindings.elements) bindings.push([(binding.propertyName || binding.name).text, binding.name, false]);
        }
        if (!bindings.length) bindings.push(['*', null, true]);
        add(node.moduleSpecifier, bindings);
      } else if (ts.isExportDeclaration(node) && node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)) {
        const bindings = node.exportClause && ts.isNamedExports(node.exportClause) ? node.exportClause.elements.map(binding => [(binding.propertyName || binding.name).text, binding.name, false]) : [['*', null, true]];
        add(node.moduleSpecifier, bindings);
      } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) && ts.isStringLiteralLike(node.argument.literal)) {
        add(node.argument.literal, [['*', null, true]]);
      } else if (ts.isExportAssignment(node) && node.isExportEquals) {
        limitations.add(`CommonJS export assignment in ${file}; exports are incomplete`);
      } else if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword || ts.isIdentifier(node.expression) && node.expression.text === 'require')) {
        if (node.arguments[0] && ts.isStringLiteralLike(node.arguments[0])) add(node.arguments[0], [['*', null, true]]);
        else limitations.add(`Dynamic import in ${file} cannot be resolved statically`);
      } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference) && node.moduleReference.expression) add(node.moduleReference.expression, [['*', null, true]]);
      ts.forEachChild(node, visit);
    }
    visit(source);
  }
  result.limitations = [...limitations].sort();
  result.complete = !result.limitations.some(message => !message.startsWith('Namespace import or re-export '));
  return result;
}
if (!module.parent) {
  try { process.stdout.write(JSON.stringify(collect(JSON.parse(fs.readFileSync(0, 'utf8'))))); }
  catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
module.exports = { collect };
