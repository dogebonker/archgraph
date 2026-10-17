'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');

function compiler(root, program) {
  const candidates = [];
  if (program) {
    const locations = program.includes(path.sep) ? [program] : (process.env.PATH || '').split(path.delimiter).map(dir => path.join(dir, program));
    for (const location of locations) {
      if (fs.existsSync(location)) {
        candidates.push(fs.realpathSync(location));
        break;
      }
    }
  }
  candidates.push(path.join(root, 'package.json'));
  for (const candidate of candidates) {
    try { return createRequire(candidate)('typescript'); } catch (error) {
      if (error.code !== 'MODULE_NOT_FOUND') throw error;
    }
  }
  throw new Error('TypeScript compiler unavailable; install TypeScript beside dependency-cruiser or in the scanned repository');
}

function resolve(input) {
  const root = fs.realpathSync(input.root);
  const ts = compiler(root, input.depcruise);
  if (![ts.createProgram, ts.readConfigFile, ts.parseJsonConfigFileContent].every(fn => typeof fn === 'function')) throw new Error('Installed TypeScript does not expose the classic compiler API');
  const files = new Map(input.files.map(file => [path.resolve(root, file.path), file]));
  const relative = filename => files.get(path.resolve(filename))?.path;
  let options = { module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler };
  const configPath = path.join(root, 'tsconfig.json');
  if (fs.existsSync(configPath)) {
    const config = ts.readConfigFile(configPath, ts.sys.readFile);
    if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, '\n'));
    const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, root);
    // Source-file diagnostics do not prevent alias resolution; invalid compiler options do.
    const errors = parsed.errors.filter(error => error.code !== 18003);
    if (errors.length) throw new Error(errors.map(error => ts.flattenDiagnosticMessageText(error.messageText, '\n')).join('\n'));
    options = { ...options, ...parsed.options };
  }
  const program = ts.createProgram({ rootNames: [...files.keys()], options: { ...options, noEmit: true, allowJs: true, skipLibCheck: true } });
  const checker = program.getTypeChecker();
  const pure = new Set();
  for (const [filename, file] of files) {
    const source = program.getSourceFile(filename);
    if (file.barrel && source && source.statements.length && source.statements.every(statement => ts.isEmptyStatement(statement) || ts.isExportDeclaration(statement) && statement.moduleSpecifier)) pure.add(file.path);
  }
  const replacements = [];
  for (const [filename, file] of files) {
    const source = program.getSourceFile(filename);
    if (!source || file.barrel) continue;
    const imports = new Map();
    for (const statement of source.statements) {
      if (!ts.isImportDeclaration(statement) || !ts.isStringLiteralLike(statement.moduleSpecifier)) continue;
      const resolved = ts.resolveModuleName(statement.moduleSpecifier.text, filename, options, ts.sys).resolvedModule;
      const barrel = resolved && relative(resolved.resolvedFileName);
      if (!pure.has(barrel)) continue;
      const entry = imports.get(barrel) || { valid: true, targets: new Set() };
      imports.set(barrel, entry);
      const clause = statement.importClause;
      if (!clause || clause.namedBindings && ts.isNamespaceImport(clause.namedBindings)) { entry.valid = false; continue; }
      const bindings = [];
      if (clause.name) bindings.push(clause.name);
      if (clause.namedBindings) for (const binding of clause.namedBindings.elements) bindings.push(binding.name);
      if (!bindings.length) { entry.valid = false; continue; }
      for (const binding of bindings) {
        let symbol = checker.getSymbolAtLocation(binding);
        const seen = new Set();
        while (symbol && symbol.flags & ts.SymbolFlags.Alias && !seen.has(symbol)) {
          seen.add(symbol);
          symbol = checker.getAliasedSymbol(symbol);
        }
        const declarations = symbol?.declarations;
        if (!declarations?.length) { entry.valid = false; continue; }
        for (const declaration of declarations) {
          const target = relative(declaration.getSourceFile().fileName);
          if (!target || pure.has(target)) entry.valid = false;
          else entry.targets.add(target);
        }
      }
    }
    // Other access to the same barrel still depends on its whole namespace.
    function inspect(node) {
      let specifier;
      if (ts.isExportDeclaration(node)) specifier = node.moduleSpecifier;
      if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) specifier = node.argument.literal;
      if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword || ts.isIdentifier(node.expression) && node.expression.text === 'require')) specifier = node.arguments[0];
      if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) specifier = node.moduleReference.expression;
      if (specifier && ts.isStringLiteralLike(specifier)) {
        const resolved = ts.resolveModuleName(specifier.text, filename, options, ts.sys).resolvedModule;
        const barrel = resolved && relative(resolved.resolvedFileName);
        if (imports.has(barrel)) imports.get(barrel).valid = false;
      }
      ts.forEachChild(node, inspect);
    }
    inspect(source);
    for (const [barrel, entry] of imports) {
      if (entry.valid && entry.targets.size) replacements.push({ from: file.path, barrel, targets: [...entry.targets].sort() });
    }
  }
  return replacements;
}

if (!module.parent) {
  try { process.stdout.write(JSON.stringify(resolve(JSON.parse(fs.readFileSync(0, 'utf8'))))); }
  catch (error) { process.stderr.write(`${error.message}\n`); process.exitCode = 1; }
}
module.exports = { resolve };
