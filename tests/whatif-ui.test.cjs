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
    modRoot: mods.map(m => m.id + '/'), N: mods.length,
    modR: mods.map(() => 30), modSignal: mods.map(() => 'cycle'), modHealth: mods.map(() => 'amber'),
    R_DOT: 2.6, SP: 10, fileRoom: () => 24, sunflower: () => [{x:10,y:10}],
    RING: { forbidden: 'red', cycle: 'amber', none: 'grey' }, hiddenMods: mods.map(() => false),
    ArchgraphPreview: engine, unflagged: false, whatifMode: false, whatifResult: null, churnView: true,
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
  assert.equal(s.$('whatif-rules').value, 'No layout rules are embedded in this scan.');
  assert.equal(JSON.stringify(s.doc), before);
  s.run('clearWhatif()');
  assert.equal(s.files[i].path, 'a/three.ts');
  assert.deepEqual([s.files[i].x, s.files[i].y], original);
  assert.equal(s.$('stats').textContent, 'Original stats');
  assert(s.$('whatif-commands').hidden);
  s.run('setWhatifMode(false)'); assert.equal(s.context.churnView, true); assert(!s.$('reset').disabled);
});
test('the move list previews chained moves together and exports one script, command and rule shifts', () => {
  const fixture = fixtures.find(f => f.name === 'chained-moves-with-rules'), s = setup(fixture.name), before = JSON.stringify(s.doc);
  const plain = value => JSON.parse(JSON.stringify(value)), listed = () => plain(s.run('whatifMoves').map(({ file, to }) => ({ file, to })));
  const preview = (file, folder) => { s.$('whatif-file').value = file; s.$('whatif-folder').value = folder; s.run('runWhatif()'); };
  s.run('setWhatifMode(true)');
  preview('app/helper.ts', 'infra'); s.run('whatifAdd()');
  // Add disables itself: focus moves to the added item's Remove button.
  assert(s.run('whatifRemove[0].focused'));
  // The listed file, named by its previewed path, moves again: its entry takes the new destination.
  preview('infra/helper.ts', 'domain');
  assert.equal(s.run('whatifMoves[0].to'), 'infra/helper.ts', 'previewing alone does not change the list');
  s.run('whatifAdd()');
  preview('infra/db.ts', 'domain/store'); s.run('whatifAdd()');
  assert.deepEqual(listed(), fixture.move);
  assert.deepEqual(plain(s.context.whatifResult.before), fixture.before);
  assert.deepEqual(plain(s.context.whatifResult.after), fixture.after);
  assert.equal(s.$('whatif-cli').value, "archgraph simulate 'graph.json' --move 'app/helper.ts' 'domain/helper.ts' --move 'infra/db.ts' 'domain/store/db.ts'");
  assert.equal(s.$('whatif-git').value, "set -e\nmkdir -p -- 'domain/store'\ngit mv -- 'app/helper.ts' 'domain/helper.ts'\ngit mv -- 'infra/db.ts' 'domain/store/db.ts'");
  assert.equal(s.$('whatif-rules').value,
    'app/helper.ts: leaves layer "app/**", enters layer "domain/**", enters forbid no-infra from "domain/**"\n' +
    'infra/db.ts: leaves forbid no-infra to "infra/**", enters layer "domain/**", enters forbid no-infra from "domain/**"');
  assert.deepEqual(s.files.filter(f => f.path.startsWith('domain/')).map(f => f.path).sort(), ['domain/helper.ts', 'domain/model.ts', 'domain/store/db.ts']);
  assert.equal(JSON.stringify(s.doc), before);
  // Remove one item: the preview and export keep the rest, and focus stays in the list.
  const list = s.$('whatif-list');
  assert.equal(list.childNodes.length, 2);
  list.childNodes[0].childNodes[1].listeners.click();
  assert.deepEqual(listed(), [fixture.move[1]]);
  assert.equal(s.$('whatif-cli').value, "archgraph simulate 'graph.json' --move 'infra/db.ts' 'domain/store/db.ts'");
  assert.equal(s.files.find(f => f.path.endsWith('helper.ts')).path, 'app/helper.ts');
  assert(list.childNodes[0].childNodes[1].focused);
  // Clear all restores the original graph and empties the list.
  s.$('whatif-clear').listeners.click();
  assert.deepEqual(listed(), []); assert.equal(s.context.whatifResult, null);
  assert(s.$('whatif-plan').hidden); assert(s.$('whatif-commands').hidden);
  assert.equal(s.files.find(f => f.path.endsWith('db.ts')).path, 'infra/db.ts');
  // Only the first matching layer counts, and a capture glob shifts when its bound value changes.
  s.doc.config.rules.doc = { layers: ['src/a/**', 'src/**'], forbid: [{ id: 'own', from: 'src/<d>/**', to: 'lib/**' }] };
  assert.equal(s.run('whatifRuleShifts([{ file: "src/a/x.ts", to: "src/b/x.ts" }])'),
    'src/a/x.ts: leaves layer "src/a/**", leaves forbid own from "src/<d>/**" (<d> = a), ' +
    'enters layer "src/**", enters forbid own from "src/<d>/**" (<d> = b)');
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

test('a claims-only layout leaves an import no rule decides neutral in the preview', () => {
  const s = setup('reachable-forbid'), move = fixtures.find(f => f.name === 'reachable-forbid').move;
  s.context.unflagged = true;
  s.$('whatif-file').value = move.file; s.$('whatif-folder').value = path.posix.dirname(move.to); s.run('runWhatif()');
  assert.equal(s.context.whatifResult.after.unplanned, 1);
  assert.deepEqual(Array.from(s.run('whatifDrawing.files'), f => f.tone), ['neutral', 'neutral']);
});
