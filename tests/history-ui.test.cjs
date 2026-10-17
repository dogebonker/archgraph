'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

test('timeline navigation renders the selected graph and preserves literal replacement tokens', () => {
  const root = path.join(__dirname, '..');
  const html = fs.readFileSync(path.join(root, 'src/history.html'), 'utf8');
  const template = fs.readFileSync(path.join(root, 'assets/graph.html'), 'utf8');
  const graphs = [0, 1].map((index) => ({
    files: [{ path: `src/$&{{GRAPH}}</script>${index}.java` }],
    modules: [], edges: [], summary: { cycles: index, forbidden: 0 },
  }));
  const history = { snapshots: graphs.map((graph, i) => ({
    commit: String(i).repeat(40), committed_at: '2026-10-16', subject: '<script>text</script>',
    graph, diff: i ? { added_modules: ['module'] } : null,
  })) };
  const elements = Object.fromEntries(['data', 'commit', 'previous', 'next', 'graph', 'summary', 'changes'].map((id) => [id, {
    listeners: {}, options: [], selectedIndex: -1,
    get value() { return String(this.selectedIndex); },
    append(option) { this.options.push(option); },
    addEventListener(event, listener) { this.listeners[event] = listener; },
  }]));
  elements.data.textContent = JSON.stringify([history, template]);
  vm.runInNewContext(html.match(/<script>\n([\s\S]*?)<\/script>/)[1], {
    document: { getElementById: (id) => elements[id], createElement: () => ({}) },
  });
  const selectedGraph = () => {
    const text = elements.graph.srcdoc.match(/<script type="application\/json" id="graph">([\s\S]*?)<\/script>/)[1];
    assert.ok(!text.includes('<'), 'graph data can close its script element');
    return JSON.parse(text);
  };
  assert.equal(elements.commit.options.length, 2);
  assert.deepEqual(selectedGraph(), graphs[1]);
  assert.equal(elements.next.disabled, true);
  assert.equal(elements.previous.disabled, false);
  elements.previous.listeners.click();
  assert.deepEqual(selectedGraph(), graphs[0]);
  assert.equal(elements.previous.disabled, true);
  assert.equal(elements.changes.textContent, 'First snapshot — no earlier comparison.');
  elements.next.listeners.click();
  assert.deepEqual(selectedGraph(), graphs[1]);
  assert.deepEqual(JSON.parse(elements.changes.textContent), history.snapshots[1].diff);
  elements.commit.selectedIndex = 0;
  elements.commit.listeners.change();
  assert.deepEqual(selectedGraph(), graphs[0]);
});

function driftPage(history) {
  const root = path.join(__dirname, '..');
  const html = fs.readFileSync(path.join(root, 'src/history.html'), 'utf8');
  const template = fs.readFileSync(path.join(root, 'assets/graph.html'), 'utf8');
  const element = (tagName = '') => ({
    tagName, children: [], attributes: {}, listeners: {}, selectedIndex: -1, hidden: true,
    get value() { return String(this.selectedIndex); },
    set value(value) { this.optionValue = value; },
    set innerHTML(_) { throw new Error('Repository text must use textContent'); },
    append(child) { this.children.push(child); },
    setAttribute(name, value) { this.attributes[name] = value; },
    addEventListener(event, listener) { this.listeners[event] = listener; },
  });
  const elements = Object.fromEntries(['data', 'commit', 'previous', 'next', 'graph', 'summary', 'changes', 'trend', 'failures', 'failure-list'].map(id => [id, element()]));
  elements.data.textContent = JSON.stringify([history, template]);
  const context = vm.createContext({ document: {
    getElementById: id => elements[id], createElement: element,
    createElementNS: (namespace, tagName) => {
      assert.equal(namespace, 'http://www.w3.org/2000/svg');
      return element(tagName);
    },
  } });
  const scripts = [...html.matchAll(/<script>\n([\s\S]*?)<\/script>/g)];
  assert.ok(scripts.length >= 2, 'Both navigation and chart scripts must execute');
  for (const script of scripts) vm.runInContext(script[1], context);
  return elements;
}

