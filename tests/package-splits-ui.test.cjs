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
function context(extra = {}) {
  const ctx = vm.createContext({ document: { createElement: node, createTextNode: text => ({ textContent: text }) }, ...extra });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), ctx);
  return ctx;
}
function candidate(patch = {}) {
  return { path: 'src/shared', modules: ['shared'], files: ['src/shared/date.ts'], classification: 'free',
    readiness: 'import-level-ready', fan_in: 4, size: 1, score: 4, cohesion: 0.8, internal_edges: 2,
    cut_list: [], package_dependencies: [], external_packages: [], uncertainties: [],
    simulation: { cycles_before: 2, cycles_after: 1, boundary_cycle: [], scope: 'remaining graph' }, ...patch };
}
function open(detail) { detail.open = true; detail.listeners.toggle(); }
function descendants(root) { return [root, ...(root.children || []).flatMap(descendants)]; }
function setup(doc) {
  const panel = node(), legend = node(), targets = [];
  const ctx = context({ doc, panel, mods: [{ id: 'shared' }], $: () => legend,
    fileIndex: new Map([['src/shared/date.ts', 0]]), short: x => x, go: t => targets.push(t) });
  vm.runInContext(fragment('function btn(', 'const icon ='), ctx);
  vm.runInContext(fragment('/* ---------- package split model', '/* ---------- end package split model'), ctx);
  vm.runInContext(fragment('/* ---------- package split panel', '/* ---------- end package split panel'), ctx);
  return { ctx, panel, legend, targets };
}
test('old graphs have no package split marker, legend or panel', () => {
  const { ctx, panel, legend } = setup({ files: [] });
  vm.runInContext('packageSplitContext(); packageSplitContext("shared")', ctx);
  assert.equal(panel.children.length, 0);
  assert.equal(legend.hidden, true);
  assert.equal(vm.runInContext('packageSplitMarkers[0]', ctx), null);
  assert.doesNotThrow(() => new vm.Script(script));
});
test('marker states distinguish verified free candidates, cheap cuts and uncertainty', () => {
  const ctx = context();
  for (const [row, expected] of [
    [candidate(), 'ready'],
    [candidate({ classification: 'cheap', readiness: 'cuts-required' }), 'cheap'],
    [candidate({ readiness: 'needs-verification' }), 'uncertain'],
    [candidate({ simulation: { boundary_cycle: ['shared'] } }), 'uncertain'],
    [candidate({ simulation: undefined }), 'uncertain'],
  ]) {
    ctx.row = row;
    assert.equal(vm.runInContext('packageSplitMarker([row]).state', ctx), expected);
  }
  const { legend } = setup({ files: [], package_splits: { candidates: [candidate()] } });
  assert.equal(legend.hidden, false);
  assert.match(legend.textContent, /teal = import-level ready.*amber = cuts required.*grey = needs verification/);
  assert.match(legend.textContent, /only part of a module/);
});
test('candidate evidence uses safe text, lazy cut lists, actual file navigation and partial coverage', () => {
  const evil = '<img src=x onerror=alert(1)>', row = candidate({ path: evil, cut_list: [
    { from: 'src/shared/date.ts', to: evil, kind: 'runtime', line: 9, reason: evil }
  ], uncertainties: [evil], simulation: { cycles_before: 2, cycles_after: 1, boundary_cycle: [evil, 'other'], scope: evil } });
  const { ctx, panel, targets } = setup({ files: [{ path: 'src/shared/date.ts', module: 'shared' }, { path: 'src/shared/other.ts', module: 'shared' }],
    package_splits: { candidates: [row], workspace: { checked: true }, limitations: [evil] } });
  vm.runInContext('packageSplitContext("shared")', ctx);
  const detail = panel.children.at(-1).children[0];
  assert.equal(detail.children.length, 1);
  open(detail);
  assert.match(panel.textContent, /Partial module coverage: 1 of 2 files/);
  assert.match(panel.textContent, /Score = 4 inbound modules × 1 files = 4; cohesion 0.80; 2 internal imports/);
  assert.match(panel.textContent, /Simulation: cycles 2 → 1/);
  assert.ok(panel.textContent.includes('Boundary cycle: ' + evil));
  const inner = detail.children[1], cuts = inner.children.find(n => n.children[0]?.textContent === '1 import to cut');
  assert.equal(cuts.children.length, 1);
  open(cuts);
  assert.ok(cuts.textContent.includes(evil));
  assert.match(cuts.textContent, /date.ts:9 →/);
  descendants(cuts).find(n => n.listeners?.click).listeners.click();
  assert.equal(targets[0].type, 'file');
  assert.equal(targets[0].i, 0);
  const count = detail.children.length;
  open(detail); open(cuts);
  assert.equal(detail.children.length, count);
  assert.equal(cuts.children.length, 2);
});
test('overview explains workspace checks and empty candidate results', () => {
  for (const checked of [true, false]) {
    const { ctx, panel } = setup({ files: [], package_splits: { candidates: [], workspace: { checked, limitations: ['workspace warning'] }, limitations: ['static imports only'] } });
    vm.runInContext('packageSplitContext()', ctx);
    assert.ok(panel.textContent.includes(checked ? 'were checked' : 'not fully checked'));
    assert.match(panel.textContent, /No package split candidates/);
    assert.match(panel.textContent, /static imports only/);
    assert.match(panel.textContent, /workspace warning/);
  }
  assert.match(script, /packageSplitContext\(m.id\)/);
});
test('island halo preserves the existing surface and health rings and canvas state', () => {
  const calls = [], ctx = context({ canvas: Object.fromEntries(['save', 'restore', 'setLineDash', 'beginPath', 'arc', 'stroke'].map(name => [name, (...args) => calls.push([name, ...args])])) });
  vm.runInContext('drawPackageSplitHalo(canvas, { color: "teal" }, 1, 2, 10, 2)', ctx);
  assert.equal(calls[0][0], 'save');
  assert.equal(calls.at(-1)[0], 'restore');
  assert.deepEqual(calls.find(c => c[0] === 'arc'), ['arc', 1, 2, 26, 0, Math.PI * 2]);
  assert.match(script, /ctx.lineWidth = surfaceRingWidth\(surface\?\.exported.length \|\| 0, chosen \|\| s >= S_IN\) \* px/);
  assert.match(script, /ctx.strokeStyle = chosen \? C.focus/);
  assert.match(script, /drawPackageSplitHalo\(ctx, packageSplitMarkers\[i\], x, y, r, px\)/);
});
