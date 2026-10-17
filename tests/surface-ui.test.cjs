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
    children: [], listeners: {}, hidden: true,
    setAttribute() {},
    addEventListener(event, fn) { this.listeners[event] = fn; },
    append(...children) { this.children.push(...children); },
    set textContent(value) { this.text = String(value); this.children = []; },
    get textContent() { return (this.text || '') + this.children.map(c => c.textContent).join(''); },
    set innerHTML(_) { throw new Error('surface text must not become markup'); },
  };
}
function context(extra = {}) {
  const ctx = vm.createContext({ document: { createElement: node, createTextNode: text => ({ textContent: text }) }, ...extra });
  vm.runInContext(fragment('/* Every piece of document text', '/* ---------- header'), ctx);
  return ctx;
}
test('ring width grows logarithmically, stays bounded, and preserves selection thickness', () => {
  const ctx = context();
  const width = (n, active = false) => { ctx.n = n; ctx.active = active; return vm.runInContext('surfaceRingWidth(n, active)', ctx); };
  assert.equal(width(0), 1.25);
  assert.equal(width(0, true), 2);
  assert.equal(width(NaN), 1.25);
  assert.equal(width(-2), 1.25);
  assert.equal(width(Infinity), 1.25);
  assert.ok(width(10) > width(1));
  assert.ok(width(100) - width(10) < width(10) - width(0));
  assert.equal(width(Number.MAX_VALUE), 6);
  assert.equal(width(Number.MAX_VALUE, true), 6);
  assert.match(script, /surfaceByModule\.get\(mods\[i\]\.id\)/);
  assert.match(script, /ctx\.lineWidth = surfaceRingWidth\(surface\?\.exported\.length \|\| 0, chosen \|\| s >= S_IN\) \* px/);
});
test('surface legend stays absent for old documents and explains incomplete counts', () => {
  for (const doc of [{}, { surface: { complete: true } }, { surface: { complete: false } }]) {
    const legend = node(), ctx = context({ doc, $: () => legend });
    vm.runInContext(fragment('/* ---------- public surface legend', '/* ---------- end public surface legend'), ctx);
    assert.equal(legend.hidden, !doc.surface);
    if (doc.surface) {
      assert.match(legend.textContent, /Thicker module rings expose more symbols/);
      assert.equal(legend.textContent.includes('partial or conservative estimates'), !doc.surface.complete);
    }
  }
});
test('surface panel escapes symbols and suggestions, links files, and explains uncertainty', () => {
  const panel = node(), targets = [], evil = '<img src=x onerror=alert(1)>';
  const row = { entrypoints: ['src/index.ts'], exported: [{ name: evil }], used_externally: [],
    could_be_internal: [{ name: evil, file: 'src/private.ts' }],
    deep_imports: [{ from: 'src/client.ts', to: 'src/private.ts', symbols: [evil], suggestion: evil }], oversized: true };
  const ctx = context({ panel, doc: { surface: { complete: false, max_symbols: 5, limitations: [evil] } },
    fileIndex: new Map([['src/index.ts', 0], ['src/private.ts', 1], ['src/client.ts', 2]]), short: x => x,
    go: target => targets.push(target), row });
  vm.runInContext(fragment('function btn(', 'const icon ='), ctx);
  vm.runInContext(fragment('/* ---------- public surface panel', '/* ---------- end public surface panel'), ctx);
  vm.runInContext('surfaceContext(row)', ctx);
  assert.match(panel.textContent, /1 exported symbol; 0 symbols used externally; 1 deep import/);
  assert.match(panel.textContent, /Could be internal/);
  assert.match(panel.textContent, /Deep imports/);
  assert.ok(panel.textContent.includes(evil));
  assert.match(panel.textContent, /counts are partial or conservative estimates/);
  assert.match(panel.textContent, /more than 5 symbols.*facade/);
  panel.children.find(n => n.textContent.startsWith('Entrypoints:')).children[0].listeners.click();
  assert.equal(targets[0].type, 'file');
  assert.equal(targets[0].i, 0);
});
test('missing surface rows leave module panels unchanged', () => {
  const panel = node(), ctx = context({ panel, doc: {} });
  vm.runInContext(fragment('/* ---------- public surface panel', '/* ---------- end public surface panel'), ctx);
  vm.runInContext('surfaceContext(undefined); surfaceContext({})', ctx);
  assert.equal(panel.children.length, 0);
  assert.match(script, /surfaceContext\(surfaceByModule\.get\(m\.id\)\)/);
  assert.doesNotThrow(() => new vm.Script(script));
});
