'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { collect } = require('../src/symbols/typescript.cjs');
function fixture(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'archgraph-symbols-'));
  for (const [file, contents] of Object.entries(files)) { fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true }); fs.writeFileSync(path.join(root, file), contents); }
  return root;
}
function python(files) {
  const root = fixture(files);
  try {
    const output = spawnSync('python3', ['-I', '-B', path.resolve(__dirname, '../src/symbols/python.py')], { input: JSON.stringify({ root, files: Object.keys(files) }), encoding: 'utf8' });
    assert.equal(output.status, 0, output.stderr);
    return JSON.parse(output.stdout);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
}
test('Python follows aliases, literal __all__ additions, star imports and nested consumers', () => {
  const result = python({
    'pkg/model.py': 'class Order: pass\nclass Secret: pass\n',
    'pkg/__init__.py': 'from .model import Order as PublicOrder\nBASE = ["PublicOrder"]\n__all__ = BASE + []\n',
    'app.py': 'from pkg import *\ndef f():\n    from pkg.model import Secret\n',
  });
  assert.equal(result.complete, true);
  assert.deepEqual(result.exports.filter(item => item.file === 'pkg/__init__.py'), [{ file: 'pkg/__init__.py', name: 'PublicOrder', target: { file: 'pkg/model.py', name: 'Order' } }]);
  assert.ok(result.imports.some(item => item.from === 'app.py' && item.wildcard));
  assert.ok(result.imports.some(item => item.name === 'Secret' && item.target.name === 'Secret'));
});
test('Python source modules cannot shadow the collector standard library', () => {
  const result = python({ 'ast.py': 'raise RuntimeError("executed repository ast.py")\n', 'pkg.py': 'class Order: pass\n' });
  assert.ok(result.exports.some(item => item.name === 'Order'));
});
test('Python dynamic __all__ and unresolved explicit exports withhold completeness', () => {
  const result = python({ 'pkg/__init__.py': '__all__ = get_names()\n', 'broken.py': '__all__ = ["missing"]\n' });
  assert.equal(result.complete, false);
  assert.equal(result.limitations.length, 2);
});
test('Python star re-exports preserve original symbol identities', () => {
  const result = python({ 'pkg/model.py': 'class Order: pass\n_hidden = 1\n', 'pkg/__init__.py': 'from .model import *\n', 'app.py': 'from pkg import Order\n' });
  assert.equal(result.complete, true);
  assert.deepEqual(result.imports.find(item => item.from === 'app.py').target, { file: 'pkg/model.py', name: 'Order' });
});
test('Python external re-exports and mutated exports remain explicit coverage gaps', () => {
  const result = python({ 'pkg.py': 'from external import thing\n__all__ = ["thing"]\n__all__.append("later")\n' });
  assert.equal(result.complete, false);
  assert.ok(result.limitations.some(message => message.includes('external')));
  assert.ok(result.limitations.some(message => message.includes('Mutated')));
});
let compilerPath = process.env.ARCHGRAPH_TYPESCRIPT;
if (!compilerPath) { try { compilerPath = require.resolve('typescript/package.json'); } catch {} }
test('TypeScript resolves paths, aliases, star and default re-exports', { skip: !compilerPath || !fs.existsSync(compilerPath) }, () => {
  const files = {
    'tsconfig.json': JSON.stringify({ compilerOptions: { baseUrl: '.', paths: { '@pkg': ['pkg/index.ts'] } } }),
    'pkg/model.ts': 'export class Order {}\nexport default class DefaultOrder {}\n',
    'pkg/index.ts': "export { Order as PublicOrder, default } from './model';\n",
    'app.ts': "import DefaultOrder, { PublicOrder as Local } from '@pkg';\n",
    'star.ts': "export * from './pkg/index';\n",
  };
  const root = fixture(files);
  try {
    const result = collect({ root, files: Object.keys(files).filter(file => file.endsWith('.ts')), depcruise: compilerPath });
    assert.equal(result.complete, true, result.limitations.join('\n'));
    assert.deepEqual(result.exports.find(item => item.file === 'pkg/index.ts' && item.name === 'PublicOrder').target, { file: 'pkg/model.ts', name: 'Order' });
    assert.deepEqual(result.imports.find(item => item.from === 'app.ts' && item.name === 'PublicOrder').target, { file: 'pkg/model.ts', name: 'Order' });
    assert.ok(result.imports.find(item => item.from === 'app.ts' && item.name === 'default').target);
    assert.ok(result.exports.some(item => item.file === 'star.ts' && item.name === 'PublicOrder'));
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
test('Python namespace re-exports retain aliases and spelled package entrypoints', () => {
  const result = python({
    'pkg/child.py': 'class Item: pass\n',
    'pkg/__init__.py': 'from . import child\n__all__ = ["child"]\n',
    'alias.py': 'import pkg.child as child\n__all__ = ["child"]\n',
    'app.py': 'from pkg import child\n',
  });
  assert.equal(result.complete, false);
  for (const file of ['pkg/__init__.py', 'alias.py']) assert.deepEqual(result.exports.find(item => item.file === file && item.name === 'child').target, { file: 'pkg/child.py', name: '*' });
  assert.deepEqual(result.imports.find(item => item.from === 'app.py'), { from: 'app.py', to: 'pkg/__init__.py', name: 'child', target: { file: 'pkg/child.py', name: '*' }, wildcard: false });
});
test('Python match bindings and mutable __all__ aliases are incomplete', () => {
  const result = python({ 'match.py': 'match 1:\n    case item:\n        A = item\n', 'pkg.py': 'class A: pass\nclass B: pass\nPUBLIC = ["A"]\n__all__ = PUBLIC\nPUBLIC.append("B")\n' });
  assert.equal(result.complete, false);
  assert.ok(result.limitations.some(message => message.includes('Conditional')));
  assert.ok(result.limitations.some(message => message.includes('constant alias')));
});
test('TypeScript CommonJS property exports explicitly withhold completeness', { skip: !compilerPath || !fs.existsSync(compilerPath) }, () => {
  const root = fixture({ 'api.js': 'module.exports = { Foo: class Foo {} }; exports.Bar = class Bar {};\n' });
  try {
    const result = collect({ root, files: ['api.js'], depcruise: compilerPath });
    assert.equal(result.complete, false);
    assert.ok(result.limitations.some(message => message.includes('CommonJS')));
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
test('Explicit TypeScript tool path never falls back to repository compiler code', () => {
  const root = fixture({ 'node_modules/typescript/index.js': 'throw new Error("executed repository compiler");', 'api.ts': 'export class Foo {}' });
  try {
    assert.throws(() => collect({ root, files: ['api.ts'], depcruise: path.join(os.tmpdir(), 'nonexistent-archgraph-tool') }), /TypeScript compiler API unavailable/);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
test('Python package attributes take precedence over same-name submodules', () => {
  const result = python({
    'pkg/__init__.py': 'from .api import thing\n',
    'pkg/api.py': 'def thing(): pass\n',
    'pkg/thing.py': 'class Unrelated: pass\n',
    'app.py': 'from pkg import thing\n',
  });
  assert.deepEqual(result.imports.find(item => item.from === 'app.py').target, { file: 'pkg/api.py', name: 'thing' });
  assert.deepEqual(result.exports.find(item => item.file === 'app.py' && item.name === 'thing').target, { file: 'pkg/api.py', name: 'thing' });
});
