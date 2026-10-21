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
    prepend(...children) { this.children.unshift(...children); },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(c => c.textContent).join(''); },
    replaceChildren(...children) { this.text = ''; this.children = children; },
    set innerHTML(_) { throw new Error('repository text must not become markup'); },
  };
}
const descendants = root => [root, ...(root.children || []).flatMap(descendants)];
const evil = '<img src=x onerror=alert(1)>';
/* What a cell shows: a hatch, a tick or a broken count, without the words screen readers hear. */
const symbol = td => td.attrs.class === 'rv-na' ? 'hatch'
  : descendants(td).find(n => n.attrs?.['aria-hidden'] === 'true' || n.tag === 'button').textContent;
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
    moves: [
      { slice: 'auth', files: [], import_lines: 0, names: [] },
      { slice: 'sop', files: [{ file: 'internal/sop/handler.go', to: 'internal/sop/handler/handler.go' }], import_lines: 12,
        names: [{ name: 'sop.store', declared_in: 'internal/sop/repository.go', used_by: ['internal/sop/handler.go'] }],
        new_cycles: [{ from: 'internal/sop/handler.go', to: 'internal/sop/repository.go' }] },
      { slice: 'apikey', files: [{ file: 'internal/apikey/service.go', to: 'internal/apikey/svc/service.go' },
        { file: 'internal/sop/repository.go', to: 'internal/apikey/svc/sql.go' }], import_lines: 9, names: null },
    ],
    progress: { conforming: { done: 16, of: 18 }, moved: { done: 0, of: 23 } },
  };
}
function setup(doc) {
  const targets = [];
  doc = { source: { language: 'go', scan_root: 'apps/api', collector: { name: 'go-types' }, git: { commit: '502938cb264c3e', dirty: true } },
    config: { source_root: '.', rules: { path: 'layout.yaml' } }, summary: { forbidden: 2, cycles: 0 }, ...doc };
  const ctx = vm.createContext({ document: { createElement: node, createTextNode: text => ({ textContent: text }) },
    doc, short: x => x, go: t => targets.push(t), setHover() {}, icon: () => node('svg'),
    nf: n => n.toLocaleString('en-US'), LANG: { typescript: 'TypeScript', python: 'Python' },
    assessment: { bad: [], next: [] }, mods: [{ id: 'internal/sop' }], files: [{ mod: 0 }, { mod: 0 }, { mod: 0 }],
    dirname: p => p.slice(0, Math.max(0, p.lastIndexOf('/'))), mx: [0], my: [0], modR: [30], SP: 10, R_DOT: 2.6, GAP: 34,
    scanDirs: new Set(['internal', 'internal/sop', 'internal/apikey']), destination: () => [9, 9],
    fileIndex: new Map([['internal/sop/handler.go', 0], ['internal/sop/repository.go', 1], ['internal/apikey/service.go', 2]]),
    edgeOf: l => (l.to === evil ? undefined : 7) });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), ctx);
  vm.runInContext(fragment('function btn(', 'const icon ='), ctx);
  vm.runInContext(fragment('/* ---------- target panel', '/* ---------- end target panel'), ctx);
  vm.runInContext(fragment('function refTarget(', '/* The repository'), ctx);
  vm.runInContext(fragment('const MARK = ', 'function overview()'), ctx);
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
test('the verdict says in sentences how many slices follow every claim, then moves, rules and cycles', () => {
  const says = (doc, patch = {}) => {
    const { ctx } = setup(doc);
    Object.assign(ctx.doc.summary, patch);
    return vm.runInContext('verdictText()', ctx).children.map(p => p.textContent);
  };
  assert.deepEqual(says({ target: target() }), ['16 of 18 slices follow every claim.', 'None of 23 slices has moved. 2 imports break a rule.']);
  const t = target();
  delete t.progress.moved;
  assert.deepEqual(says({ target: t }, { forbidden: 1, cycles: 3 }), ['16 of 18 slices follow every claim.', '1 import breaks a rule. 3 cycles remain.']);
  const all = target();
  all.progress = { conforming: { done: 18, of: 18 }, moved: { done: 23, of: 23 } };
  assert.deepEqual(says({ target: all }, { forbidden: 0 }), ['All 18 slices follow every claim.', 'All 23 slices have moved. No import breaks a rule.']);
  const none = target();
  none.progress = { conforming: { done: 0, of: 20 }, moved: { done: 4, of: 23 } };
  assert.deepEqual(says({ target: none }, { forbidden: 1800 }), ['None of 20 slices follows every claim.', '4 of 23 slices have moved. 1,800 imports break a rule.']);
  assert.deepEqual(says({ config: { rules: null } }, { cycles: 1 }), ['1 cycle remains.']);
  assert.deepEqual(says({ config: { rules: null } }), ['No cycles.']);
});
test('the title block names the path, language, collector, commit and rules', () => {
  const rows = doc => vm.runInContext('titleBlock()', setup(doc).ctx).children.map(d => d.children.map(c => c.textContent));
  assert.deepEqual(rows({}), [['Path', 'apps/api'], ['Language', 'go'], ['Collector', 'go-types'], ['Commit', '502938c with local changes'], ['Rules', 'layout.yaml']]);
  assert.deepEqual(rows({ source: { language: 'typescript', collector: { name: evil } }, config: { source_root: 'web', rules: null } }),
    [['Path', 'web'], ['Language', 'TypeScript'], ['Collector', evil], ['Rules', 'none']]);
});
test('the claims matrix has a column per claim and a cell per slice: holds, broken with its count, or not bound', () => {
  const { ctx } = setup({ target: target() });
  const table = vm.runInContext('claimsMatrix()', ctx);
  assert.equal(table.tag, 'table');
  const [thead, tbody] = table.children;
  const head = thead.children[0].children;
  assert.deepEqual(head.map(th => th.attrs.scope), ['col', 'col', 'col']);
  assert.equal(head[0].textContent, 'Slice');
  assert.deepEqual(head[1].children.map(c => c.textContent), ['layered', 'holds in 1 of 2 slices', 'Dependencies point inward', 'deck.pdf slide 8', '']);
  assert.deepEqual(head[2].children.map(c => c.textContent), ['service-no-sql', 'broken', evil, '']);
  /* The measured bar: one segment per bound slice, in row order. */
  const bar = th => th.children.at(-1).children.map(i => i.attrs.class);
  assert.deepEqual(bar(head[1]), ['no', 'ok']);
  assert.deepEqual(bar(head[2]), ['no', 'ok']);
  assert.equal(head[1].children.at(-1).attrs['aria-hidden'], 'true');
  const rows = tbody.children.map(tr => [tr.children[0].textContent, ...tr.children.slice(1).map(symbol)]);
  assert.deepEqual(rows, [['apikey', 'hatch', '1'], ['sop', '2', 'hatch'], ['*', 'hatch', '✓'], ['auth', '✓', 'hatch']]);
  assert.ok(tbody.children.every(tr => tr.children[0].tag === 'th' && tr.children[0].attrs.scope === 'row'));
  const unbound = tbody.children[0].children[1];
  assert.equal(unbound.attrs.title, "rule can't fail here");
  assert.ok(unbound.textContent.includes("rule can't fail here"), 'screen readers hear why the cell is empty');
  assert.ok(tbody.children[3].children[1].textContent.includes('holds'));
  const buttons = descendants(table).filter(n => n.tag === 'button');
  assert.deepEqual(buttons.map(b => b.textContent), ['1', '2'], 'only broken cells are buttons, so Tab walks them');
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
const rowsOf = table => table.children[1].children.map(tr => tr.children.map(c => c.textContent));
const sortBy = (table, label) => table.children[0].children[0].children.find(th => th.textContent === label).children[0].listeners.click();
test('the move cost table lists every slice to move, most files first, and sorts by any column', () => {
  const { ctx } = setup({ target: target() });
  const table = vm.runInContext('moveTable()', ctx);
  assert.equal(table.tag, 'table');
  const head = table.children[0].children[0].children;
  assert.deepEqual(head.map(th => th.textContent), ['Slice', 'Files', 'Import lines', 'Names to export', 'Imports in new package cycles', 'Moved']);
  assert.ok(head.every(th => th.attrs.scope === 'col' && th.children[0].tag === 'button'), 'each header sorts from the keyboard');
  assert.deepEqual(head.map(th => th.attrs['aria-sort']), ['none', 'descending', 'none', 'none', 'none', 'none']);
  assert.deepEqual(rowsOf(table), [['apikey', '2', '9', 'unknown', '0', 'not yet'], ['sop', '1', '12', '1', '1', 'not yet'], ['auth', '0', '0', '0', '0', 'moved']]);
  const cls = table.children[1].children.map(tr => tr.children.map(c => c.attrs.class));
  assert.equal(cls[0][3], 'muted', 'an unknown count reads as unknown, quietly');
  assert.equal(cls[0][5], 'muted', 'a pending slice reads not yet, unhatched: hatching means does not apply');
  assert.equal(cls[2][5], 'rv-moved');
  sortBy(table, 'Import lines');
  assert.deepEqual(rowsOf(table).map(r => r[0]), ['sop', 'apikey', 'auth']);
  assert.equal(head[2].attrs['aria-sort'], 'descending');
  assert.equal(head[1].attrs['aria-sort'], 'none');
  sortBy(table, 'Import lines');
  assert.deepEqual(rowsOf(table).map(r => r[0]), ['auth', 'apikey', 'sop']);
  assert.equal(head[2].attrs['aria-sort'], 'ascending');
  sortBy(table, 'Slice');
  assert.deepEqual(rowsOf(table).map(r => r[0]), ['apikey', 'auth', 'sop']);
  assert.equal(head[0].attrs['aria-sort'], 'ascending');
  ctx.doc.source.language = 'typescript';
  assert.deepEqual(rowsOf(vm.runInContext('moveTable()', ctx))[0], ['apikey', '2', '9', 'unknown', 'unknown', 'not yet'], 'cycles are simulated for Go only');
});
test('a pending move row selects that move as the Target panel does, with its destination discs', () => {
  const { ctx, targets } = setup({ target: target() });
  const table = vm.runInContext('moveTable()', ctx);
  const buttons = descendants(table.children[1]).filter(n => n.tag === 'button');
  assert.deepEqual(buttons.map(b => b.textContent), ['apikey', 'sop'], 'a moved slice has nothing to show');
  assert.equal(buttons[1].attrs['aria-label'], 'sop: show the move on the map');
  buttons[1].listeners.click();
  let t = targets.at(-1);
  assert.equal(t.kind, 'Pending move');
  assert.equal(t.title, 'sop');
  assert.deepEqual([...t.fs], [0]);
  assert.deepEqual([...t.fe], [7]);
  assert.deepEqual(Array.from(t.moves, mv => [mv.file, mv.to, mv.note]), [[0, 'internal/sop/handler/handler.go', 'new folder internal/sop/handler/']]);
  buttons[0].listeners.click();
  t = targets.at(-1);
  assert.deepEqual(Array.from(t.moves, mv => [mv.file, mv.to]), [[2, 'internal/apikey/svc/service.go'], [1, 'internal/apikey/svc/sql.go']]);
});
test('the page shows the side sheet\'s findings and next steps, each reference opening the map', () => {
  const { ctx, targets } = setup({ target: target() });
  ctx.assessment = { bad: [{ touch: 3, parts: ['The ', { m: 0 }, ' module imports ', { b: evil }, '.'] }], next: [['Fix ', { fd: 4, label: 'the cycle' }, '.']] };
  ctx.out = node('div');
  vm.runInContext('reviewPage(out)', ctx);
  const text = ctx.out.textContent;
  assert.ok(text.includes('Findings to review') && text.includes('What to do next'));
  assert.ok(text.includes(`The internal/sop module imports ${evil}.`));
  assert.ok(text.includes('Fix the cycle.'));
  const refs = descendants(ctx.out).filter(n => n.attrs?.role === 'button');
  assert.deepEqual(refs.map(r => r.textContent), ['internal/sop', 'the cycle']);
  refs[1].listeners.click();
  assert.deepEqual({ ...targets.at(-1) }, { type: 'finding', i: 4 });
  refs[0].listeners.keydown({ key: 'Enter', preventDefault() {} });
  assert.deepEqual({ ...targets.at(-1) }, { type: 'module', i: 0 });
  assert.deepEqual(descendants(ctx.out).filter(n => n.tag === 'table').map(n => n.attrs['aria-labelledby']), ['rv-claims', 'rv-moves']);
  assert.ok(text.startsWith('Review16 of 18 slices follow every claim.'), 'the verdict leads the page');
});
test('without moves there is no move cost table', () => {
  const t = target();
  t.moves = [];
  const { ctx } = setup({ target: t });
  ctx.out = node('div');
  vm.runInContext('reviewPage(out)', ctx);
  assert.deepEqual(descendants(ctx.out).filter(n => n.tag === 'table').map(n => n.attrs['aria-labelledby']), ['rv-claims']);
});
/* sRGB relative luminance and the WCAG contrast ratio, for colours read from the page's tokens. */
const lum = ([r, g, b]) => [r, g, b].map(v => v / 255).map(v => v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)
  .reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
const token = name => html.match(new RegExp(`--${name}: ([^;]+);`))[1];
const hex = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));
test('a broken cell is a tinted sheet cell with a red edge and a red count of at least 4.5:1, solid only under the pointer or focus', () => {
  const [r, g, b, a] = token('broken-tint').match(/[\d.]+/g).map(Number), sheet = hex(token('sheet'));
  const tinted = [r, g, b].map((v, i) => v * a + sheet[i] * (1 - a));
  assert.ok(ratio(hex(token('broken')), tinted) >= 4.5, `count contrast ${ratio(hex(token('broken')), tinted).toFixed(2)}`);
  assert.match(html, /td\.rv-no \{[^}]*background: var\(--broken-tint\);[^}]*box-shadow: inset 3px 0 0 var\(--broken\);/);
  assert.match(html, /\.rv-hit \{[^}]*color: var\(--broken\);/);
  assert.match(html, /\.rv-hit:hover, \.rv-hit:focus-visible \{[^}]*background: var\(--broken\); color: var\(--sheet\);/);
});
test('the side sheet hides while the review page shows and comes back with the map', () => {
  assert.match(html, /\.reviewing #sheet, \.reviewing #side-open \{ visibility: hidden; \}/);
  assert.doesNotMatch(html, /#review-body \{[^}]*margin-right/, 'the page takes the width the sheet leaves');
});
test('the view switch is wired to the hash, the header and the selection path without markup', () => {
  const review = fragment('/* ---------- review page', '/* ---------- end review page');
  assert.ok(!review.includes('innerHTML'));
  assert.match(review, /addEventListener\("hashchange", \(\) => showView\(viewOf\(location\.hash\)\)\)/);
  assert.match(script, /function go\(t\) \{ if \(reviewing\) location\.hash = "#map"; select\(t\);/);
  assert.match(script, /initReview\(\);\nshowPanel\(\);/);
  assert.match(fragment('function overview()', '/* ---------- target panel'), /findingSections\(panel\);/);
  assert.match(html, /<div class="seg glass capsule" role="radiogroup" aria-label="View" id="views">/);
  assert.match(html, /<main id="review-page" aria-label="Review">/);
  assert.doesNotThrow(() => new vm.Script(script));
});
