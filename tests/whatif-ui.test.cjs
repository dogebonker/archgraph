'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const engine = require('../assets/whatif.js');
const fixtures = require('./fixtures/whatif-parity.json');
const code = fs.readFileSync(path.join(__dirname, '../assets/whatif-ui.js'), 'utf8');
function node(tag = 'div') {
  return { tag, attrs: {}, listeners: {}, childNodes: [], hidden: false, value: '',
    append(...nodes) { this.childNodes.push(...nodes); },
    replaceChildren(...nodes) { this.childNodes = nodes; this.text = ''; },
    setAttribute(key, value) { this.attrs[key] = value; },
    addEventListener(key, fn) { this.listeners[key] = fn; },
    focus() { this.focused = true; }, select() { this.selected = true; },
    set textContent(value) { this.text = String(value); this.childNodes = []; },
    get textContent() { return (this.text || '') + this.childNodes.map(n => n.textContent).join(' '); },
    set innerHTML(_) { throw Error('Untrusted text must not become markup'); }
  };
}
function setup(name = 'cut-module-cycle') {
  const doc = structuredClone(fixtures.find(f => f.name === name).document);
  const mods = doc.modules, modIndex = new Map(mods.map((m, i) => [m.id, i]));
  const files = doc.files.map((f, i) => ({ ...f, i, mod: modIndex.get(f.module), x: 10 + i, y: 20 + i }));
  const mx = mods.map((_, i) => i * 100), my = mods.map(() => 0);
  const lx = files.map(f => f.x - mx[f.mod]), ly = files.map(f => f.y);
  const controls = new Map();
  const $ = id => { if (!controls.has(id)) controls.set(id, node()); return controls.get(id); };
  $('stats').append(Object.assign(node(), { textContent: 'Original stats' }));
  $('stats-text').textContent = 'Original stats';
  $('whatif-graph').value = 'graph.json';
  const el = (tag, attrs, text) => { const e = node(tag); e.attrs = attrs; if (text !== undefined) e.textContent = text; return e; };
  const fileIndex = new Map(files.map(f => [f.path, f.i]));
  const edges = doc.edges.map((e, i) => ({ ...e, i, a: fileIndex.get(e.from), b: fileIndex.get(e.to) }));
  const context = vm.createContext({ doc, mods, modIndex, files, fileIndex, edges, mx, my, lx, ly,
    modFiles: mods.map((_, m) => files.filter(f => f.mod === m).map(f => f.i)), N: mods.length,
    modR: mods.map(() => 30), modSignal: mods.map(() => 'cycle'), modHealth: mods.map(() => 'amber'),
    R_DOT: 2.6, SP: 10, fileRoom: () => 24, sunflower: () => [{x:10,y:10}],
    RING: { forbidden: 'red', cycle: 'amber', none: 'grey' }, hiddenMods: mods.map(() => false),
    ArchgraphPreview: engine, whatifMode: false, whatifResult: null, churnView: true,
    sel: null, sideHidden: false, sheetMode: false, panel: node(), navigator: {}, $, el,
    basename: p => p.slice(p.lastIndexOf('/') + 1), dirname: p => p.slice(0, Math.max(0, p.lastIndexOf('/'))),
    count: (n, a, b) => `${n} ${n === 1 ? a : b}`, fmt: n => n ?? 'n/a',
    runtime: e => doc.config.all_kinds || ['runtime', 'static'].includes(e.kind),
    fileEdges() {}, cancelSettle() {}, buildGrid() {}, applyFocus() {}, redraw() {},
    setSideHidden() {}, snapSheet() {}, showPanel() {}, curve() {} });
  vm.runInContext(code, context);
  return { context, controls, $, doc, files, lx, ly, mx, my,
    run: expression => vm.runInContext(expression, context) };
}
test('preview updates findings and commands; clear restores the immutable graph and file position', () => {
  const s = setup(), before = JSON.stringify(s.doc), i = s.files.findIndex(f => f.path === 'a/three.ts');
  const original = [s.files[i].x, s.files[i].y];
  s.run('setWhatifMode(true)');
  assert(s.$('reset').disabled);
  s.$('whatif-file').value = 'a/three.ts'; s.$('whatif-folder').value = 'b';
  s.run('runWhatif()');
  assert.match(s.$('whatif-result').textContent, /Improves the structure[\s\S]*largest module cycle 2 → 0/);
  assert.match(s.$('whatif-result').textContent, /Resolved[\s\S]*Module cycle · 2/);
  assert.equal(s.files[i].path, 'b/three.ts');
  assert.equal(s.$('whatif-cli').value, "archgraph simulate 'graph.json' --move 'a/three.ts' 'b/three.ts'");
  assert.equal(JSON.stringify(s.doc), before);
  s.run('clearWhatif()');
  assert.equal(s.files[i].path, 'a/three.ts');
  assert.deepEqual([s.files[i].x, s.files[i].y], original);
  assert.equal(s.$('stats').textContent, 'Original stats');
  assert(s.$('whatif-commands').hidden);
  s.run('setWhatifMode(false)'); assert.equal(s.context.churnView, true); assert(!s.$('reset').disabled);
});
test('invalid moves invalidate earlier results instead of retaining success or commands', () => {
  const s = setup();
  s.run('setWhatifMode(true)'); s.$('whatif-file').value = 'a/three.ts'; s.$('whatif-folder').value = 'b'; s.run('runWhatif()');
  s.$('whatif-folder').value = '../outside'; s.run('runWhatif()');
  assert.match(s.$('whatif-status').textContent, /Cannot preview/);
  assert.equal(s.context.whatifResult, null); assert.equal(s.$('whatif-cli').value, '');
  assert.equal(s.$('whatif-result').textContent, '');
});
test('a real folder drop uses source basename and can be undone by dropping back', () => {
  const s = setup(), i = s.files.findIndex(f => f.path === 'a/three.ts');
  s.run('setWhatifMode(true)');
  s.context.dropIndex = i; s.context.originalLocal = [s.lx[i], s.ly[i]];
  s.run('whatifDrop(dropIndex, 100, 0, originalLocal)');
  assert.equal(s.context.whatifResult.move.to, 'b/three.ts');
  s.run('whatifDrop(dropIndex, 0, 0, originalLocal)');
  assert.equal(s.context.whatifResult, null); assert.equal(s.files[i].path, 'a/three.ts');
});
test('partial previews display limitations and never claim no new cycle membership', () => {
  const name = fixtures.find(f => f.unsupported).name, s = setup(name);
  const move = fixtures.find(f => f.name === name).move;
  s.$('whatif-file').value = move.file; s.$('whatif-folder').value = path.posix.dirname(move.to);
  s.run('runWhatif()');
  assert(s.context.whatifResult.partial);
  assert.match(s.$('whatif-result').textContent, /Grab-bag refinement/);
  assert.doesNotMatch(s.$('whatif-status').textContent, /no new cycle membership/);
});
test('commands are shell-quoted data and unavailable clipboard falls back to selection', async () => {
  const s = setup();
  s.$('whatif-file').value = 'a/three.ts'; s.$('whatif-folder').value = "folder'quote;$x";
  s.$('whatif-graph').value = "my graph'$(whoami).json"; s.run('runWhatif()');
  assert.equal(s.$('whatif-cli').value, "archgraph simulate 'my graph'\\''$(whoami).json' --move 'a/three.ts' 'folder'\\''quote;$x/three.ts'");
  s.$('whatif-graph').value = '- odd graph.json'; s.run('whatifCommands()');
  assert.match(s.$('whatif-cli').value, /simulate '\.\/- odd graph.json'/);
  await s.run('whatifCopy(whatifUI.cli)');
  assert(s.$('whatif-cli').selected); assert.match(s.$('whatif-status').textContent, /Clipboard unavailable/);
  assert.equal(s.$('whatif-result').innerHTML, undefined);
});

test('live preview links are populated before the canvas takes its sorted draw batches', () => {
  const s = setup();
  s.$('whatif-file').value = 'a/three.ts'; s.$('whatif-folder').value = 'b'; s.run('runWhatif()');
  const html = fs.readFileSync(path.join(__dirname, '../assets/graph.html'), 'utf8');
  const start = html.indexOf('const buckets = new Map(), marks = [];');
  const end = html.indexOf('let clustersDone = false;', start);
  Object.assign(s.context, { fileT: 0, FE: new Float64Array(), seen: () => true, TONE: {neutral:'grey'}, C: {amber:'amber'} });
  const length = s.run(html.slice(start, end) + '\norder.length');
  assert(length > 0, 'preview imports must enter the canvas drawing batches');
});

test('dropping at an occupied module centre keeps the two file discs separate', () => {
  const s = setup();
  const resident = s.files.find(f => f.path === 'b/two.ts'); resident.x = 100; resident.y = 0;
  s.$('whatif-file').value = 'a/three.ts'; s.$('whatif-folder').value = 'b'; s.run('runWhatif()');
  const moved = s.files.find(f => f.path === 'b/three.ts');
  assert(Math.hypot(moved.x - resident.x, moved.y - resident.y) > 7);
});
