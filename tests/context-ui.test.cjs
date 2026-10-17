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
  assert.ok(a >= 0 && b > a, `missing script fragment: ${start}`);
  return script.slice(a, b);
}
function node(tag = 'div') {
  return {
    tag, children: [], attrs: {}, listeners: {}, hidden: true,
    setAttribute(key, value) { this.attrs[key] = value; },
    addEventListener(event, handler) { this.listeners[event] = handler; },
    append(...children) { this.children.push(...children); },
    set textContent(text) { this.text = String(text); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(c => c.textContent).join(''); },
    set innerHTML(_) { throw new Error('Untrusted content must use textContent'); },
  };
}
function context(extra = {}) {
  const ctx = vm.createContext({
    document: { createElement: node, createTextNode: text => ({ textContent: text }) },
    ...extra,
  });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), ctx);
  return ctx;
}

test('complete graph script is valid JavaScript', () => {
  assert.doesNotThrow(() => new vm.Script(script));
});

test('churn marks encode frequency and summaries explain counts and cycle size', () => {
  const ctx = context();
  assert.equal(vm.runInContext('churnMark(0).scale', ctx), 1);
  assert.equal(vm.runInContext('churnMark(1).scale', ctx), 3);
  assert.equal(vm.runInContext('churnMark(99).scale', ctx), 3);
  assert.equal(vm.runInContext('churnMark(NaN).scale', ctx), 1);
  assert.notEqual(vm.runInContext('churnMark(0).color', ctx), vm.runInContext('churnMark(1).color', ctx));
  assert.equal(vm.runInContext('hotspotSummary({commits:41,authors:9,cycle_size:27})', ctx),
    'Changed 41 times by 9 authors; in a 27-module cycle.');
  assert.match(vm.runInContext('hotspotSummary({commits:1,authors:1,cycle_size:0})', ctx), /1 time by 1 author; outside module cycles/);
});

test('churn controls leave older and shallow documents unchanged and toggle accessibly', () => {
  const code = fragment('if (doc.hotspots && !doc.hotspots.shallow && hotspotFiles.length)', '/* Popovers grow');
  for (const doc of [{}, { hotspots: { shallow: true } }]) {
    const controls = { 't-churn': node(), 'churn-legend': node() };
    const ctx = vm.createContext({ doc, hotspotFiles: [{}], $: id => controls[id] });
    vm.runInContext(code, ctx);
    assert.equal(controls['t-churn'].hidden, true);
    assert.equal(controls['t-churn'].listeners.click, undefined);
  }
  const controls = { 't-churn': node(), 'churn-legend': node() };
  let redraws = 0;
  const ctx = vm.createContext({ doc: { hotspots: { shallow: false } }, hotspotFiles: [{}],
    churnView: false, $: id => controls[id], redraw: () => redraws++ });
  vm.runInContext(code, ctx);
  assert.equal(controls['t-churn'].hidden, false);
  controls['t-churn'].listeners.click({ currentTarget: controls['t-churn'] });
  assert.equal(controls['t-churn'].attrs['aria-pressed'], 'true');
  assert.equal(controls['churn-legend'].hidden, false);
  controls['t-churn'].listeners.click({ currentTarget: controls['t-churn'] });
  assert.equal(controls['t-churn'].attrs['aria-pressed'], 'false');
  assert.equal(controls['churn-legend'].hidden, true);
  assert.equal(redraws, 2);
});

