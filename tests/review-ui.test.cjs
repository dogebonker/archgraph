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
function node(tag) {
  return {
    tag, children: [], listeners: {}, attrs: {},
    get childNodes() { return this.children; },
    setAttribute(k, v) { this.attrs[k] = String(v); },
    addEventListener(event, fn) { this.listeners[event] = fn; },
    append(...children) { this.children.push(...children); },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(c => c.textContent).join(''); },
    set innerHTML(_) { throw new Error('repository text must not become markup'); },
  };
}
const descendants = root => [root, ...(root.children || []).flatMap(descendants)];
const evil = '<img src=x onerror=alert(1)>';
/* The symbol a cell shows, without the words screen readers hear. */
const symbol = td => descendants(td).find(n => n.attrs?.['aria-hidden'] === 'true' || n.tag === 'button').textContent;
function target() {
  return {
    claims: [
      { id: 'layered', reason: 'Dependencies point inward', source: 'deck.pdf slide 8', verdict: 'holds_in', notes: [], slices: [
        { slice: 'auth', files: ['internal/auth/handler.go'], broken: [] },
        { slice: 'sop', files: ['internal/sop/handler.go', 'internal/sop/repository.go'],
          broken: [{ from: 'internal/sop/handler.go', to: 'internal/sop/repository.go' }, { from: 'internal/sop/handler.go', to: evil }] },
      ] },
      { id: 'service-no-sql', reason: evil, source: null, verdict: 'broken', notes: [], slices: [
        { slice: 'apikey', files: ['internal/apikey/service.go'], broken: [{ from: 'internal/apikey/service.go', to: 'internal/apikey/sql.go' }] },
        { slice: '*', files: ['internal/x.go'], broken: [] },
      ] },
    ],
    moves: [],
    progress: { conforming: { done: 16, of: 18 }, moved: { done: 0, of: 23 } },
  };
}
function setup(doc) {
  const targets = [];
  doc = { config: { rules: { path: 'layout.yaml' } }, summary: { forbidden: 2, cycles: 0 }, ...doc };
  const ctx = vm.createContext({ document: { createElement: node, createTextNode: text => ({ textContent: text }) },
    doc, short: x => x, go: t => targets.push(t),
    fileIndex: new Map([['internal/sop/handler.go', 0], ['internal/sop/repository.go', 1], ['internal/apikey/service.go', 2]]),
    edgeOf: l => (l.to === evil ? undefined : 7) });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), ctx);
  vm.runInContext(fragment('function btn(', 'const icon ='), ctx);
  vm.runInContext(fragment('/* ---------- target panel', '/* ---------- end target panel'), ctx);
  vm.runInContext(fragment('/* ---------- review page', '/* ---------- end review page'), ctx);
  return { ctx, targets };
}
test('the page opens on Review with a target and on Map without one; the hash overrides either', () => {
  const cases = [
    [{ target: target() }, '', 'review'], [{ target: target() }, '#map', 'map'], [{ target: target() }, '#review', 'review'],
    [{ target: target() }, '#other', 'review'], [{}, '', 'map'], [{}, '#review', 'review'], [{}, '#map', 'map'],
  ];
  for (const [doc, hash, view] of cases) {
    const { ctx } = setup(doc);
    ctx.hash = hash;
    assert.equal(vm.runInContext('viewOf(hash)', ctx), view, `${doc.target ? 'target' : 'no target'} ${hash || '(no hash)'}`);
  }
});
test('the verdict strip counts slices, moves, forbidden imports and cycles, leaving out what does not apply', () => {
  const strip = doc => vm.runInContext('verdictStrip()', setup(doc).ctx).textContent;
  assert.equal(strip({ target: target() }), '16 of 18 slices conform · 0 of 23 slices moved · 2 forbidden imports · 0 cycles');
  const t = target();
  delete t.progress.moved;
  assert.equal(strip({ target: t, summary: { forbidden: 1, cycles: 3 } }), '16 of 18 slices conform · 1 forbidden import · 3 cycles');
  assert.equal(strip({ config: { rules: null }, summary: { forbidden: 0, cycles: 1 } }), '1 cycle');
});
test('the claims matrix has a column per claim and a cell per slice: holds, broken with its count, or not bound', () => {
  const { ctx } = setup({ target: target() });
  const table = vm.runInContext('claimsMatrix()', ctx);
  assert.equal(table.tag, 'table');
  const [thead, tbody] = table.children;
  const head = thead.children[0].children;
  assert.deepEqual(head.map(th => th.attrs.scope), ['col', 'col', 'col']);
  assert.equal(head[0].textContent, 'Slice');
  assert.deepEqual(head[1].children.map(c => c.textContent), ['layered', 'holds in 1 of 2 slices', 'Dependencies point inward', 'deck.pdf slide 8']);
  assert.deepEqual(head[2].children.map(c => c.textContent), ['service-no-sql', 'broken', evil]);
  const rows = tbody.children.map(tr => [tr.children[0].textContent, ...tr.children.slice(1).map(symbol)]);
  assert.deepEqual(rows, [['apikey', '·', '✗ 1'], ['sop', '✗ 2', '·'], ['*', '·', '✓'], ['auth', '✓', '·']]);
  assert.ok(tbody.children.every(tr => tr.children[0].tag === 'th' && tr.children[0].attrs.scope === 'row'));
  const unbound = tbody.children[0].children[1];
  assert.ok(descendants(unbound).some(n => n.attrs?.title === "rule can't fail here"));
  assert.ok(unbound.textContent.includes("rule can't fail here"), 'screen readers hear why the cell is empty');
  assert.ok(tbody.children[3].children[1].textContent.includes('holds'));
  const buttons = descendants(table).filter(n => n.tag === 'button');
  assert.deepEqual(buttons.map(b => b.textContent), ['✗ 1', '✗ 2'], 'only broken cells are buttons, so Tab walks them');
  assert.ok(buttons.every(b => b.attrs.type === 'button'));
  assert.equal(buttons[1].attrs['aria-label'], '2 broken imports, show on the map');
});
test('a broken cell selects its slice as the Target panel does: its files and broken imports', () => {
  const { ctx, targets } = setup({ target: target() });
  const table = vm.runInContext('claimsMatrix()', ctx);
  const [one, two] = descendants(table).filter(n => n.tag === 'button');
  two.listeners.click();
  let t = targets.at(-1);
  assert.equal(t.type, 'target');
  assert.equal(t.title, 'layered: sop');
  assert.deepEqual([...t.fs], [0, 1]);
  assert.deepEqual([...t.fe], [7]);
  one.listeners.click();
  t = targets.at(-1);
  assert.equal(t.title, 'service-no-sql: apikey');
  assert.deepEqual([...t.fs], [2]);
  assert.deepEqual([...t.fe], [7]);
});
test('the page says when there is no target and has no matrix', () => {
  const { ctx } = setup({});
  ctx.out = node('div');
  vm.runInContext('reviewPage(out)', ctx);
  assert.ok(!descendants(ctx.out).some(n => n.tag === 'table'));
  assert.match(ctx.out.textContent, /The rules state no claim or move/);
});
test('the view switch is wired to the hash, the header and the selection path without markup', () => {
  const review = fragment('/* ---------- review page', '/* ---------- end review page');
  assert.ok(!review.includes('innerHTML'));
  assert.match(review, /addEventListener\("hashchange", \(\) => showView\(viewOf\(location\.hash\)\)\)/);
  assert.match(script, /function go\(t\) \{ if \(reviewing\) location\.hash = "#map"; select\(t\);/);
  assert.match(script, /initReview\(\);\nshowPanel\(\);/);
  assert.match(html, /<div class="seg glass capsule" role="radiogroup" aria-label="View" id="views">/);
  assert.match(html, /<main id="review-page" aria-label="Review">/);
  assert.doesNotThrow(() => new vm.Script(script));
});
