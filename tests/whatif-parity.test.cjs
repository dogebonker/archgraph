'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const preview = require('../assets/whatif.js');
const fixtures = require('./fixtures/whatif-parity.json');
const plain = value => JSON.parse(JSON.stringify(value));
for (const fixture of fixtures) {
  test(`Rust preview parity: ${fixture.name}`, () => {
    const original = JSON.stringify(fixture.document);
    const result = preview.simulate(fixture.document, fixture.move);
    assert.equal(JSON.stringify(fixture.document), original, 'preview must not mutate embedded data');
    if (fixture.unsupported) {
      assert.equal(result.partial, true);
      assert.ok(result.warnings.some(w => /grab.bag/i.test(w)));
    } else {
      assert.deepEqual(plain(result.before), fixture.before);
      assert.deepEqual(plain(result.after), fixture.after);
      assert.deepEqual(result.warnings, []);
      assert.equal(result.partial, false);
    }
  });
}
test('browser export has the same pure API without DOM or module globals', () => {
  const sandbox = {};
  vm.runInNewContext(fs.readFileSync(path.join(__dirname,'../assets/whatif.js'),'utf8'), sandbox);
  assert.equal(typeof sandbox.ArchgraphPreview.simulate,'function');
  assert.equal(typeof sandbox.ArchgraphPreview.analyze,'function');
});
test('reject unknown, duplicate, no-op and unsafe destinations', () => {
  const doc=fixtures[0].document, file=doc.files[0].path;
  for(const to of [file,doc.files[1].path,'/tmp/x','../x','x/../y','x//y','./x','x\\y','C:/x','x\nq','x\u0000q','x\u0085q',''])
    assert.throws(()=>preview.simulate(doc,{file,to}),undefined,to);
  assert.throws(()=>preview.simulate(doc,{file:'missing.ts',to:'new/file.ts'}),/Unknown/);
  // Like the CLI, a move list checks destinations against the scan, even ones an earlier move vacates.
  const [a,b]=doc.files.map(f=>f.path);
  assert.throws(()=>preview.simulate(doc,[{file:a,to:'new/x.ts'},{file:b,to:a}]),/already exists in the scan/);
  assert.throws(()=>preview.simulate(doc,[{file:a,to:'new/x.ts'},{file:b,to:'new/x.ts'}]),/Two files/);
});
test('legacy, unknown schema and unsupported glob rules explicitly mark partial', () => {
  for(const schema of ['archgraph/v1','archgraph/v100']) {
    const doc={...fixtures[0].document,schema};
    assert.equal(preview.analyze(doc).partial,true);
  }
  const doc=structuredClone(fixtures[0].document);
  doc.config.rules={doc:{version:1,forbid:[{from:'a/**oops',to:'b/**'}]}};
  assert.equal(preview.analyze(doc).partial,true);
  assert.match(preview.analyze(doc).warnings.join(' '),/glob/);
});
test('moving a file in a file-granularity SCC does not invent a new cycle',()=>{
  const fixture=fixtures.find(f=>f.name==='file-granularity');
  const doc=structuredClone(fixture.document);
  doc.edges=[{from:doc.files[0].path,to:doc.files[1].path,kind:'runtime'},{from:doc.files[1].path,to:doc.files[0].path,kind:'runtime'}];
  assert.equal(preview.simulate(doc,fixture.move).new_cycles.length,0);
});
test('large chains and cycles do not overflow the JavaScript call stack',()=>{
  const count=12000, files=Array.from({length:count},(_,i)=>({path:`m${i}/file.ts`,barrel:false}));
  const edges=files.slice(1).map((f,i)=>({from:files[i].path,to:f.path,kind:'runtime'}));
  const doc={schema:'archgraph/v1.1',source:{language:'typescript'},config:{depth:1,source_root:'.'},files,edges};
  assert.equal(preview.analyze(doc).cycles.length,0);
  edges.push({from:files.at(-1).path,to:files[0].path,kind:'runtime'});
  assert.equal(preview.analyze(doc).largest_cycle,count);
});
test('braces, glob classes and question marks match supported layout globs',()=>{
  const doc=structuredClone(fixtures[0].document);
  doc.config.rules={doc:{version:1,forbid:[{from:'{a,c}/[ot]?e.ts',to:'b/**'}]}};
  assert.equal(preview.analyze(doc).forbidden,1);
  // A capture may sit inside brace alternation, as an ADR's layered layout writes it.
  assert.equal(preview.bind('{a/<n>/h*.go,b/<n>/h/*.go}','b/x/h/m.go'),'x');
});

