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
  assert.ok(a >= 0 && b > a, `missing fragment ${start}`);
  return script.slice(a, b);
}
function element() {
  return { children: [], listeners: {}, hidden: true, open: false,
    get childNodes() { return this.children; },
    setAttribute() {}, addEventListener(name, callback) { this.listeners[name] = callback; },
    append(...children) { this.children.push(...children); },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(child => child.textContent).join(''); },
    set innerHTML(_) { throw Error('unsafe markup'); } };
}
function setup(doc = {}) {
  const panel = element(), legend = element(), calls = [];
  const context = vm.createContext({ doc: { config: {}, ...doc }, panel, document: {
    createElement: element, createTextNode: text => ({ textContent: text }) }, $: () => legend,
    mods: [{ id: 'domain' }], modIndex: new Map([['domain', 0]]), mx: [0], my: [0], modR: [40],
    files: [{ path: 'domain/a.ts', mod: 0, i: 0 }], fileIndex: new Map([['domain/a.ts', 0]]),
    whatifResult: null, hover: null, kb: null, sel: null, hiddenMods: [false],
    C: { surface: 'surface', muted: 'muted', forbidden: 'red', focus: 'focus', fg: 'fg' }, MONO: 'monospace',
    head: (...args) => calls.push(args), go: target => calls.push(target),
    ctx: new Proxy({}, { get: (target, key) => target[key] || ((...args) => calls.push([key, ...args])) }) });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), context);
  vm.runInContext(fragment('function btn(', 'const icon ='), context);
  vm.runInContext(fragment('/* ---------- external model', '/* ---------- end external model'), context);
  vm.runInContext(fragment('/* ---------- external panel and drawing', '/* ---------- end external panel and drawing'), context);
  return { context, panel, legend, calls, run: text => vm.runInContext(text, context) };
}
const packageNode = { id: 'external:typescript:typeorm', external: true, package: 'typeorm' };
const packageEdge = { from: 'domain/a.ts', to: packageNode.id, kind: 'type', weight: .1, line: 7,
  specifier: 'typeorm', verdict: 'forbidden', rule: 'domain-no-orm', confidence: 'low' };
