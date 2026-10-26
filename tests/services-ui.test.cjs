'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const html = fs.readFileSync(path.join(__dirname, '../assets/services.html'), 'utf8');
const script = html.match(/<script>\n([\s\S]*?)<\/script>/)[1];
const descendants = node => [node, ...node.children.flatMap(descendants)];
function element(tag) {
  return {
    tag, children: [], attrs: {}, style: {}, listeners: {}, value: '', hidden: false, disabled: false, namespaceURI: tag === 'svg' ? 'http://www.w3.org/2000/svg' : undefined,
    setAttribute(name, value) { this.attrs[name] = String(value); },
    getAttribute(name) { return this.attrs[name] ?? null; },
    addEventListener(name, callback) { this.listeners[name] = callback; },
    append(...children) { this.children.push(...children); },
    replaceChildren(...children) { this.text = ''; this.children = children; },
    focus() { this.focused = true; },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(child => child.textContent).join(''); },
    set innerHTML(_) { throw new Error('snapshot text must not become markup'); },
  };
}
function fixture() {
  return {
    schema: 'boundrail/v1.2', kind: 'services', source: { collector: 'inferred', path: 'example', complete: false },
    services: ['api', 'worker'],
    modules: [{ id: 'api', fan_in: 1, fan_out: 0, instability: 0 }, { id: 'worker', fan_in: 0, fan_out: 1, instability: 1 }],
    edges: [{ from: 'worker', to: 'api', kind: 'http', confidence: 'medium', operation: 'GET /users/:id', source: { file: 'worker/main.ts', line: 7 }, verdict: 'forbidden', rule: 'no-api' }],
    resources: [
      { service: 'api', kind: 'database', package: 'pg', confidence: 'high', source: { file: 'api/db.ts', line: 4 }, verdict: 'allowed' },
      { service: 'worker', kind: 'database', package: 'pg', confidence: 'medium', source: { file: 'shared/db.ts', line: 9 }, verdict: 'allowed' },
    ],
    forbid: [{ id: 'no-api', from: 'worker', to: 'api', reason: 'Use events.' }],
    summary: { services: 2, calls: 1, resources: 2, forbidden_calls: 1 },
    caveats: ['Source-based and incomplete.', 'Runtime configuration can hide calls.'],
  };
}
function setup(report = fixture(), clipboard = async () => {}, stacked = false) {
  const nodes = new Map();
  for (const [, tag, id] of html.matchAll(/<(\w+)[^>]*\bid="([^"]+)"/g)) {
    const node = element(tag); node.setAttribute('id', id); nodes.set(id, node);
  }
  nodes.get('services').textContent = JSON.stringify(report);
  const all = () => [...nodes.values()].flatMap(descendants);
  const copied = [];
  const document = {
    getElementById: id => all().find(node => node.attrs.id === id) || null,
    createElement: element,
    createElementNS: (namespace, tag) => { assert.equal(namespace, 'http://www.w3.org/2000/svg'); return element(tag); },
    querySelectorAll: query => { assert.equal(query, '[data-focus]'); return all().filter(node => node.attrs['data-focus']); },
  };
  const ctx = vm.createContext({ document, matchMedia: () => ({ matches: stacked }), navigator: { clipboard: { writeText: async text => { copied.push(text); await clipboard(text); } } } });
  vm.runInContext(script, ctx);
  const find = (root, predicate) => descendants(document.getElementById(root)).find(predicate);
  const byText = (root, text) => find(root, node => node.tag === 'button' && node.textContent.includes(text));
  return { nodes, document, ctx, copied, find, byText, text: id => document.getElementById(id).textContent };
}
test('service page shows truthful totals, mode, coverage and directed evidence', () => {
  const ui = setup();
  assert.match(ui.text('report-mode'), /Inferred model.*example/);
  assert.match(ui.text('totals'), /Services2Calls1Client usages2Forbidden1/);
  assert.equal(ui.text('catalog-count'), '2 of 2');
  assert.equal(ui.text('flow-count'), '1 of 1 inferred calls');
  assert.match(ui.text('call-list'), /worker → api.*GET \/users\/:id.*medium confidence.*forbidden/);
  assert.match(ui.text('caveat-list'), /Source-based and incomplete/);
  assert.equal(ui.nodes.get('flow-graph').children.filter(node => node.attrs.class?.includes('graph-node')).length, 2);
  const edge = ui.find('flow-graph', node => node.attrs.class === 'graph-hit');
  assert.match(edge.attrs['aria-label'], /from worker to api/);
});
test('search finds owned clients and operations, reports empty state, and clear restores catalog', () => {
  const ui = setup();
  const search = ui.nodes.get('search');
  search.value = 'GET /users'; search.listeners.input();
  assert.equal(ui.text('catalog-count'), '2 of 2');
  search.value = 'not-found'; search.listeners.input();
  assert.equal(ui.text('catalog-count'), '0 of 2');
  assert.match(ui.text('catalog-list'), /No matching services/);
  assert.equal(ui.nodes.get('graph-wrap').hidden, true);
  ui.nodes.get('clear').listeners.click();
  assert.equal(search.value, '');
  assert.equal(ui.text('catalog-count'), '2 of 2');
  assert.equal(search.focused, true);
});
test('service selection retains separate client usages even when packages match', () => {
  const ui = setup();
  ui.byText('catalog-list', 'api').listeners.click();
  assert.equal(ui.text('inspector-title'), 'api');
  assert.match(ui.text('inspector-body'), /Client usages1.*database · pg/);
  const apiResource = ui.find('inspector-body', node => node.attrs['data-resource-id']);
  assert.equal(apiResource.attrs['data-resource-id'], 'api:0');
  apiResource.listeners.click();
  assert.equal(ui.text('inspector-title'), 'api: pg');
  assert.match(ui.text('inspector-body'), /api\/db.ts:4/);
  ui.byText('catalog-list', 'worker').listeners.click();
  const workerResource = ui.find('inspector-body', node => node.attrs['data-resource-id']);
  assert.equal(workerResource.attrs['data-resource-id'], 'worker:1');
  workerResource.listeners.click();
  assert.match(ui.text('inspector-body'), /shared\/db.ts:9/);
  assert.match(ui.text('inspector-body'), /Confidencemedium/);
});
test('call and source selection expose rule evidence and copy only file:line', async () => {
  const ui = setup();
  ui.byText('call-list', 'worker').listeners.click();
  assert.equal(ui.text('inspector-title'), 'worker → api');
  assert.match(ui.text('inspector-body'), /Ruleno-apiReasonUse events/);
  ui.byText('inspector-body', 'worker/main.ts:7').listeners.click();
  assert.match(ui.text('source-evidence'), /Source locationworker\/main.ts:7/);
  await ui.document.getElementById('source-copy').listeners.click();
  assert.deepEqual(ui.copied, ['worker/main.ts:7']);
  assert.equal(ui.text('announcement'), 'Source location copied.');
});
test('clipboard rejection leaves source text and an actionable status', async () => {
  const ui = setup(fixture(), async () => { throw new Error('denied'); });
  ui.byText('call-list', 'worker').listeners.click();
  ui.byText('inspector-body', 'worker/main.ts:7').listeners.click();
  await ui.document.getElementById('source-copy').listeners.click();
  assert.match(ui.text('announcement'), /Select and copy/);
  assert.match(ui.text('source-evidence'), /worker\/main.ts:7/);
});
test('graph nodes and arrows respond to Enter and Space and retain keyboard focus', () => {
  const ui = setup();
  const graphService = ui.find('flow-graph', node => node.attrs['aria-label'] === 'Inspect service api');
  let prevented = false;
  graphService.listeners.keydown({ key: 'Enter', preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  assert.equal(ui.text('inspector-title'), 'api');
  assert.equal(ui.find('flow-graph', node => node.attrs['data-focus'] === 'graph-service:api').focused, true);
  const graphCall = ui.find('flow-graph', node => node.attrs.class === 'graph-hit');
  graphCall.listeners.keydown({ key: ' ', preventDefault() {} });
  assert.equal(ui.text('inspector-title'), 'worker → api');
  assert.equal(ui.find('flow-graph', node => node.attrs['data-focus'] === 'graph-call:0').focused, true);
});
test('empty inferred and declared graphs state what is absent without claiming independence', () => {
  const report = fixture(); report.edges = []; report.resources = []; report.summary = { services: 2, calls: 0 };
  const inferred = setup(report);
  assert.match(inferred.text('graph-empty'), /No calls inferred/);
  assert.match(inferred.text('graph-empty'), /may hide real connections/);
  inferred.byText('catalog-list', 'api').listeners.click();
  assert.match(inferred.text('inspector-body'), /No infrastructure client usage recorded/);
  report.source.collector = 'declarative';
  assert.match(setup(report).text('graph-empty'), /No calls declared/);
  report.services = [];
  const empty = setup(report);
  assert.match(empty.text('catalog-list'), /No services in this report/);
  assert.match(empty.text('graph-empty'), /no service boundaries/);
});
test('mixed provenance does not invent per-call inferred versus declared classification', () => {
  const report = fixture(); report.source.collector = 'inferred+declarative';
  const ui = setup(report); ui.byText('call-list', 'worker').listeners.click();
  assert.equal(ui.text('inspector-kind'), 'Call evidence');
  assert.match(ui.text('report-mode'), /Inferred and declared model/);
  assert.match(ui.text('flow-count'), /reported calls/);
});
test('snapshot strings stay text and prototype-like service IDs remain selectable', () => {
  const evil = '</script><img src=x onerror=alert(1)>';
  const report = fixture(); report.services[0] = '__proto__'; report.modules[0].id = '__proto__';
  report.edges[0].to = '__proto__'; report.edges[0].operation = evil;
  report.resources[0].service = '__proto__'; report.resources[0].package = evil; report.resources[0].source.file = evil + '.ts';
  report.caveats.push(evil);
  const ui = setup(report);
  ui.byText('catalog-list', '__proto__').listeners.click();
  assert.equal(ui.text('inspector-title'), '__proto__');
  assert.match(ui.text('inspector-body'), /<img/);
  assert.ok(ui.text('caveat-list').includes(evil));
  assert.ok(ui.text('call-list').includes(evil));
  assert.equal(ui.find('inspector-body', node => node.tag === 'img'), undefined);
});
test('template is offline and uses data-only DOM operations', () => {
  assert.equal((html.match(/\{\{SERVICES\}\}/g) || []).length, 1);
  assert.ok(html.includes("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'"));
  for (const banned of ['fetch(', 'innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'eval(', 'new Function', 'Math.random', 'Date.now']) assert.ok(!script.includes(banned), banned);
  assert.ok(html.includes('prefers-reduced-motion'));
  assert.ok(html.includes('aria-live="polite"'));
  assert.ok(html.includes('@media (max-width: 680px)'));
});

test('parallel and reverse calls retain distinct selectable diagram paths', () => {
  const report = fixture();
  report.edges.push({ ...report.edges[0], operation: 'POST /users' }, { ...report.edges[0], from: 'api', to: 'worker', operation: 'GET /jobs' });
  const ui = setup(report);
  const paths = descendants(ui.nodes.get('flow-graph')).filter(node => node.attrs.class === 'graph-hit');
  assert.equal(paths.length, 3);
  assert.equal(new Set(paths.map(node => node.attrs.d)).size, 3);
  paths[1].listeners.click();
  assert.match(ui.text('inspector-body'), /POST \/users/);
});

test('stacked layouts move selection focus to evidence and let readers return to catalog', () => {
  const ui = setup(fixture(), async () => {}, true);
  ui.byText('catalog-list', 'worker').listeners.click();
  assert.equal(ui.nodes.get('inspector-title').focused, true);
  ui.nodes.get('back-to-services').listeners.click();
  assert.equal(ui.nodes.get('search').focused, true);
});

test('diagram retains its geometry width on narrow screens instead of shrinking its labels', () => {
  for (const serviceCount of [2, 4]) {
    const report = fixture();
    if (serviceCount === 4) report.services.push('billing', 'notifications');
    const ui = setup(report, async () => {}, true);
    const graph = ui.nodes.get('flow-graph');
    const geometryWidth = Number(graph.attrs.viewBox.split(' ')[2]);
    assert.equal(graph.style.minWidth, `${geometryWidth}px`);
    assert.ok(geometryWidth > 390);
    ui.nodes.get('search').value = 'worker'; ui.nodes.get('search').listeners.input();
    assert.equal(graph.style.minWidth, `${graph.attrs.viewBox.split(' ')[2]}px`);
  }
});
