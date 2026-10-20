/* Import-level preview. Rust's simulate command remains authoritative. */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.ArchgraphPreview = api;
})(typeof globalThis === 'object' ? globalThis : this, function () {
  'use strict';
  const cmp = (a, b) => a < b ? -1 : a > b ? 1 : 0;
  const sorted = xs => [...xs].sort(cmp);
  const key = x => JSON.stringify(x);
  const round = n => Math.round(n * 10000) / 10000;
  const ratio = (a, b) => b > 0 ? round(a / b) : null;
  const kind = e => ({static:'runtime', dynamic:'lazy', 'type-reexport':'type'}[e.kind] || e.kind || 'runtime');
  const tests = p => /(?:^|\/)(?:tests|test|__tests__)\//.test(p) || /(?:_test\.go|_test\.exs|(?:^|\/)test_helper\.exs|\.(?:test|spec)\.[^/]+|(?:^|\/)(?:test_[^/]*\.py|conftest\.py)|_test\.py|Tests?\.java)$/.test(p);
  function glob(raw) {
    if (typeof raw !== 'string' || /[\\\x00-\x1f\x7f]/.test(raw)) throw Error('unsupported glob: ' + raw);
    let out = '', braces = 0;
    for (let i = 0; i < raw.length; i++) {
      const c = raw[i];
      if (c === '*') {
        if (raw[i+1] === '*') {
          if ((i > 0 && raw[i-1] !== '/') || (i+2 < raw.length && raw[i+2] !== '/')) throw Error('unsupported globstar: ' + raw);
          i++;
          if (raw[i+1] === '/') { out += '(?:.*/)?'; i++; } else out += '.*';
        } else out += '[^/]*';
      } else if (c === '?') out += '[^/]';
      else if (c === '{') { if (braces++) throw Error('unsupported nested glob braces'); out += '(?:'; }
      else if (c === '}') { if (!braces--) throw Error('unbalanced glob braces'); out += ')'; }
      else if (c === ',' && braces) out += '|';
      else if (c === '[') {
        const end = raw.indexOf(']', i+1);
        if (end < 0) throw Error('unbalanced glob class');
        let cls = raw.slice(i+1,end);
        if (!cls || cls.includes('/') || cls.includes('[')) throw Error('unsupported glob class');
        if (cls[0] === '!' || cls[0] === '^') cls = '^' + cls.slice(1);
        out += '['+cls+']'; i = end;
      } else out += c.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    }
    if (braces) throw Error('unbalanced glob braces');
    return new RegExp('^(?:'+out+')$');
  }
  function pattern(raw) {
    const tokens = [...raw.matchAll(/<[^>]*>/g)].map(m => m[0]);
    const unique = [...new Set(tokens)];
    if (unique.length > 1 || raw.split('/').some(s => /[<>]/.test(s) && !/^<[\p{L}\p{N}_]+>$/u.test(s))) throw Error('unsupported capture: '+raw);
    const token = unique[0];
    const any = glob(token ? raw.split(token).join('*') : raw);
    const fragments = token && raw.split(token).map(p => glob(p).source.slice(4,-2));
    // The reachability search tests the same rule target against many paths: compile it once.
    const compiled = new Map();
    return {
      raw, token,
      bind(path) {
        if (!any.test(path)) return undefined;
        if (!token) return null;
        // Bind literal path segments, including characters with glob meaning.
        for (const value of path.split('/')) {
          const literal = value.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
          if (new RegExp('^(?:'+fragments.join(literal)+')$').test(path)) return value;
        }
        return undefined;
      },
      instance(text, value) { return token && value !== null && value !== undefined ? text.split(token).join(value) : text; },
      matches(text, value, path) {
        const bound = token && value !== null && value !== undefined, id = bound ? text+'\0'+value : text;
        let re = compiled.get(id);
        if (!re) {
          if ([...text.matchAll(/<[^>]*>/g)].some(m=>m[0]!==token)) throw Error('unbound capture: '+text);
          const literal = bound && value.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
          re = bound ? new RegExp('^(?:'+text.split(token).map(p => glob(p).source.slice(4,-2)).join(literal)+')$') : glob(text);
          compiled.set(id, re);
        }
        return re.test(path);
      }
    };
  }
  function folder(path, config, language) {
    const root = config.source_root || '.';
    const outside = root !== '.' && !path.startsWith(root+'/');
    const rel = root === '.' || outside ? path : path.slice(root.length+1);
    const segments = rel.split('/');
    let id;
    if (segments.length === 1) id = ['go','java'].includes(language) ? '' : rel;
    else id = segments.slice(0,-1).slice(0,config.package_modules ? Infinity : (config.depth ?? 1)).join('/');
    return outside ? (id ? '../'+id : '..') : (id || '.');
  }
  function scc(pairs) {
    const adjacency = new Map();
    for (const [a,b] of pairs) { if (!adjacency.has(a)) adjacency.set(a,[]); if (!adjacency.has(b)) adjacency.set(b,[]); adjacency.get(a).push(b); }
    // Iterative Kosaraju avoids the browser call-stack limit on real repositories.
    const seen = new Set(), order = [], reverse = new Map([...adjacency.keys()].map(n=>[n,[]]));
    for (const [a,bs] of adjacency) for (const b of bs) reverse.get(b).push(a);
    for (const start of sorted(adjacency.keys())) {
      if (seen.has(start)) continue;
      seen.add(start); const stack = [[start,0]];
      while (stack.length) { const frame = stack[stack.length-1], next = adjacency.get(frame[0]);
        if (frame[1] < next.length) { const n = next[frame[1]++]; if (!seen.has(n)) { seen.add(n); stack.push([n,0]); } }
        else { order.push(frame[0]); stack.pop(); }
      }
    }
    seen.clear(); const result = [];
    for (const start of order.reverse()) {
      if (seen.has(start)) continue;
      const members = [], stack = [start]; seen.add(start);
      while (stack.length) { const n = stack.pop(); members.push(n); for (const p of reverse.get(n)) if (!seen.has(p)) { seen.add(p); stack.push(p); } }
      if (members.length > 1) result.push(sorted(members));
    }
    return result.sort((a,b)=>cmp(key(a),key(b)));
  }
  function compute(doc, moved) {
    const config = doc.config || {}, language = doc.source?.language, rules = config.rules?.doc;
    const warnings = [], files = doc.files || [], rename = p => moved?.get(p) ?? p;
    const edges = (doc.edges || []).map(e=>({...e,from:rename(e.from),to:rename(e.to),original_to:e.original_to ? rename(e.original_to) : undefined}));
    const nodes = new Map(files.map(f=>[rename(f.path),{...f,path:rename(f.path)}]));
    if ((doc.external_nodes || []).length || (doc.external_edges || []).length || doc.external_exposure ||
      [...(rules?.forbid || []), ...(rules?.allow || [])].some(r => /^(?:pkg|category):/.test(r.to || ""))) {
      warnings.push('Internal-only preview: external dependency policy verdicts and exposure are not recomputed. Run archgraph simulate for authoritative results.');
    }
    let moduleRules = [], compiled = null;
    try {
      if (rules) {
        if (rules.version !== 1) throw Error('unsupported layout version');
        const known = ['version','exclude','include','edge_weights','pkg-category','external_exposure','grabbag','modules','layers','public','entrypoints','forbid','allow','default'];
        if (Object.keys(rules).some(k=>!known.includes(k))) throw Error('unknown layout fields');
        const checkFields = (rows, known) => { if (rows.some(row=>Object.keys(row).some(k=>!known.includes(k)))) throw Error('unknown layout rule fields'); };
        checkFields(rules.modules || [],['id','path']);
        checkFields(rules.public || [],['module','entries','kinds']);
        checkFields(rules.forbid || [],['id','from','to','reachable','reason','kinds']);
        checkFields(rules.allow || [],['id','from','to','reason','kinds']);
        for (const row of [...(rules.forbid||[]),...(rules.allow||[]),...(rules.public||[])]) if (row.kinds?.some(k=>!['runtime','static','lazy','dynamic','type','type-reexport','reexport'].includes(k))) throw Error('unknown rule edge kind');
        moduleRules = (rules.modules || []).map(m=>({p:pattern(m.path),id:m.id}));
        const pairs = (xs,name)=>(xs || []).map((r,i)=>({...r,p:pattern(r.from),id:r.id ?? name+'['+i+']'}));
        compiled = {forbid:pairs(rules.forbid,'forbid'),allow:pairs(rules.allow,'allow'),
          entrypoints:Object.entries(rules.entrypoints || {}).sort(([a],[b])=>cmp(a,b)).map(([p,e])=>({p:pattern(p),entry:e})),
          public:(rules.public || []).map(r=>({...r,p:pattern(r.module)})),layers:(rules.layers || []).map(glob)};
        for (const r of [...compiled.forbid,...compiled.allow]) { r.p.matches(r.to,'x',''); }
        for (const r of compiled.public) for (const e of r.entries) r.p.matches(e,'x','');
      }
    } catch (error) { warnings.push('Rules preview incomplete: '+error.message); compiled = null; moduleRules = []; }
    const policy = path => {
      for (const r of moduleRules) { const v = r.p.bind(path); if (v !== undefined) return r.p.instance(r.id,v); }
      return folder(path,config,language);
    };
    const modulesOf = new Map(), policyOf = new Map(), testFiles = new Set(), barrels = new Set();
    const legacy = doc.schema === 'archgraph/v1' || config.legacy_module_classes;
    if (legacy) warnings.push('Legacy module boundaries may differ; verify with archgraph simulate.');
    for (const [path,f] of nodes) {
      const isTest = f.class === 'tests' || tests(path);
      if (isTest) testFiles.add(path);
      const base = config.granularity === 'file' && !legacy ? path : policy(path);
      modulesOf.set(path, !legacy && isTest ? 'tests::'+base : base); policyOf.set(path,policy(path));
      if (f.barrel === true) barrels.add(path);
      else if (!Object.hasOwn(f,'barrel') && /(?:^|\/)(?:index\.(?:ts|tsx|js|jsx|mjs|cjs)|__init__\.py)$/.test(path)) warnings.push('Missing barrel metadata: '+path);
    }
    for (const e of edges) if (e.original_to && !policyOf.has(e.original_to)) policyOf.set(e.original_to,policy(e.original_to));
    if (!['archgraph/v1','archgraph/v1.1','archgraph/v1.2'].includes(doc.schema)) warnings.push('Unknown graph schema; preview may be incomplete.');
    const includes = (r,e) => !r.kinds || r.kinds.some(k=>kind({kind:k}) === kind(e));
    const adjacency = new Map();
    for (const e of edges) { if (!adjacency.has(e.from)) adjacency.set(e.from,[]); adjacency.get(e.from).push(e); }
    const verdict = e => {
      if (!rules || !compiled) return {from:e.from,to:e.to,verdict:null,rule:null,kind:kind(e)};
      const written = e.original_to || e.to;
      const answer = (v,r)=>({from:e.from,to:e.to,verdict:v,rule:r,kind:kind(e)});
      for (const r of compiled.forbid) {
        if (!includes(r,e)) continue; const v = r.p.bind(e.from); if (v === undefined) continue;
        if (r.p.matches(r.to,v,written)) return answer('forbidden',r.id);
        if (r.reachable) { const queue = [e.to], seen = new Set(queue);
          for (let i=0;i<queue.length;i++) { const f=queue[i]; if (r.p.matches(r.to,v,f)) return answer('forbidden',r.id);
            for (const next of adjacency.get(f) || []) if (includes(r,next) && !seen.has(next.to)) {seen.add(next.to);queue.push(next.to);} }
        }
      }
      for (const r of compiled.entrypoints) { const v=r.p.bind(written); if (v!==undefined && !r.p.matches(r.p.raw,v,e.from) && written!==r.p.instance(r.entry,v)) return answer('forbidden','entrypoints'); }
      for (const r of compiled.allow) { const v=r.p.bind(e.from); if (includes(r,e) && v!==undefined && r.p.matches(r.to,v,written)) return answer('allowed',r.id); }
      const entry = (r,v)=>r.entries.some(x=>r.p.matches(x,v,written));
      for (const r of compiled.public) { const v=r.p.bind(written); if (includes(r,e) && v!==undefined && !r.p.matches(r.p.raw,v,e.from) && !entry(r,v)) return answer('forbidden','public'); }
      const a=compiled.layers.findIndex(r=>r.test(e.from)), b=compiled.layers.findIndex(r=>r.test(written));
      if (a>=0 && b>=0) return answer(b<=a ? 'allowed':'forbidden','layers');
      if (policyOf.get(e.from)===policyOf.get(written)) return answer('allowed','same-module');
      for (const r of compiled.entrypoints) { const v=r.p.bind(written); if (v!==undefined && written===r.p.instance(r.entry,v)) return answer('allowed','entrypoints'); }
      for (const r of compiled.public) { const v=r.p.bind(written); if (includes(r,e) && v!==undefined && entry(r,v)) return answer('allowed','public'); }
      return answer(rules.default==='forbidden'?'forbidden':'unplanned',rules.default==='forbidden'?'default':'none');
    };
    const verdicts = edges.map(verdict).sort((a,b)=>cmp(key(a),key(b)));
    const production = edges.filter(e=>!testFiles.has(e.from)&&!testFiles.has(e.to));
    const selected = production.filter(e=>config.all_kinds || kind(e)==='runtime');
    const modulePairs = selected.filter(e=>modulesOf.get(e.from)!==modulesOf.get(e.to)).map(e=>[modulesOf.get(e.from),modulesOf.get(e.to)]);
    const dir = p=>p.includes('/')?p.slice(0,p.lastIndexOf('/')):'';
    const filePairs = selected.filter(e=>e.from!==e.to && modulesOf.get(e.from)===modulesOf.get(e.to) && (language!=='go'||dir(e.from)!==dir(e.to))).map(e=>[e.from,e.to]);
    const cycles = [...scc(filePairs).map(members=>({level:'file',members})),...scc(modulePairs).map(members=>({level:'module',members}))];
    const runtimePairs = new Set(selected.filter(e=>modulesOf.get(e.from)!==modulesOf.get(e.to)).map(e=>key([modulesOf.get(e.from),modulesOf.get(e.to)])));
    const twoWay = [...runtimePairs].map(JSON.parse).filter(([a,b])=>cmp(a,b)<0&&runtimePairs.has(key([b,a]))).sort((a,b)=>cmp(key(a),key(b)));
    const weights = {runtime:1,lazy:.3,type:.1,reexport:0,...rules?.edge_weights};
    if (Object.values(weights).some(w=>!Number.isFinite(w)||w<0)) throw Error('Edge weights must be finite and non-negative.');
    if (edges.some(e=>!['runtime','lazy','type','reexport'].includes(kind(e)))) warnings.push('Unknown edge kind; verify cycle and metric results with archgraph simulate.');
    const targets = new Map();
    for (const e of edges) if (barrels.has(e.from)) { if (!targets.has(e.from)) targets.set(e.from,[]); targets.get(e.from).push(e.to); }
    const pairs = new Map();
    for (const e of edges) if (!barrels.has(e.from)) {
      const stack=[e.to], seen=new Set();
      while (stack.length) { const target=stack.pop(); if (seen.has(target)) continue;seen.add(target);
        if (targets.has(target)) stack.push(...targets.get(target));
        else if (target!==e.from && nodes.has(target)) { const k=key([e.from,target]);pairs.set(k,Math.max(pairs.get(k)||0,weights[kind(e)]??1)); }
      }
    }
    const stats = new Map();
    for (const id of modulesOf.values()) { if (!stats.has(id)) stats.set(id,{ca:new Set(),ce:new Set(),internal:0,outgoing:0,wi:0,wo:0,files:0});stats.get(id).files++; }
    const neighbours = new Map();
    for (const [pair,w] of pairs) { const [a,b]=JSON.parse(pair), from=modulesOf.get(a),to=modulesOf.get(b),s=stats.get(from);
      if (from===to) {s.internal++;s.wi+=w;} else {s.ce.add(to);stats.get(to).ca.add(from);s.outgoing++;s.wo+=w;const k=key([from,to]);neighbours.set(k,Math.max(neighbours.get(k)||0,w));} }
    const weighted = new Map([...stats.keys()].map(id=>[id,{ca:0,ce:0}]));
    for (const [pair,w] of neighbours) {const [a,b]=JSON.parse(pair);weighted.get(a).ce+=w;weighted.get(b).ca+=w;}
    const modules=Object.create(null), thresholds={cohesion_max:.2,fan_in_min:5,internal_edges_per_file_max:.5,...rules?.grabbag};
    for (const id of sorted(stats.keys())) {const s=stats.get(id),w=weighted.get(id);modules[id]={ca:s.ca.size,ce:s.ce.size,weighted_ca:round(w.ca),weighted_ce:round(w.ce),instability:ratio(w.ce,w.ca+w.ce),cohesion:ratio(s.wi,s.wi+s.wo)};
      if (config.granularity!=='file' && !id.startsWith('tests::') && s.files>=2 && modules[id].cohesion!==null && modules[id].cohesion<thresholds.cohesion_max && modules[id].weighted_ca>=thresholds.fan_in_min && s.internal/s.files<thresholds.internal_edges_per_file_max) warnings.push('Grab-bag refinement required for '+id+'; raw cycle preview only.');
    }
    return {moduleOf:modulesOf,projection:{cycles,two_way_pairs:twoWay,largest_cycle:Math.max(0,...cycles.filter(c=>c.level==='module').map(c=>c.members.length)),modules,forbidden:verdicts.filter(v=>v.verdict==='forbidden').length,unplanned:verdicts.filter(v=>v.verdict==='unplanned').length,verdicts},warnings};
  }
  function analyze(doc) { const result=compute(doc);return {...result.projection,warnings:sorted(new Set(result.warnings)),partial:result.warnings.length>0}; }
  /* One move, or a list applied together like `archgraph simulate --move A B --move C D`. */
  function simulate(doc, moves) {
    const valid=p=>typeof p==='string' && p.length>0 && !/[\\:\p{Cc}]/u.test(p) && !p.startsWith('/') && p.split('/').every(s=>s!==''&&s!=='.'&&s!=='..');
    moves=Array.isArray(moves)?moves:[moves];
    if (!moves.length) throw Error('No move to preview.');
    const renamed=new Map(),original=new Map();
    for (const move of moves) {
      if (!move || !valid(move.file) || !valid(move.to)) throw Error('Move paths must be relative paths inside the scan root.');
      if (!doc.files.some(f=>f.path===move.file)) throw Error('Unknown source file: '+move.file);
      if (doc.files.some(f=>f.path===move.to)) throw Error('Destination already exists in the scan: '+move.to);
      if (renamed.has(move.file)) throw Error(move.file+' is moved twice');
      if (original.has(move.to)) throw Error('Two files are moved to '+move.to);
      renamed.set(move.file,move.to); original.set(move.to,move.file);
    }
    const before=compute(doc),after=compute(doc,renamed),normalize=p=>original.get(p)??p;
    const identity=c=>key([c.level,c.members.map(m=>c.level==='file'||doc.config?.granularity==='file'?normalize(m):m).sort(cmp)]);
    const afterKeys=new Set(after.projection.cycles.map(identity));
    const covered = cycle => { const [level,members]=JSON.parse(identity(cycle)); return before.projection.cycles.some(old=>{ const [oldLevel,oldMembers]=JSON.parse(identity(old));return level===oldLevel && members.every(member=>oldMembers.includes(member)); }); }; 
    const was=before.projection.verdicts.filter(v=>v.verdict==='forbidden'),now=after.projection.verdicts.filter(v=>v.verdict==='forbidden');
    const pair=(v,norm)=>key([norm(v.from),norm(v.to),v.kind]);
    const wasKeys=new Set(was.map(v=>pair(v,p=>p))),nowKeys=new Set(now.map(v=>pair(v,normalize)));
    const warnings=sorted(new Set([...before.warnings,...after.warnings]));
    const moved=moves.map(m=>({file:m.file,to:m.to,module:after.moduleOf.get(m.to)}));
    return {move:moved[0],moves:moved,before:before.projection,after:after.projection,
      new_cycles:after.projection.cycles.filter(c=>!covered(c)),removed_cycles:before.projection.cycles.filter(c=>!afterKeys.has(identity(c))),
      new_forbidden:now.filter(v=>!wasKeys.has(pair(v,normalize))),removed_forbidden:was.filter(v=>!nowKeys.has(pair(v,p=>p))),warnings,partial:warnings.length>0};
  }
  /* What a layout glob binds for a path: undefined when it does not match, null when it has no capture, else the captured segment. */
  const bind = (raw, path) => pattern(raw).bind(path);
  return {analyze,simulate,bind};
});