test('legacy graph invents neither exposure counts nor packages', () => {
  const { run, panel, legend } = setup();
  run('externalContext(); externalContext("domain"); drawExternals(1)');
  assert.equal(panel.children.length, 0); assert.equal(legend.hidden, true);
  assert.equal(run('externalPositions().length'), 0);
  assert.doesNotThrow(() => new vm.Script(script));
});
test('ring and hit testing stay outside repository extent, stable order and preview hides stale packages', () => {
  const { run, context, legend } = setup({ external_nodes: [packageNode, { ...packageNode, id: 'a', package: 'axios' }], external_edges: [packageEdge] });
  assert.equal(legend.hidden, false);
  assert.equal(run('externalPositions()[0].package'), 'axios');
  assert.equal(run('externalPositions().every(p => Math.hypot(p.x, p.y) > 100)'), true);
  assert.equal(run('externalHit(externalPositions(), externalPositions()[0].x, externalPositions()[0].y, 1).i'), 0);
  assert.equal(run('externalHit(externalPositions(), 0, 0, 1)'), undefined);
  const before = run('JSON.stringify(externalPositions())');
  assert.equal(run('JSON.stringify(externalPositions())'), before);
  context.whatifResult = {};
  assert.equal(run('externalPositions().length'), 0);
  context.whatifResult = null;
  assert.equal(run('JSON.stringify(externalPositions())'), before);
  assert.equal(run('files.length'), 1); assert.equal(run('mods.length'), 1);
});
test('module exposure retains weighted factors and advisory; missing rows stay unknown', () => {
  const { run, panel } = setup({ external_nodes: [packageNode], external_edges: [packageEdge], external_exposure: {
    threshold_percent: 70, modules: [{ module: 'domain', imports: 5, external_imports: 4, external_packages: 1,
      external_weight: .4, total_weight: 1.4, external_percent: 80, weighted_external_percent: 28.5714, unresolved_imports: 2, exceeds_threshold: true }] } });
  run('externalContext("domain")');
  assert.match(panel.textContent, /4 of 5.*80%.*weight 0.40 of 1.40/);
  assert.match(panel.textContent, /2 unresolved/);
  assert.match(panel.textContent, /intentional adapter or wrapper/);
  const count = panel.children.length; run('externalContext("missing")'); assert.equal(panel.children.length, count);
});
test('package inspector renders source evidence, kinds, weights and verdicts as text; package list supports keyboard buttons', () => {
  const evil = '<img src=x onerror=alert(1)>';
  const { run, panel, calls } = setup({ config: { rules: { doc: { 'pkg-category': { orm: [evil] } } } },
    external_nodes: [{ ...packageNode, package: evil }], external_edges: [{ ...packageEdge, rule: evil, specifier: evil }] });
  run('externalPanel(0)');
  assert.match(panel.textContent, /domain\/a.ts:7 — type; weight 0.10; forbidden/);
  assert.match(panel.textContent, /confidence low/); assert.match(panel.textContent, /Exact category memberships: orm/);
  assert.ok(panel.textContent.includes(evil));
  run('externalContext()');
  const detail = panel.children.at(-1); detail.open = true; detail.listeners.toggle();
  const button = detail.children[1].children[0]; button.listeners.click();
  assert.equal(calls.at(-1).type, 'external');
});
test('external dots are visible without drawing every package link; selected package draws recorded links', () => {
  const { run, context, calls } = setup({ external_nodes: [packageNode], external_edges: [packageEdge] });
  run('drawExternals(1)'); assert.ok(calls.some(call => call[0] === 'arc'));
  assert.equal(calls.filter(call => call[0] === 'lineTo').length, 0);
  const saved = [];
  context.ctx = new Proxy({ textAlign: 'start', save() { saved.push(this.textAlign); }, restore() { this.textAlign = saved.pop(); } },
    { get: (target, key) => target[key] || ((...args) => calls.push([key, ...args])) });
  context.sel = { type: 'external', i: 0 }; run('drawExternals(1)');
  assert.equal(calls.filter(call => call[0] === 'lineTo').length, 1);
  assert.ok(calls.some(call => call[0] === 'fillText' && call[1] === 'pkg:typeorm'));
  assert.equal(context.ctx.textAlign, 'start', 'the centred package label must not leak into later labels');
});
test('ring integrates camera bounds, hit testing, search, selection and module context', () => {
  for (const [start, end, expected] of [
    ['function extent()', '/* Past the extent', /externalPositions/],
    ['function bounds(k)', 'function fitK()', /externalPositions/],
    ['function hit(sx', '/* ---------- tooltip', /externalHit/],
    ['function find(q)', '$("find").addEventListener', /externalNodes/],
    ['function showPanel()', '/*__ARCHGRAPH_PREVIEW_UI__*/', /externalPanel/],
    ['function modulePanel(i)', 'function filePanel', /externalContext\(m.id\)/],
  ]) assert.match(fragment(start, end), expected);
});

test('external rule verdicts contribute separate assessment counts and a real package/file next step', () => {
  const { context, run } = setup({ external_nodes: [packageNode], external_edges: [packageEdge,
    { ...packageEdge, kind: 'runtime', line: 8 }, { ...packageEdge, verdict: 'unplanned', line: 9 }] });
  vm.runInContext('const bad = [], coverage = [], next = [];', context);
  const start = script.indexOf('  for (const verdict of ["forbidden", "unplanned"]) {');
  const end = script.indexOf('    const groups = ruleGroups(verdict);', start);
  vm.runInContext(script.slice(start, end) + '\n}', context);
  assert.match(run('bad.map(row => row.parts.filter(p => typeof p === "string").join("")).join(" ")'), /2 recorded external imports are forbidden/);
  assert.match(run('coverage.map(row => row.parts.filter(p => typeof p === "string").join("")).join(" ")'), /1 recorded external import is unplanned/);
  assert.equal(run('bad.length'), 1);
  assert.equal(run('bad[0].touch'), 1);
  assert.equal(run('next.length'), 1); assert.equal(run('next[0][1].f'), 0);
  vm.runInContext(fragment('function refTarget(p)', 'function sentence('), context);
  assert.equal(run('refTarget(next[0][3]).type'), 'external');
});
test('forbidden external imports mark only their source module red and empty assessment text uses a full row', () => {
  const { context, run } = setup({ external_nodes: [packageNode], external_edges: [packageEdge] });
  Object.assign(context, { N: 1, edges: [], findings: [], modCycle: [-1], findingsOfModule: [[]] });
  context.doc.cycles = [];
  vm.runInContext(fragment('const forbiddenMod =', '/* ---------- assessment:'), context);
  assert.equal(run('modSignal[0]'), 'forbidden'); assert.equal(run('modHealth[0]'), 'red');
  assert.match(html, /\.assess li\.empty\s*\{[^}]*display:\s*block/);
});