test('grab-bag fan-in threshold uses rounded Rust metrics',()=>{
  const files=[{path:'utils/a.ts',barrel:false},{path:'utils/b.ts',barrel:false},{path:'app/a.ts',barrel:false}];
  const edges=[{from:'app/a.ts',to:'utils/a.ts',kind:'runtime'},{from:'utils/b.ts',to:'app/a.ts',kind:'runtime'}];
  for(let i=0;i<50;i++){files.push({path:`x${i}/a.ts`,barrel:false});edges.push({from:`x${i}/a.ts`,to:'utils/a.ts',kind:'type'});}
  const doc={schema:'archgraph/v1.1',source:{language:'typescript'},config:{depth:1,source_root:'.',rules:{doc:{version:1,grabbag:{fan_in_min:6}}}},files,edges};
  const result=preview.analyze(doc);
  assert.equal(result.modules.utils.weighted_ca,6);
  assert.equal(result.partial,true);
  assert.ok(result.warnings.some(w=>w.includes('Grab-bag refinement required for utils')));
});

test('external dependencies and package/category rules explicitly limit previews', () => {
  for (const patch of [
    { external_nodes: [{ id: 'external:typescript:typeorm', external: true, package: 'typeorm' }] },
    { external_edges: [{ from: 'a/one.ts', to: 'external:typescript:typeorm', kind: 'runtime' }] },
    { external_exposure: { modules: [] } },
    { config: { ...fixtures[0].document.config, rules: { doc: { version: 1, forbid: [{ from: '**', to: 'pkg:typeorm' }] } } } },
    { config: { ...fixtures[0].document.config, rules: { doc: { version: 1, allow: [{ from: '**', to: 'category:orm' }] } } } },
  ]) {
    const doc = { ...structuredClone(fixtures[0].document), ...patch };
    const result = preview.simulate(doc, fixtures[0].move);
    assert.equal(result.partial, true);
    assert.ok(result.warnings.some(warning => /Internal-only preview: external dependency policy verdicts and exposure/.test(warning)));
  }
});

test('pkg-category and external_exposure layout fields keep internal rule verdicts', () => {
  const fixture = fixtures.find(f => f.name === 'forbid-before-allow-kind-filter'), doc = structuredClone(fixture.document);
  Object.assign(doc.config.rules.doc, { 'pkg-category': { orm: ['typeorm'] }, external_exposure: { max_percent: 70 } });
  const result = preview.analyze(doc);
  assert.equal(result.forbidden, fixture.before.forbidden);
  assert.ok(!result.warnings.some(w => /Rules preview incomplete/.test(w)), result.warnings.join(' '));
});

test('a move that clears one kind of a forbidden pair reports that import as fixed', () => {
  const doc = { schema: 'archgraph/v1.1', source: { language: 'typescript' },
    config: { depth: 1, source_root: '.', rules: { doc: { version: 1, forbid: [
      { id: 'no-store-types', from: 'app/**', to: 'store/**', kinds: ['type'] },
      { id: 'no-runtime-b', from: 'app/**', to: '**/b.ts', kinds: ['runtime'] }] } } },
    files: [{ path: 'app/a.ts', barrel: false }, { path: 'store/b.ts', barrel: false }],
    edges: [{ from: 'app/a.ts', to: 'store/b.ts', kind: 'runtime' }, { from: 'app/a.ts', to: 'store/b.ts', kind: 'type' }] };
  const result = preview.simulate(doc, { file: 'store/b.ts', to: 'lib/b.ts' });
  assert.deepEqual(result.removed_forbidden.map(v => [v.kind, v.rule]), [['type', 'no-store-types']]);
  assert.deepEqual(result.new_forbidden, []);
});
