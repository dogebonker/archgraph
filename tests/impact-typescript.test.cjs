'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createRequire } = require('node:module');
const { resolve } = require('../src/impact_typescript.cjs');
let compiler = process.env.ARCHGRAPH_TYPESCRIPT;
if (!compiler) {
  try { compiler = createRequire(__filename).resolve('typescript/package.json'); } catch {}
}

test('symbol resolution distinguishes nested barrel exports and keeps namespace or dynamic consumers', { skip: !compiler && 'requires installed TypeScript (ARCHGRAPH_TYPESCRIPT)' }, t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'archgraph-impact-symbols-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const sources = {
    'a.ts': 'export function a() { return 1; }',
    'b.ts': 'export default function b() { return 2; }',
    'nested/index.ts': 'export { a as alpha } from "../a"; export { default } from "../b";',
    'index.ts': 'export { alpha, default } from "./nested";',
    'consumer.ts': 'import { alpha } from "./index"; alpha();',
    'default.ts': 'import b from "./index"; b();',
    'namespace.ts': 'import * as all from "./index"; all.alpha();',
    'side.ts': 'import "./index";',
    'missing.ts': 'import { missing } from "./index"; missing();',
    'dynamic.ts': 'import { alpha } from "./index"; alpha(); import("./index");',
    'require.ts': 'import { alpha } from "./index"; alpha(); require("./index");',
    'import-type.ts': 'import { alpha } from "./index"; alpha(); type Other = import("./index").Other;',
    'mixed.ts': 'import { alpha } from "./index"; alpha(); export * from "./index";',
    'tsconfig.json': JSON.stringify({ compilerOptions: { target: 'es2020', module: 'esnext', moduleResolution: 'bundler' } }),
  };
  for (const [file, source] of Object.entries(sources)) {
    fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
    fs.writeFileSync(path.join(root, file), source);
  }
  const files = Object.keys(sources).filter(file => file.endsWith('.ts')).map(file => ({ path: file, barrel: file.endsWith('index.ts') }));
  const input = { root, depcruise: compiler, files };
  const expected = [
    { from: 'consumer.ts', barrel: 'index.ts', targets: ['a.ts'] },
    { from: 'default.ts', barrel: 'index.ts', targets: ['b.ts'] },
  ];
  assert.deepEqual(resolve(input), expected);
  const child = require('node:child_process').spawnSync(process.execPath, ['-e', fs.readFileSync(path.join(__dirname, '../src/impact_typescript.cjs'), 'utf8')], { input: JSON.stringify(input), encoding: 'utf8' });
  assert.equal(child.status, 0, child.stderr);
  assert.deepEqual(JSON.parse(child.stdout), expected);
});