test('Violations filter excludes uncovered imports at both file and module levels', () => {
  const context = vm.createContext({ hiddenMods: [false, false], files: [{ mod: 0 }, { mod: 1 }] });
  vm.runInContext(fragment('const filt =', 'const same ='), context);
  vm.runInContext('filt.problems = true', context);
  for (const tone of ['neutral', 'unplanned', 'forbidden']) {
    context.edge = { a: 0, b: 1, tone, kind: 'runtime', v: { forbidden: tone === 'forbidden' ? 1 : 0 } };
    assert.equal(vm.runInContext('showFE(edge)', context), tone === 'forbidden');
    assert.equal(vm.runInContext('showME(edge)', context), tone === 'forbidden');
  }
  vm.runInContext('filt.problems = false', context);
  context.edge.tone = 'unplanned';
  assert.equal(vm.runInContext('showFE(edge)', context), true);
});

test('folder coupling alone is advisory rather than red repair advice', () => {
  const { context, run } = setup();
  Object.assign(context, { N: 1, edges: [], findings: [], modCycle: [0], findingsOfModule: [[]] });
  context.doc.cycles = [{ level: 'module', break: [{ from: 'domain/a.ts' }], members: ['domain'] }];
  vm.runInContext(fragment('const forbiddenMod =', '/* ---------- assessment:'), context);
  assert.equal(run('modHealth[0]'), 'amber');
  assert.doesNotMatch(fragment('const KIND_ADVICE =', 'function advice('), /cannot be changed|Remove or invert/);
  assert.doesNotMatch(fragment('  modCycles.forEach(c => {', '  const moves = ofKind('), /remove or invert|by removing|drop the re-export|"Move "/);
});

test('diagnostics remain informational, render reason as text and tolerate older documents', () => {
  const { context, run, panel } = setup();
  vm.runInContext(fragment('/* ---------- diagnostic observations', '/* ---------- end diagnostic observations'), context);
  run('diagnosticContext()');
  assert.equal(panel.children.length, 0);
  context.doc.diagnostics = [{ finding: { kind: 'cycle', subject: 'app ↔ api', evidence: { members: ['app', 'api'] } }, reason: '<img src=x onerror=alert(1)>' }];
  run('diagnosticContext()');
  assert.match(panel.textContent, /Observations without a fix/);
  assert.match(panel.textContent, /app ↔ api/);
  assert.match(panel.textContent, /<img src=x onerror=alert\(1\)>/);
  assert.doesNotMatch(panel.textContent, /remove imports|Violation/);
});

test('folder cycles remain inspectable without an actionable finding', () => {
  const context = vm.createContext({ observations: [], modCycles: [{ members: ['app', 'api'] }],
    modIndex: new Map([['app', 0], ['api', 1]]), cycleFinding: () => -1,
    names: list => list.map(i => ['app', 'api'][i]) });
  const start = script.indexOf('  modCycles.forEach(c => {');
  const end = script.indexOf('  if (fileCycles.length)', start);
  vm.runInContext(script.slice(start, end), context);
  const text = vm.runInContext('observations[0].join("")', context);
  assert.match(text, /reciprocal folder dependencies/);
  assert.match(text, /without a circular file import/);
  assert.match(text, /No policy violation is implied/);
});

test('uncovered internal imports go to coverage rather than findings to repair', () => {
  const context = vm.createContext({ bad: [], coverage: [], next: [], externalEdges: [],
    edges: [{ a: 0, b: 1 }], ruleGroups: verdict => verdict === 'unplanned' ? [['unnamed', [0]]] : [],
    count: (n, one, many) => `${n} ${n === 1 ? one : many}` });
  const start = script.indexOf('  for (const verdict of ["forbidden", "unplanned"]) {');
  const end = script.indexOf('  const unstable =', start);
  vm.runInContext(script.slice(start, end), context);
  assert.equal(vm.runInContext('bad.length', context), 0);
  assert.equal(vm.runInContext('next.length', context), 0);
  assert.equal(vm.runInContext('coverage.length', context), 1);
  assert.match(vm.runInContext('coverage[0].parts[1]', context), /not a violation/);
});

test('file-cycle counts state their recorded scope and point to the full file graph', () => {
  const header = fragment('  stat(doc.cycles.filter(c => c.level === "file")', '  stat(forbidden,');
  assert.match(header, /within-module file cycle/);
  assert.doesNotMatch(header, /"file import cycle"/);
  const observations = fragment('  modCycles.forEach(c => {', '  if (fileCycles.length)');
  assert.match(observations, /Cross-module circular file imports may be summarized as folder loops/);
  assert.match(observations, /--granularity file/);
});
