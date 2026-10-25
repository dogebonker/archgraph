'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const html = fs.readFileSync(path.join(__dirname, '../assets/comparison.html'), 'utf8');
const script = html.match(/<script>\n([\s\S]*?)<\/script>/)[1];
const descendants = node => [node, ...node.children.flatMap(descendants)];
function element(tag) {
  return {
    tag, children: [], attrs: {}, style: {}, listeners: {}, value: '', hidden: false,
    setAttribute(name, value) { this.attrs[name] = String(value); },
    getAttribute(name) { return this.attrs[name] ?? null; },
    addEventListener(name, callback) { this.listeners[name] = callback; },
    append(...children) { this.children.push(...children); },
    replaceChildren(...children) { this.text = ''; this.children = children; },
    focus(options) { this.focused = true; this.focusOptions = options; },
    scrollIntoView(options) { this.scrolled = options; },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(child => child.textContent).join(''); },
    set innerHTML(_) { throw new Error('Repository data must not become markup'); },
  };
}
function evidence(overrides = {}) {
  return { line: 3, specifier: '../b/item', original_to: null, is_reexport: false, verdict: 'forbidden', rule: 'no-b', reasons: ['Keep the domain independent.'], sources: ['docs/adr/boundary.md'], known: false, ...overrides };
}
function state(from_node, to_node, values = [evidence()]) { return { from_node, to_node, evidence: values }; }
function fixture() {
  const snapshot = { label: 'before.json', commit: 'a'.repeat(40), dirty: false, tool_version: '0.1.0', language: 'typescript', collector: { name: 'native', version: '0.1.0' }, scan_root: '.', policy: { path: 'layout.yaml', sha256: '1234' } };
  return {
    comparison: { status: 'recorded-compatible', policy_changed: true, warnings: ['Matching settings do not establish repository identity.'], before: snapshot, after: { ...snapshot, label: 'after.json', dirty: true } },
    nodes: [
      { id: 'module:a', kind: 'module', label: 'a', status: 'changed', before: { files: ['src/a/main.ts', 'src/a/old.ts', 'src/a/internal.ts'] }, after: { files: ['src/a/main.ts', 'src/a/internal.ts'] } },
      { id: 'module:b', kind: 'module', label: 'b', status: 'unchanged', before: { files: ['src/b/item.ts'] }, after: { files: ['src/b/item.ts'] } },
      { id: 'module:gone', kind: 'module', label: 'gone', status: 'removed', before: { files: ['src/moved.ts'] }, after: null },
      { id: 'module:new', kind: 'module', label: 'new', status: 'added', before: null, after: { files: ['src/moved.ts'] } },
      { id: 'external:pkg:dep', kind: 'package', label: 'pkg:dep', status: 'unchanged', before: { files: [] }, after: { files: [] } },
      { id: 'external:__proto__', kind: 'global', label: 'global:window', status: 'unchanged', before: { files: [] }, after: { files: [] } },
    ],
    imports: [
      { from: 'src/a/main.ts', to: 'src/b/item.ts', kind: 'runtime', status: 'added', changes: [], before: null, after: state('module:a', 'module:b', [evidence(), evidence({ line: 5, original_to: 'src/b/index.ts', known: true })]) },
      { from: 'src/a/old.ts', to: 'src/b/item.ts', kind: 'runtime', status: 'removed', changes: [], before: state('module:a', 'module:b', [evidence({ line: 7 })]), after: null },
      { from: 'src/a/main.ts', to: 'pkg:dep', kind: 'runtime', status: 'changed', changes: ['verdict', 'rule'], before: state('module:a', 'external:pkg:dep', [evidence({ verdict: 'allowed', rule: 'old-allow', reasons: ['Before reason.'], line: 8 })]), after: state('module:a', 'external:pkg:dep', [evidence({ rule: 'no-dependency', reasons: ['After reason.'], line: 8 })]) },
      { from: 'src/a/main.ts', to: 'src/a/internal.ts', kind: 'type', status: 'changed', changes: ['evidence'], before: state('module:a', 'module:a', [evidence({ line: 1, verdict: 'allowed', rule: null, reasons: [], sources: [] })]), after: state('module:a', 'module:a', [evidence({ line: 9, verdict: 'allowed', rule: null, reasons: [], sources: [] })]) },
      { from: 'src/b/item.ts', to: 'global:window', kind: 'runtime', status: 'unchanged', changes: [], before: state('module:b', 'external:__proto__', [evidence({ verdict: null, rule: null, reasons: [], sources: [], line: null })]), after: state('module:b', 'external:__proto__', [evidence({ verdict: null, rule: null, reasons: [], sources: [], line: null })]) },
      { from: 'src/moved.ts', to: 'src/b/item.ts', kind: 'runtime', status: 'changed', changes: ['module'], before: state('module:gone', 'module:b'), after: state('module:new', 'module:b') },
    ],
  };
}
function setup(view = fixture(), clipboard = async () => {}, stacked = false) {
  const nodes = new Map();
  for (const [,tag,id] of html.matchAll(/<(\w+)[^>]*\bid="([^"]+)"/g)) { const node = element(tag); node.setAttribute('id',id); nodes.set(id,node); }
  nodes.get('comparison').textContent = JSON.stringify(view);
  const all = () => [...nodes.values()].flatMap(descendants);
  const copied = [];
  const document = {
    getElementById: id => all().find(node => node.attrs.id === id) || null,
    createElement: element,
    createElementNS: (namespace,tag) => { assert.equal(namespace, 'http://www.w3.org/2000/svg'); return element(tag); },
  };
  const ctx = vm.createContext({ document, matchMedia: query => {
    assert.equal(query, '(max-width:1180px)'); return { matches:stacked };
  }, navigator: { clipboard: { writeText: async text => { copied.push(text); await clipboard(text); } } } });
  vm.runInContext(script,ctx);
  const find = (id,predicate) => descendants(document.getElementById(id)).find(predicate);
  const byText = (id,text) => find(id,node => node.tag === 'button' && node.textContent.includes(text));
  return { nodes, document, copied, find, byText, text: id => document.getElementById(id).textContent };
}
test('comparison totals distinguish structural, rule, and evidence changes under an explicit policy notice', () => {
  const ui = setup();
  assert.match(ui.text('totals'), /Modules4Imports6\+ Added1− Removed1Rule changed1Evidence only1/);
  assert.match(ui.text('change-summary'), /3 structural import changes · 1 rule changes · 1 evidence-only updates/);
  assert.equal(ui.text('import-count'), 'Showing 5 of 5');
  assert.match(ui.text('compatibility'), /Recorded collection settings match/);
  assert.match(ui.text('policy-notice'), /Policy changed/);
  assert.match(ui.text('warnings'), /repository identity/);
  assert.match(ui.text('inspector-body'), /Recorded basea{40}Tree stateclean working tree.*After snapshot.*dirty working tree/);
});
test('mixed connection retains added and removed imports even when its net size is unchanged', () => {
  const ui = setup();
  const connection = ui.byText('connection-list', 'a → b: +1 added · −1 removed');
  assert.ok(connection);
  connection.listeners.click();
  assert.equal(ui.nodes.get('all-mode').attrs['aria-pressed'], 'true');
  assert.equal(ui.text('import-count'), 'Showing 2 of 6');
  assert.match(ui.text('import-list'), /\+ Added.*src\/a\/main.ts.*− Removed.*src\/a\/old.ts/);
  assert.match(ui.text('inspector-body'), /Before1 import identitiesAfter1 import identities/);
  ui.byText('import-list', 'src/a/old.ts').listeners.click();
  assert.match(ui.text('inspector-body'), /src\/a\/old.ts:7.*Not present in the after snapshot/);
});
test('before and after rule reasons and every retained occurrence remain inspectable', () => {
  const ui = setup();
  ui.byText('import-list', 'pkg:dep').listeners.click();
  assert.match(ui.text('inspector-body'), /Before.*VerdictallowedRuleold-allow.*Before reason.*After.*VerdictforbiddenRuleno-dependency.*After reason/);
  ui.byText('import-list', 'src/a/main.ts → src/b/item.ts').listeners.click();
  assert.match(ui.text('inspector-body'), /src\/a\/main.ts:3.*src\/a\/main.ts:5.*Original targetsrc\/b\/index.ts.*Recorded known violation/);
  assert.match(ui.text('inspector-body'), /docs\/adr\/boundary.md/);
});
test('line movement is evidence only and self-connections remain selectable', () => {
  const ui = setup();
  assert.match(ui.text('import-list'), /Evidence only · type/);
  const loop = ui.byText('connection-list', 'a → a: 1 evidence only');
  assert.ok(loop);
  loop.listeners.click();
  ui.byText('import-list', 'src/a/internal.ts').listeners.click();
  assert.match(ui.text('inspector-kind'), /^Evidence only · type$/);
  assert.match(ui.text('inspector-body'), /Before.*src\/a\/main.ts:1.*After.*src\/a\/main.ts:9/);
});
test('module reassignment is moved in/out rather than a new import', () => {
  const ui = setup();
  assert.ok(ui.byText('connection-list', 'gone → b: 1 moved out'));
  assert.ok(ui.byText('connection-list', 'new → b: 1 moved in'));
  ui.byText('node-list', 'gone').listeners.click();
  assert.match(ui.text('inspector-kind'), /module · − Removed/);
  assert.match(ui.text('inspector-body'), /Before1 recorded filessrc\/moved.tsAfterNot present/);
  assert.match(ui.text('import-list'), /Membership moved/);
});
test('search, clear, and all-imports controls preserve global counts and reveal packages and globals', () => {
  const ui = setup();
  ui.nodes.get('search').value = 'nothing-matches'; ui.nodes.get('search').listeners.input();
  assert.equal(ui.text('import-count'), 'Showing 0 of 5');
  assert.match(ui.text('import-list'), /No matching changes/);
  assert.equal(ui.nodes.get('graph-wrap').hidden,true);
  assert.match(ui.text('totals'), /Imports6/);
  ui.nodes.get('clear').listeners.click();
  assert.equal(ui.nodes.get('search').focused,true);
  assert.equal(ui.text('import-count'),'Showing 5 of 5');
  ui.nodes.get('all-mode').listeners.click();
  assert.equal(ui.text('import-count'),'Showing 6 of 6');
  ui.byText('node-list','global:window').listeners.click();
  assert.equal(ui.text('inspector-kind'),'global · Unchanged');
  assert.match(ui.text('inspector-body'), /External global/);
  ui.nodes.get('clear').listeners.click();
  ui.byText('node-list','pkg:dep').listeners.click();
  assert.match(ui.text('inspector-body'), /External package/);
});
test('graph keyboard activation preserves focus for nodes and connections', () => {
  const ui = setup();
  const node = ui.find('module-graph', n => n.attrs['aria-label'] === 'Inspect module gone, − Removed');
  let prevented = false;
  node.listeners.keydown({key:'Enter',preventDefault(){prevented=true;}});
  assert.equal(prevented,true);
  assert.equal(ui.text('inspector-title'),'gone');
  assert.equal(ui.find('module-graph', n => n.attrs['aria-label'] === 'Inspect module gone, − Removed').focused,true);
  assert.equal(ui.nodes.get('inspector-title').focused,undefined);
  assert.equal(ui.nodes.get('inspector').scrolled,undefined);
  const link = ui.find('module-graph', n => n.attrs.class === 'graph-hit');
  link.listeners.keydown({key:' ',preventDefault(){}});
  assert.equal(ui.text('inspector-kind'),'Module connection');
  assert.equal(ui.find('module-graph', n => n.attrs.class === 'graph-hit').focused,true);
});
test('stacked selection reveals the inspector and connection selection reveals filtered imports', () => {
  const ui = setup(fixture(),async () => {},true);
  const heading = ui.nodes.get('inspector-title'); const inspector = ui.nodes.get('inspector');
  ui.byText('node-list','gone').listeners.click();
  assert.equal(ui.text('inspector-title'),'gone');
  assert.equal(heading.focused,true);
  assert.equal(heading.focusOptions.preventScroll,true);
  assert.equal(inspector.scrolled.block,'start');
  ui.nodes.get('clear').listeners.click(); heading.focused = false; inspector.scrolled = undefined;
  ui.byText('import-list','pkg:dep').listeners.click();
  assert.match(ui.text('inspector-title'), /pkg:dep/);
  assert.equal(heading.focused,true);
  assert.equal(inspector.scrolled.block,'start');
  ui.nodes.get('clear').listeners.click(); heading.focused = false; inspector.scrolled = undefined;
  ui.byText('connection-list','a → b: +1 added · −1 removed').listeners.click();
  assert.equal(ui.nodes.get('imports-title').focused,true);
  assert.equal(ui.nodes.get('imports-title').focusOptions.preventScroll,true);
  assert.equal(ui.nodes.get('imports-title').scrolled.block,'start');
  assert.equal(heading.focused,false);
  assert.equal(inspector.scrolled,undefined);
  assert.equal(ui.text('import-count'),'Showing 2 of 6');
  ui.byText('import-list','src/a/old.ts').listeners.click();
  assert.equal(heading.focused,true);
  assert.equal(inspector.scrolled.block,'start');
});
test('unverified no-change view keeps provenance warnings and does not claim complete compliance', () => {
  const view = fixture(); view.imports = []; view.nodes = []; view.comparison.status = 'unverified'; view.comparison.policy_changed = false;
  view.comparison.warnings = ['Original analyzer version is unavailable.'];
  const ui = setup(view);
  assert.equal(ui.text('compatibility'),'Unverified comparison');
  assert.match(ui.text('warnings'), /Original analyzer version is unavailable/);
  assert.match(ui.text('import-list'), /No structural or rule changes observed in the supplied snapshots/);
  assert.equal(ui.text('import-count'),'Showing 0 of 0');
  assert.equal(ui.text('graph-count'),'0 nodes · 0 connections');
});
test('repository text stays literal and opaque prototype-like node IDs remain safe', () => {
  const view = fixture(); const unsafe = '</script><img src=x onerror=alert(1)>';
  view.nodes[0].label = unsafe; view.imports[0].after.evidence[0].reasons = [unsafe];
  const ui = setup(view);
  assert.ok(ui.byText('node-list',unsafe));
  ui.byText('import-list','src/a/main.ts → src/b/item.ts').listeners.click();
  assert.ok(ui.text('inspector-body').includes(unsafe));
  assert.equal(descendants(ui.document.getElementById('inspector-body')).filter(n => n.tag === 'img' || n.tag === 'script').length,0);
  ui.nodes.get('all-mode').listeners.click();
  assert.ok(ui.byText('node-list','global:window'));
  assert.ok(!/\b(?:fetch|XMLHttpRequest)\s*\(/.test(script));
  assert.ok(html.includes("default-src 'none'"));
});
test('source copy is exact and failure retains selectable source text', async () => {
  const ui = setup();
  ui.byText('import-list','src/a/main.ts → src/b/item.ts').listeners.click();
  await ui.byText('inspector-body','Copy source location').listeners.click();
  assert.deepEqual(ui.copied,['src/a/main.ts:3']);
  assert.equal(ui.text('announcement'),'Source location copied.');
  const denied = setup(fixture(),async () => {throw new Error('denied');});
  denied.byText('import-list','pkg:dep').listeners.click();
  await denied.byText('inspector-body','Copy source location').listeners.click();
  assert.match(denied.text('announcement'), /Select and copy/);
  assert.match(denied.text('inspector-body'), /src\/a\/main.ts:8/);
});
test('mobile graph retains readable SVG sizing and labels', () => {
  const ui = setup();
  const graph = ui.nodes.get('module-graph');
  assert.equal(graph.style.minWidth, `${graph.attrs.viewBox.split(' ')[2]}px`);
  assert.match(html, /@media\(max-width:680px\)/);
  assert.match(html, /\.graph-node text \{[^}]*font:13px/);
});