function driftSnapshot(index) {
  return {
    commit: String(index).repeat(40), committed_at: `2026-0${index + 1}-01T12:00:00Z`,
    subject: '<script>subject</script>', tags: index === 1 ? ['v2<script>'] : [],
    summary: { cycles: index, modules_in_cycles_percent: index * 50, cycles_per_100_modules: index * 25 },
    graph: { files: [{ path: `src/$&{{GRAPH}}</script>${index}.ts` }], modules: [], edges: [], summary: { cycles: index, forbidden: 0 } },
    diff: index ? { new_cycles: [index] } : null,
  };
}

test('drift chart selects maps with click and keyboard and shows releases and failures as text', () => {
  const history = {
    snapshots: [0, 1, 2].map(driftSnapshot),
    release_markers: [{ commit: '1'.repeat(40), committed_at: '2026-02-01T12:00:00Z', tags: ['v2<script>'] }],
    failures: [{ commit: 'f'.repeat(40), error: 'collector <script>failed</script>' }],
  };
  const elements = driftPage(history);
  const children = elements.trend.children;
  const points = children.filter(el => el.tagName === 'circle');
  assert.equal(points.length, 3);
  assert.equal(children.filter(el => el.tagName === 'polyline').length, 2);
  for (const point of points) {
    assert.equal(point.attributes.role, 'button');
    assert.equal(point.attributes.tabindex, '0');
    assert.ok(Number.isFinite(Number(point.attributes.cx)));
    assert.ok(Number.isFinite(Number(point.attributes.cy)));
    assert.match(point.attributes['aria-label'], /module cycles.*% modules in cycles/);
  }
  assert.match(points[1].attributes['aria-label'], /v2<script>/);
  const marker = children.find(el => el.tagName === 'line' && el.attributes['stroke-dasharray']);
  assert.equal(marker.children[0].textContent, 'v2<script> · 2026-02-01T12:00:00Z');
  assert.equal(elements.failures.hidden, false);
  assert.equal(elements['failure-list'].textContent, 'ffffffff · collector <script>failed</script>');
  assert.match(elements.summary.textContent, /100.0% modules in cycles.*50.0 cycles\/100 modules/);
  const selectedGraph = () => JSON.parse(elements.graph.srcdoc.match(/<script type="application\/json" id="graph">([\s\S]*?)<\/script>/)[1]);
  points[0].listeners.click();
  assert.equal(elements.commit.selectedIndex, 0);
  assert.deepEqual(selectedGraph(), history.snapshots[0].graph);
  let prevented = 0;
  points[1].listeners.keydown({ key: 'Enter', preventDefault() { prevented++; } });
  assert.equal(elements.commit.selectedIndex, 1);
  assert.deepEqual(selectedGraph(), history.snapshots[1].graph);
  points[2].listeners.keydown({ key: ' ', preventDefault() { prevented++; } });
  assert.equal(elements.commit.selectedIndex, 2);
  assert.equal(prevented, 2);
  points[0].listeners.keydown({ key: 'Escape', preventDefault() { assert.fail('Unrelated keys should not be captured'); } });
  assert.equal(elements.commit.selectedIndex, 2);
});

test('single-snapshot drift chart has finite coordinates and no failure notice', () => {
  const elements = driftPage({ snapshots: [driftSnapshot(0)] });
  const point = elements.trend.children.find(el => el.tagName === 'circle');
  assert.ok(Number.isFinite(Number(point.attributes.cx)));
  assert.ok(Number.isFinite(Number(point.attributes.cy)));
  assert.equal(elements.failures.hidden, true);
  assert.equal(elements.previous.disabled, true);
  assert.equal(elements.next.disabled, true);
});