test('provenance distinguishes verified introduction from unknown boundaries without inventing identity', () => {
  const commit = { commit: 'a'.repeat(40), author: '<script>Eve</script>', email: 'eve@example.test',
    committed_at: '2026-10-16', subject: '<img src=x onerror=alert(1)>', age_days: 41, pull_request: 99 };
  const provenance = { status: 'verified-transition', method: 'first-parent', scans: 9,
    from: 'old', to: 'new', previous_commit: 'old', closing_edge: { from: 'a.java', to: 'b.java' }, limitations: ['bounded search'] };
  const ctx = context({ finding: { introduced_by: commit, provenance } });
  const text = vm.runInContext('provenanceLines(finding).join("\\n")', ctx);
  assert.match(text, /Verified introduction: aaaaaaaaaaaa by <script>Eve<\/script>/);
  assert.match(text, /Age: 41 days/);
  assert.match(text, /PR #99 inferred from the commit message/);
  assert.match(text, /Cycle-closing import: a.java → b.java/);
  assert.ok(!text.includes('github.com'));
  assert.equal(vm.runInContext('el("p", {}, provenanceLines(finding).join("\\n")).textContent', ctx), text);
  ctx.finding.age_days = 3;
  assert.match(vm.runInContext('provenanceLines(finding).join(" ")', ctx), /Recent architecture change/);
  assert.match(vm.runInContext('findingAge(finding)', ctx), /new · 3 days/);
  ctx.finding = { introduced_by: commit, provenance: { ...provenance, status: 'present-at-boundary', limitations: ['shallow clone'] } };
  const unknown = vm.runInContext('provenanceLines(finding).join("\\n")', ctx);
  assert.equal(vm.runInContext('findingAge(finding)', ctx), '');
  assert.match(unknown, /Introduction unknown: already present/);
  assert.match(unknown, /shallow clone/);
  assert.ok(!unknown.includes('Verified introduction'));
  assert.equal(vm.runInContext('provenanceLines({}).length', ctx), 0);
  assert.match(vm.runInContext('provenanceLines({age_days:2}).join(" ")', ctx), /unverified/);
});

test('hotspot panels render factors as text and link the existing associated finding', () => {
  const panel = node(), targets = [];
  const ctx = context({ panel, doc: { hotspots: { shallow: false } }, short: x => x,
    fileIndex: new Map([['<script>file</script>', 0]]), go: target => targets.push(target),
    findings: [{ i: 0, f: { subject: '<b>cycle</b>', kind: 'cycle' } }],
    looksLikePath: () => false, subjectText: f => f.subject });
  vm.runInContext(fragment('function btn(', 'const icon ='), ctx);
  vm.runInContext(fragment('function findingList(', '/* A sentence of text'), ctx);
  ctx.row = { path: '<script>file</script>', commits: 41, authors: 9, cycle_size: 27,
    score: 2, churn: 1, coupling: 0.5, cycle: 1, author_factor: 0.5,
    lines_added: 10, lines_deleted: 4, cut_candidate: true, finding_indices: [0] };
  vm.runInContext('hotspotContext([row])', ctx);
  assert.match(panel.textContent, /Changed 41 times by 9 authors; in a 27-module cycle/);
  assert.match(panel.textContent, /× \(coupling 0.50 \+ cycle factor 1\) × \(1 \+ author factor 0.50\)/);
  assert.match(panel.textContent, /<script>file<\/script>/);
  const list = panel.children.at(-1), button = list.children[0].children[0];
  button.listeners.click();
  assert.equal(targets[0].type, 'finding');
  assert.equal(targets[0].i, 0);
});

test('history promotes existing hotspot cuts, preserving ties and excluding unrelated moves', () => {
  const ctx = vm.createContext({
    doc: { hotspots: { shallow: false } },
    findings: [{ f: { kind: 'cycle', hotspot_priority: 4 } }, { f: { kind: 'misplaced-file', hotspot_priority: 9 } }],
    edges: [{ a: 0, b: 1, verdict: null }, { a: 1, b: 2, verdict: 'forbidden' }, { a: 0, b: 2, verdict: null }],
    files: [{ path: 'hot.java' }, { path: 'cold.java' }, { path: 'other.java' }],
    breaks: new Set(['hot.java\ncold.java']), hotspotByPath: new Map([['hot.java', { score: 5 }], ['other.java', { score: 2 }]]),
  });
  vm.runInContext(fragment('function hotspotStepPriority(', '/* Ring colour comes'), ctx);
  assert.equal(vm.runInContext('hotspotStepPriority([{fd:0}])', ctx), 4);
  assert.equal(vm.runInContext('hotspotStepPriority([{fd:1}])', ctx), 0);
  assert.equal(vm.runInContext('hotspotStepPriority([{e:0}])', ctx), 5);
  assert.equal(vm.runInContext('hotspotStepPriority([{e:1}])', ctx), 2);
  assert.equal(vm.runInContext('hotspotStepPriority([{e:2}])', ctx), 0);
  assert.equal(vm.runInContext('JSON.stringify([[{fd:1}], [{e:0}], [{fd:0}], ["tie"]].sort((a,b)=>hotspotStepPriority(b)-hotspotStepPriority(a)))', ctx),
    JSON.stringify([[{e:0}], [{fd:0}], [{fd:1}], ['tie']]));
  ctx.doc = {};
  assert.equal(vm.runInContext('hotspotStepPriority([{e:0}])', ctx), 0);
});

test('assessment and module next steps ask for ownership review when a move conflicts', () => {
  const fd = { i: 0, f: { subject: 'src/file.java', simulation: {}, evidence: {
    target_module: 'target', ownership: { assessment: 'conflicts' },
  } } };
  const next = [], spoken = [];
  const ctx = vm.createContext({ next, mv: fd, short: x => x, cohesionGain: () => 0,
    ofKind: () => [fd], say: (...parts) => spoken.push(...parts) });
  vm.runInContext(fragment('  const moves = ofKind("misplaced-file")', '  /* The split that moves'), ctx);
  assert.match(next[0][0].label, /Review ownership/);
  assert.match(next[0][1], /before moving/);
  vm.runInContext(fragment('  if (mv) say(', '  return out;'), ctx);
  assert.match(spoken[2].label, /review ownership.*before proposing a move/);
});
