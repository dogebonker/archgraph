'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const html = fs.readFileSync(path.join(__dirname, '../assets/graph.html'), 'utf8');
const script = html.match(/<script>\n([\s\S]*?)<\/script>/)[1];
function fragment(start, end) {
  const a = script.indexOf(start), b = script.indexOf(end, a);
  assert.ok(a >= 0 && b > a, `missing fragment: ${start}`);
  return script.slice(a, b);
}
function node() {
  return {
    children: [], listeners: {}, hidden: true, open: false,
    get childNodes() { return this.children; },
    setAttribute() {},
    addEventListener(event, fn) { this.listeners[event] = fn; },
    append(...children) { this.children.push(...children); },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(c => c.textContent).join(''); },
    set innerHTML(_) { throw new Error('repository text must not become markup'); },
  };
}
function open(detail) { detail.open = true; detail.listeners.toggle(); }
function descendants(root) { return [root, ...(root.children || []).flatMap(descendants)]; }
const evil = '<img src=x onerror=alert(1)>';
function target(patch = {}) {
  return {
    claims: [{ id: 'layered', reason: 'Dependencies point inward (slide 8)', source: null, verdict: 'holds_in', notes: [evil], slices: [
      { slice: 'auth', files: ['internal/auth/handler.go'], broken: [] },
      { slice: 'sop', files: ['internal/sop/handler.go', 'internal/sop/repository.go'],
        broken: [{ from: 'internal/sop/handler.go', to: 'internal/sop/repository.go' }, { from: 'internal/sop/handler.go', to: evil }] },
    ] }],
    moves: [
      { slice: 'auth', files: [], import_lines: 0, names: [] },
      { slice: 'sop', files: [{ file: 'internal/sop/handler.go', to: 'internal/sop/handler/handler.go' }], import_lines: 4,
        names: [{ name: 'sop.store', declared_in: 'internal/sop/repository.go', used_by: ['internal/sop/handler.go'] }],
        new_cycles: [{ from: 'internal/sop/handler.go', to: 'internal/sop/repository.go' }] },
    ],
    progress: { conforming: { done: 1, of: 2 }, moved: { done: 1, of: 2 } },
    notes: ['moves[1] matches no file'], ...patch,
  };
}
function setup(doc) {
  const panel = node(), targets = [], linked = [];
  const ctx = vm.createContext({ document: { createElement: node, createTextNode: text => ({ textContent: text }) },
    doc, panel, short: x => x, go: t => targets.push(t),
    dirname: p => p.slice(0, Math.max(0, p.lastIndexOf('/'))),
    mx: [0], my: [0], modR: [30], files: [{ mod: 0 }, { mod: 0 }], SP: 10, R_DOT: 2.6, GAP: 34,
    scanDirs: new Set(['internal', 'internal/sop']), destination: to => (to === 'internal/sop/h.go' ? [7, 8] : [9, 9]),
    fileIndex: new Map([['internal/sop/handler.go', 0], ['internal/sop/repository.go', 1]]),
    edgeOf: l => { linked.push(l); return l.to === evil ? undefined : 7; } });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), ctx);
  vm.runInContext(fragment('function btn(', 'const icon ='), ctx);
  vm.runInContext(fragment('/* ---------- target panel', '/* ---------- end target panel'), ctx);
  return { ctx, panel, targets, linked };
}
const click = root => descendants(root).find(n => n.listeners?.click && n.textContent === 'Show on map').listeners.click();
test('a claim shows its verdict, reason and notes, then progress', () => {
  const { ctx, panel } = setup({ target: target() });
  vm.runInContext('targetContext()', ctx);
  assert.match(panel.textContent, /layered: holds in 1 of 2 slices/);
  assert.match(panel.textContent, /Dependencies point inward \(slide 8\)/);
  assert.ok(panel.textContent.includes(evil));
  assert.match(panel.textContent, /1 of 2 slices conform; 1 of 2 slices moved/);
  assert.match(panel.textContent, /moves\[1\] matches no file/);
  for (const [verdict, text] of [['holds', 'holds'], ['broken', 'broken'], ['vacuous', 'vacuous']]) {
    ctx.claim = { verdict, slices: [] };
    assert.equal(vm.runInContext('claimVerdict(claim)', ctx), text);
  }
});
test('a broken slice lists its imports and selecting it highlights its files and imports', () => {
  const { ctx, panel, targets, linked } = setup({ target: target() });
  vm.runInContext('targetContext()', ctx);
  const slice = descendants(panel).find(n => n.listeners?.toggle && n.children?.[0]?.textContent === 'sop: 2 broken imports');
  assert.ok(slice, 'the broken slice is listed; the holding one is not');
  assert.ok(!descendants(panel).some(n => n.children?.[0]?.textContent?.startsWith('auth:')));
  assert.equal(slice.children.length, 1);
  open(slice);
  assert.match(slice.textContent, /handler.go → internal\/sop\/repository.go/);
  assert.ok(slice.textContent.includes(evil));
  click(slice);
  const t = targets.at(-1);
  assert.equal(t.type, 'target');
  assert.deepEqual([...t.fs], [0, 1]);
  assert.deepEqual([...t.fe], [7]);
  assert.equal(linked.length, 2);
});
test('a move row shows files, import lines, names and new cycles, and selects its files', () => {
  const { ctx, panel, targets } = setup({ target: target() });
  vm.runInContext('targetContext()', ctx);
  const row = descendants(panel).find(n => n.listeners?.toggle && n.children?.[0]?.textContent?.startsWith('sop · '));
  assert.equal(row.children[0].textContent, 'sop · 1 file, 4 import lines, 1 name to export, 1 import in new package cycles');
  assert.ok(!descendants(panel).some(n => n.children?.[0]?.textContent?.startsWith('auth · ')), 'a moved slice has no cost row');
  open(row);
  for (const list of descendants(row).filter(n => n.listeners?.toggle && n !== row)) open(list);
  assert.match(row.textContent, /internal\/sop\/handler.go → internal\/sop\/handler\/handler.go/);
  assert.match(row.textContent, /sop.store in internal\/sop\/repository.go, used by internal\/sop\/handler.go/);
  assert.match(row.textContent, /1 import in new package cycles/);
  click(row);
  const t = targets.at(-1);
  assert.equal(t.type, 'target');
  assert.deepEqual([...t.fs], [0]);
  assert.deepEqual([...t.fe], [7]);
});
test('selecting a move to a new folder places one empty disc beside the module its files leave', () => {
  const { ctx, panel, targets } = setup({ target: target() });
  vm.runInContext('targetContext()', ctx);
  const row = descendants(panel).find(n => n.listeners?.toggle && n.children?.[0]?.textContent?.startsWith('sop · '));
  open(row);
  click(row);
  const moves = targets.at(-1).moves;
  assert.equal(moves.length, 1);
  const [mv] = moves;
  assert.equal(mv.file, 0);
  assert.equal(mv.to, 'internal/sop/handler/handler.go');
  assert.equal(mv.note, 'new folder internal/sop/handler/');
  assert.equal(+mv.r.toFixed(6), 9.2);
  assert.deepEqual(Array.from(mv.at, v => +v.toFixed(6)), [56.2, 0]);
});
test('a move to an existing folder points at its files, one label per folder', () => {
  const { ctx } = setup({ target: target() });
  ctx.m = { files: [{ file: 'internal/sop/handler.go', to: 'internal/sop/h.go' }, { file: 'internal/sop/repository.go', to: 'internal/sop/r.go' }] };
  const moves = vm.runInContext('targetMoves(m)', ctx);
  assert.deepEqual(Array.from(moves, mv => [mv.file, mv.to, [...mv.at], 'r' in mv, mv.note]),
    [[0, 'internal/sop/h.go', [7, 8], false, 'moves to internal/sop/'], [1, 'internal/sop/r.go', [9, 9], false, null]]);
});
test('a what-if target compares progress with the scan and selects nothing', () => {
  const { ctx, panel } = setup({ target: target() });
  ctx.box = node(); ctx.t = target();
  vm.runInContext('targetContext(t, box, { conforming: { done: 2, of: 2 }, moved: { done: 1, of: 2 } })', ctx);
  assert.equal(panel.children.length, 0);
  assert.ok(ctx.box.textContent.includes('1 of 2 slices conform (was 2 of 2); 1 of 2 slices moved.'));
  assert.ok(ctx.box.textContent.includes('Move cost, from the scan (not previewed)'));
  const rows = descendants(ctx.box).filter(n => n.listeners?.toggle);
  assert.equal(rows.length, 2);
  for (const row of rows) open(row);
  assert.ok(!descendants(ctx.box).some(n => n.textContent === 'Show on map'));
  // A count the preview cannot recompute is the scan's, marked as such.
  ctx.box = node(); ctx.t = target({ progress: { conforming: { done: 1, of: 2, preview: false }, moved: { done: 1, of: 2 } } });
  vm.runInContext('targetContext(t, box, { conforming: { done: 1, of: 2 }, moved: { done: 1, of: 2 } })', ctx);
  assert.ok(ctx.box.textContent.includes('1 of 2 slices conform (not previewed); 1 of 2 slices moved.'));
});
test('a document without a target renders nothing', () => {
  const { ctx, panel } = setup({ files: [] });
  vm.runInContext('targetContext()', ctx);
  assert.equal(panel.children.length, 0);
});
test('the panel is wired into the overview, selection, zoom and panel routing without markup', () => {
  assert.ok(!fragment('/* ---------- target panel', '/* ---------- end target panel').includes('innerHTML'));
  assert.match(fragment('function overview()', 'function packageSplitContext'), /targetContext\(\);\n\s*packageSplitContext\(\);/);
  assert.match(script, /t\.type === "target"\) targetPanel\(t\)/);
  assert.match(script, /t\.type === "target"\) zoomToFinding\(/);
  assert.match(fragment('function applyFocus()', '/* ---------- canvas, view'), /t\.type === "target"/);
  assert.match(fragment('function applyFocus()', '/* ---------- canvas, view'), /overlay = \{ moves: t\.moves \|\| \[\], links: \[\] \}/);
  assert.match(script, /t\.type === "target"\) zoomToFinding\(\{ fs: t\.fs, ms: \[\], moves: t\.moves \|\| \[\] \}\)/);
  assert.doesNotThrow(() => new vm.Script(script));
});
test('rules holding only claims leave undecided imports unflagged', () => {
  const { ctx } = setup({ target: target() });
  const cases = [
    [undefined, false],
    [{ forbid: [{ id: 'layered', from: 'a/**', to: 'b/**' }], moves: [{ from: 'a/**', to: 'c/' }] }, true],
    [{ forbid: [{ from: 'a/**', to: 'b/**' }], layers: ['a/**'] }, false],
    [{ forbid: [{ from: 'a/**', to: 'b/**' }], allow: [{ from: 'a/**', to: '**' }] }, false],
    [{ forbid: [{ from: 'a/**', to: 'b/**' }], public: [{ module: 'b/**', entries: ['b/index.ts'] }] }, false],
    [{ forbid: [{ from: 'a/**', to: 'b/**' }], entrypoints: { 'b/**': 'b/index.ts' } }, false],
    [{ forbid: [{ from: 'a/**', to: 'b/**' }], default: 'forbidden' }, false],
    [{ forbid: [] }, false],
  ];
  for (const [rules, expected] of cases) {
    ctx.rules = rules;
    assert.equal(vm.runInContext('claimsOnly(rules)', ctx), expected, JSON.stringify(rules));
  }
});
