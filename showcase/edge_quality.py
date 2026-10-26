#!/usr/bin/env python3
"""Reproduce pinned edge-quality scans without installing into target repositories.

Run each engine with a separate stage name. JSON, collector stdout, commands, logs,
engine hashes and timing are retained outside the repository (default /tmp).
"""
import argparse
import concurrent.futures
import hashlib
import html
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
FIX_STAGES = ['baseline', 'fix1', 'fix2', 'fix3', 'fix4', 'fix5']
ARCHIVE_STAGES = [*FIX_STAGES, 'corrected']


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def git(root, *args):
    return subprocess.check_output(['git', '-c', 'core.fsmonitor=false', '-C', str(root), *args], text=True).strip()


def prepare(entry, work, cache):
    """Only create our own clones; never reset, checkout or clean a cached clone."""
    root = work / 'repos' / entry['name']
    if root.exists():
        if git(root, 'rev-parse', 'HEAD') != entry['commit']:
            raise RuntimeError(f'{root}: unexpected commit; use a new work directory')
        if git(root, 'status', '--porcelain', '--untracked-files=no'):
            raise RuntimeError(f'{root}: tracked files changed; refusing to reuse')
        return root
    root.parent.mkdir(parents=True, exist_ok=True)
    cached = cache / entry['name']
    env = dict(os.environ, GIT_LFS_SKIP_SMUDGE='1', GIT_TERMINAL_PROMPT='0')
    if (cached / '.git').exists():
        git(cached, 'cat-file', '-e', entry['commit'] + '^{commit}')
        subprocess.run(['git', 'clone', '--quiet', '--shared', '--no-checkout', str(cached), str(root)], check=True, env=env)
    else:
        subprocess.run(['git', 'init', '-q', str(root)], check=True)
        subprocess.run(['git', '-C', str(root), 'remote', 'add', 'origin', entry['repo']], check=True)
        subprocess.run(['git', '-C', str(root), 'fetch', '--quiet', '--depth', '1', 'origin', entry['commit']], check=True, env=env)
    subprocess.run(['git', '-C', str(root), '-c', 'advice.detachedHead=false', 'checkout', '--quiet', entry['commit']], check=True, env=env)
    return root


def wrapper(path, program):
    # Capture collector output while preserving bytes, arguments and return status.
    path.write_text('#!' + sys.executable + '\n' + '''import os,pathlib,subprocess,sys,time
r=subprocess.run([''' + repr(str(program)) + ''']+sys.argv[1:],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
p=pathlib.Path(os.environ['BOUNDRAIL_EDGE_CAPTURE'])/str(time.time_ns())
p.with_suffix('.stdout').write_bytes(r.stdout)
p.with_suffix('.stderr').write_bytes(r.stderr)
p.with_suffix('.argv.json').write_text(__import__('json').dumps(sys.argv[1:]))
sys.stdout.buffer.write(r.stdout);sys.stderr.buffer.write(r.stderr);sys.exit(r.returncode)
''')
    path.chmod(0o755)


def stats(doc):
    cycles = [c for c in doc['cycles'] if c['level'] == 'module']
    kinds = {}
    for edge in doc['edges']:
        kind = edge.get('kind', 'runtime')
        kinds[kind] = kinds.get(kind, 0) + 1
    statement_kinds = {}
    for occurrence in doc.get('imports') or []:
        kind = {'static': 'runtime', 'dynamic': 'lazy', 'type-reexport': 'type'}.get(occurrence['kind'], occurrence['kind'])
        statement_kinds[kind] = statement_kinds.get(kind, 0) + 1
    return {
        'import_resolution': doc.get('import_resolution'),
        'import_statement_kinds': statement_kinds if doc.get('imports') is not None else None,
        'external_packages': len(doc.get('external_nodes', [])),
        'external_edges': len(doc.get('external_edges', [])),
        'grabbag_reclassifications': len(doc.get('grabbags', [])),
        'grabbag_complete': sum(not evidence.get('remaining_cycles') for evidence in doc.get('grabbags', [])),
        'files': len(doc['files']), 'modules': len(doc['modules']),
        'edges': len(doc['edges']), 'edge_kinds': kinds,
        'collector': doc['source']['collector'],
        'module_cycles': len(cycles),
        'largest_module_cycle': max((len(c['members']) for c in cycles), default=0),
        'cycle_members': [c['members'] for c in cycles],
        'deferred_module_cycles': sum(c.get('level') == 'module' for c in doc.get('type_only_cycles', [])),
        'nonruntime_module_cycles': sum(c.get('level') == 'module' and not c.get('contains_runtime_cycle', False) for c in doc.get('type_only_cycles', [])),
        'excluded_files': len(doc.get('config', {}).get('excluded_files', [])),
        'barrels': sum(bool(f.get('barrel')) for f in doc['files']),
        'summary': doc['summary'],
    }



def module_cycles(document):
    return [set(c['members']) for c in document['cycles'] if c['level'] == 'module']


def cycle_disposition(members, document):
    """Do not mistake a merged/expanded SCC for a cycle that disappeared."""
    cycles = module_cycles(document)
    if members in cycles:
        return 'unchanged'
    if any(members <= cycle for cycle in cycles):
        return 'persists in a larger cycle'
    overlaps = [cycle & members for cycle in cycles if cycle & members]
    if overlaps:
        return 'shrunk or split (' + ', '.join(str(len(part)) for part in overlaps) + ' original modules remain cyclic)'
    return 'eliminated'


def cycle_edge_changes(members, before, after):
    """Archive evidence about the old SCC, including exclusions and redirected imports."""
    modules = {f['path']: f['module'] for f in before['files']}
    kept = {f['path'] for f in after['files']}
    old = [e for e in before['edges']
           if modules.get(e['from']) in members and modules.get(e['to']) in members]
    canonical_kind = lambda kind: {'static': 'runtime', 'dynamic': 'lazy'}.get(kind, kind)
    new = {(e['from'], e['to'], canonical_kind(e.get('kind'))) for e in after['edges']}
    runtime_after = {(e['from'], e['to']) for e in after['edges']
                     if canonical_kind(e.get('kind')) == 'runtime'}
    endpoints = {(e['from'], e['to']) for e in after['edges']}
    result = {'excluded_files': sorted({p for e in old for p in (e['from'], e['to']) if p not in kept}),
              'removed_or_redirected': [], 'changed_kinds': [], 'runtime_edges_removed': []}
    for edge in old:
        pair = (edge['from'], edge['to'])
        was_runtime = edge.get('kind') in ('static', 'runtime') or (
            before['schema'] == 'boundrail/v1' and edge.get('kind') == 'reexport')
        if was_runtime and pair not in runtime_after:
            result['runtime_edges_removed'].append(edge)
        if pair not in endpoints and all(p in kept for p in pair):
            result['removed_or_redirected'].append(edge)
        elif pair in endpoints and (*pair, canonical_kind(edge.get('kind'))) not in new:
            result['changed_kinds'].append(edge)
    return result



def comparison(work, stage, entries):
    rows = []
    for entry in entries:
        name = entry['name']
        before = json.loads((work / 'baseline' / name / 'graph.json').read_text())
        after = json.loads((work / stage / name / 'graph.json').read_text())
        cycles = []
        for members in module_cycles(before):
            cycles.append({'members': sorted(members), 'disposition': cycle_disposition(members, after),
                           'edge_changes': cycle_edge_changes(members, before, after)})
        rows.append({'name': name, 'before': stats(before), 'after': stats(after), 'cycles': cycles})
    return rows



# Each witness was read at the manifest pin and checked against both archived graphs.
HAND_CHECKS = [
    ('nest', 'packages/common/index.ts', 'file-stream/index.js', 'common/file-stream leaves the common SCC', 'SCC shrink'),
    ('nest', 'packages/common/index.ts', 'module-utils/index.js', 'common/module-utils leaves the common SCC', 'SCC shrink'),
    ('nest', 'packages/common/index.ts', 'pipes/index.js', 'common/pipes leaves the common SCC', 'SCC shrink'),
    ('nest', 'packages/common/index.ts', 'serializer/index.js', 'common/serializer leaves the common SCC', 'SCC shrink'),
    ('nest', 'packages/core/index.ts', 'repl/index.js', 'core/repl leaves the core SCC', 'SCC shrink'),
    ('nest', 'packages/microservices/index.ts', './module/index.js', 'microservices/module leaves its SCC', 'SCC shrink'),
    ('nest', 'packages/websockets/index.ts', 'decorators/index.js', 'websockets/decorators leaves its SCC', 'SCC shrink'),
    ('nest', 'packages/platform-fastify/index.ts', 'adapters/index.js', 'Fastify parent/adapters/decorators SCC disappears', 'SCC eliminated'),
    ('next.js', 'src/client/components/client-boundary-params.browser.ts', "from '../request/params.browser'", 'client/request leaves the 62-module SCC', 'SCC shrink'),
    ('strapi', 'utils/index.ts', '../configuration/get-dirs', 'configuration leaves the five-module SCC', 'SCC shrink'),
    ('strapi', 'utils/ee.ts', "from '../ee'", 'ee leaves the five-module SCC', 'SCC shrink'),
]


def archived_stages(work, entries):
    stages = ARCHIVE_STAGES
    complete = [stage for stage in stages if (work / stage / 'index.json').is_file()]
    if not complete or complete[0] != 'baseline':
        raise RuntimeError('a complete baseline is required')
    documents, metadata = {}, {}
    for stage in complete:
        index = json.loads((work / stage / 'index.json').read_text())
        by_name = {row['fingerprint']['entry']['name']: row for row in index}
        if set(by_name) != {entry['name'] for entry in entries}:
            raise RuntimeError(f'{stage}: report requires every manifest repository')
        if any(row['status'] != 'ok' for row in index):
            raise RuntimeError(f'{stage}: failed scans cannot be published')
        metadata[stage] = by_name
        documents[stage] = {}
        for entry in entries:
            path = work / stage / entry['name'] / 'graph.json'
            if digest(path) != by_name[entry['name']]['graph_sha256']:
                raise RuntimeError(f'{path}: archived graph hash changed')
            document = json.loads(path.read_text())
            if by_name[entry['name']]['fingerprint']['entry'] != entry:
                raise RuntimeError(f'{stage}/{entry["name"]}: manifest provenance changed')
            if document.get('source', {}).get('git', {}).get('commit') != entry.get('commit'):
                raise RuntimeError(f'{stage}/{entry["name"]}: pinned source commit differs')
            documents[stage][entry['name']] = document
    return complete, documents, metadata



def compare_published(work, entries, documents):
    """Validate the optional cached deployed index against this final measured cohort."""
    path = work / 'published-index.json'
    if not path.is_file():
        return None
    published = {row['name']: row for row in json.loads(path.read_text())['repos']}
    semantic_fields = ['config', 'files', 'edges', 'package_entries', 'modules', 'module_edges',
                       'cycles', 'type_only_cycles', 'test_coverage', 'imports', 'import_resolution',
                       'unresolved', 'external_nodes', 'external_edges', 'summary', 'findings',
                       'cochange', 'hotspots', 'symbols', 'surface', 'grabbags']
    full_matches = []
    for entry in entries:
        name, document = entry['name'], documents[entry['name']]
        row, measured = published[name], stats(document)
        if row['commit'] != entry['commit'] or row['status'] != 'ok':
            raise RuntimeError(f'{name}: deployed snapshot has different source provenance')
        actual = {'files': measured['files'], 'modules': measured['modules'], 'imports': measured['edges'],
                  'module_cycles': measured['module_cycles'], 'largest_module_cycle': measured['largest_module_cycle'],
                  'file_cycles': sum(cycle['level'] == 'file' for cycle in document['cycles']),
                  'forbidden': document['summary']['forbidden'], 'import_resolution': measured['import_resolution'],
                  'external_packages': measured['external_packages'],
                  'findings': {kind: sum(finding['kind'] == kind for finding in document['findings'])
                               for kind in {finding['kind'] for finding in document['findings']}}}
        for key, value in actual.items():
            if value != row['stats'].get(key):
                raise RuntimeError(f'{name}: deployed {key} differs from corrected measurement')
        full = work / f'published-{name}.json'
        if full.is_file():
            saved = json.loads(full.read_text())
            if any(saved.get(key) != document.get(key) for key in semantic_fields):
                raise RuntimeError(f'{name}: deployed semantic graph differs from corrected measurement')
            full_matches.append(name)
    return {'sha256': digest(path), 'repositories': len(entries), 'full_semantic_matches': full_matches,
            'engine_commit': json.loads(path.read_text()).get('boundrail', {}).get('commit', 'unavailable')}


def report(work, entries, output):
    stages = FIX_STAGES
    complete, documents, metadata = archived_stages(work, entries)
    if 'fix2' not in complete:
        raise RuntimeError('report requires complete baseline, fix1 and fix2 snapshots')
    latest = complete[-1]
    pending = [stage for stage in stages if stage not in complete]
    lines = ['# Edge-quality validation', '',
             f"Measured stages: {', '.join(complete)}. Pending: {', '.join(pending) or 'none'}.", '',
             'This is an interim report until all five fixes have measured snapshots. README numbers must not be replaced from an incomplete run.' if pending else 'All five fixes have measured snapshots.', '',
             '## Methodology', '',
             'The first 20 entries in `showcase/repos.json` are the published README cohort (11 TypeScript/JavaScript and nine Python repositories). Three supplemental Go repositories are reported separately. Every stage scans the exact manifest commit and the same scanned root and explicit flags; defaults change only with the corresponding fix.', '',
             'The baseline is the frozen pre-change source at `6627a5e22013044516a599e86bede437ba76ad10`. Targets are separate clean checkouts. No target installation, target build scripts, or annotation processors run. The runner captures commands, engine/source hashes, graph hashes, logs, and raw collector stdout/stderr under `<work>/<stage>/<repo>/`; cached source repositories are never checked out or reset.', '',
             '**Counting units:** edges are canonical file-dependency records, not source import statements. One named import can resolve to several defining files and increase the edge count. Runtime/lazy/type/re-export columns in the architecture tables count edges, and legacy static/dynamic names are mapped to runtime/lazy for comparison. The separate resolution table counts source specifier occurrences only when a ledger exists; missing coverage is never inferred from graph edges or the old unresolved counter. External nodes and edges stay outside internal module metrics.', '',
             'The baseline already excluded type-only and dynamic edges from runtime cycles. Fix 1 additionally excludes structural re-exports, so an observed decrease is not automatically a newly detected type-only cycle. Deferred SCCs containing an existing runtime SCC are reported separately from wholly non-runtime SCCs. JavaScript re-exports can still cause module evaluation: these results describe the requested graph policy, not proof that all runtime execution is acyclic.', '',
             '## Engine provenance', '', '| Stage | Source commit | Binary SHA-256 |', '| --- | --- | --- |']
    if latest == 'corrected':
        correction = ['## Python exact-case correction', '',
                      'The original baseline/fix1–fix5 measurements were collected on macOS. A case-insensitive filesystem lookup in the Python resolver treated differently cased names as existing module files and introduced synthetic capitalized paths. The corrected stage requires exact path-component spelling and rescans all 23 pins. Its counts are authoritative for the final report and README. Earlier archives remain unchanged as historical engine measurements; synthetic-path removal must not be credited as architectural improvement.', '',
                      '| Repository | Files: fix5 → corrected | Edges: fix5 → corrected | Removed synthetic paths (examples) |',
                      '| --- | ---: | ---: | --- |']
        for entry in entries:
            before, after = documents['fix5'][entry['name']], documents['corrected'][entry['name']]
            if before['files'] == after['files'] and before['edges'] == after['edges']:
                continue
            kept = {file['path'] for file in after['files']}
            removed = [file['path'] for file in before['files'] if file['path'] not in kept]
            examples = ', '.join('`' + path + '`' for path in removed[:3]) or 'edge-resolution changes only'
            correction.append(f"| {entry['name']} | {len(before['files'])} → {len(after['files'])} | {len(before['edges'])} → {len(after['edges'])} | {examples} |")
        deployed = compare_published(work, entries, documents['corrected'])
        if deployed is not None:
            correction += ['', f"Cross-platform check against the **prior Linux deployment**, source `{deployed['engine_commit']}`: all {deployed['repositories']} corrected repository counts, module/file-cycle counts, findings, import-resolution ledgers and external-package counts exactly match that deployment's cached index (`published-index.json`, SHA-256 `{deployed['sha256']}`). Full semantic JSON also matches the cached published graphs for {', '.join(deployed['full_semantic_matches']) or 'no separately cached repositories'}. This validates platform consistency after the correction, not the status of a new deployment or an architectural improvement claim."]
        # Insert before the engine-provenance heading and keep its table contiguous.
        position = lines.index('## Engine provenance')
        lines[position:position] = correction + ['']
    for stage in complete:
        fingerprints = {(row['fingerprint']['engine_commit'], row['fingerprint']['engine_sha256']) for row in metadata[stage].values()}
        if len(fingerprints) != 1:
            raise RuntimeError(f'{stage}: inconsistent engine provenance')
        commit, binary_hash = fingerprints.pop()
        lines.append(f'| {stage} | `{commit}` | `{binary_hash}` |')
    lines += ['', 'Collector versions are recorded per scan in `metadata.json`. Reproduce a stage with:', '', '```sh', 'python3 -B showcase/edge_quality.py --engine /absolute/frozen-boundrail \\', '  --engine-commit FULL_SOURCE_COMMIT --stage fixN', 'python3 -B showcase/edge_quality.py --report-output showcase/edge-quality-report.md', '```', '', '## Published 20 repositories', '',
              'Arrow cells compare baseline → ' + latest + '. SCC means a strongly connected component with a cycle; largest is its module count. All intermediate stages remain in the cohort totals, provenance and full SCC inventory.', '',
              '| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (' + latest + ') | Barrels (' + latest + ') | Excluded candidate files | Non-runtime / extended-runtime SCCs | Via grab-bag: complete / partial |',
              '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    def table_rows(cohort):
        for entry in cohort:
            name = entry['name']
            snapshots = [stats(documents[stage][name]) for stage in ['baseline', latest]]
            arrow = lambda key: ' → '.join(str(snapshot[key]) for snapshot in snapshots)
            kinds = {}
            for edge in documents[latest][name]['edges']:
                kind = {'static': 'runtime', 'dynamic': 'lazy', 'type-reexport': 'type'}.get(edge['kind'], edge['kind'])
                kinds[kind] = kinds.get(kind, 0) + 1
            counts = ' / '.join(str(kinds.get(kind, 0)) for kind in ['runtime', 'lazy', 'type', 'reexport'])
            excluded = snapshots[-1]['excluded_files'] if 'fix3' in complete else 'pending'
            nonruntime = snapshots[-1]['nonruntime_module_cycles']
            extended = snapshots[-1]['deferred_module_cycles'] - nonruntime
            complete_grabbags = snapshots[-1]['grabbag_complete']
            partial_grabbags = snapshots[-1]['grabbag_reclassifications'] - complete_grabbags
            grabbag_counts = f'{complete_grabbags} / {partial_grabbags}' if 'fix5' in complete else 'pending'
            lines.append(f"| {name} | {arrow('files')} | {arrow('edges')} | {arrow('module_cycles')} | {arrow('largest_module_cycle')} | {counts} | {snapshots[-1]['barrels']} | {excluded} | {nonruntime} / {extended} | {grabbag_counts} |")
    table_rows(entries[:20])
    lines += ['', '### Published cohort totals by measured stage', '',
              '| Stage | Files | Canonical file edges | Module SCCs | Largest module SCC |',
              '| --- | ---: | ---: | ---: | ---: |']
    for stage in complete:
        snapshots = [stats(documents[stage][entry['name']]) for entry in entries[:20]]
        lines.append(f"| {stage} | {sum(snapshot['files'] for snapshot in snapshots)} | {sum(snapshot['edges'] for snapshot in snapshots)} | {sum(snapshot['module_cycles'] for snapshot in snapshots)} | {max(snapshot['largest_module_cycle'] for snapshot in snapshots)} |")
    lines += ['', '## Supplemental Go repositories', '', '| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (' + latest + ') | Barrels (' + latest + ') | Excluded candidate files | Non-runtime / extended-runtime SCCs | Via grab-bag: complete / partial |', '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    table_rows(entries[20:])
    lines += ['', 'The fix3 directory rules exclude real source as well as generated/test artifacts: Next.js `packages/next/src/build/index.ts:1115` declares its framework build function; Django `django/test/testcases.py:1378` declares its public TestCase API; Payload `src/cli/commands/build/` contains CLI implementation. The measured reduction must be described as changed scan scope. Layout include overrides can restore Next.js build files and their cycle participation. Including Django/test restores files but they retain the test class and remain excluded from cycles; production-cycle analysis of test-named APIs requires a future classification override.', '']
    lines += ['', '## Confidence limits on runtime projection', '',
              'Low-confidence zero-weight re-export fallbacks are retained in JSON but excluded from the default runtime projection. They include un-narrowed namespace/star imports, ambiguous exports, and named exports whose metadata is unproven. These are uncertainty categories, **not proven type-only imports**. Resolved-file coverage and absence of a reported cycle do not prove complete runtime coverage; dynamic wiring and these fallbacks can hide dependencies.', '',
              '| Repository | Zero-weight low-confidence re-export edges | Namespace/star | Ambiguous | Unproven named metadata | Other |',
              '| --- | ---: | ---: | ---: | ---: | ---: |']
    fallback_totals = [0, 0, 0, 0]
    for entry in entries:
        counts = [0, 0, 0, 0]
        for edge in documents[latest][entry['name']]['edges']:
            if edge.get('confidence') != 'low' or edge['kind'] != 'reexport' or edge.get('weight') != 0:
                continue
            reason = edge.get('resolution_reason', '')
            category = 2 if 'unproven' in reason and 'metadata' in reason else 1 if 'ambiguous' in reason else 0 if 'namespace' in reason or 'star import' in reason else 3
            counts[category] += 1
        fallback_totals = [left + right for left, right in zip(fallback_totals, counts)]
        lines.append(f"| {entry['name']} | {sum(counts)} | " + ' | '.join(map(str, counts)) + ' |')
    lines.append('| All 23 | ' + str(sum(fallback_totals)) + ' | ' + ' | '.join(map(str, fallback_totals)) + ' |')
    lines += ['', '## Import resolution: source specifier occurrences', '',
              'These counts come only from `import_resolution` and the source-occurrence ledger. They include known external packages and do not use the file-edge denominator. An unavailable ledger stays unavailable; a zero-import ledger has no percentage. Percentages are rounded to one decimal, so 100.0% with a nonzero unresolved count is rounding, not complete coverage. Runtime/lazy/type/re-export columns in this table count source occurrences, unlike the architecture table above.', '',
              '| Repository | Imports | Resolved | Resolved % | Unresolved | Runtime / lazy / type / re-export occurrences | Known external packages |',
              '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for entry in entries:
        document = documents[latest][entry['name']]
        measured = stats(document)
        resolution = measured['import_resolution']
        if resolution is None:
            cells = 'unavailable | unavailable | unavailable | unavailable | unavailable | unavailable'
        else:
            if resolution['imports'] != resolution['resolved'] + resolution['unresolved']:
                raise RuntimeError(f"{entry['name']}: resolution accounting does not add up")
            if document.get('imports') is None or len(document['imports']) != resolution['imports']:
                raise RuntimeError(f"{entry['name']}: occurrence ledger and resolution total disagree")
            percent = resolution.get('resolved_percent')
            percent = 'n/a' if percent is None else f'{percent:.1f}%'
            kinds = measured['import_statement_kinds']
            counts = ' / '.join(str(kinds.get(kind, 0)) for kind in ['runtime', 'lazy', 'type', 'reexport'])
            cells = f"{resolution['imports']} | {resolution['resolved']} | {percent} | {resolution['unresolved']} | {counts} | {measured['external_packages']}"
        lines.append(f"| {entry['name']} | {cells} |")
    dispositions = [cycle_disposition(members, documents[latest][entry['name']])
                    for entry in entries for members in module_cycles(documents['baseline'][entry['name']])]
    unchanged = sum(d == 'unchanged' for d in dispositions)
    shrunk = sum(d.startswith('shrunk or split') for d in dispositions)
    eliminated = sum(d == 'eliminated' for d in dispositions)
    expanded = sum(d == 'persists in a larger cycle' for d in dispositions)
    inventory_text = f'Through {latest}, {unchanged} original SCCs remain exactly unchanged, {shrunk} have shrunk or split, {eliminated} are eliminated, and {expanded} persist in larger SCCs. '
    lines += ['', '## Resolution and exclusions', '',
              ('Grab-bag reclassifications are measured from `grabbags`: complete means no residual module SCC, while partial retains verified residual runtime SCCs. ' if 'fix5' in documents else 'Grab-bag reclassification remains pending fix5. ') + 'Import-statement totals and resolved percentages are shown only for snapshots with the fix4 ledger; older snapshots remain unavailable. Excluded candidate files are the actual length of `config.excluded_files` in the latest snapshot: they include pre-existing defaults and explicit manifest exclusions, not just files newly removed in fix3. Non-runtime SCCs have no runtime cycle; extended-runtime SCCs include a runtime SCC and must not be counted as removed cycles.', '',
              '## Every baseline module SCC', '',
              'This inventory includes all original SCCs, including shrinkage. An old SCC absorbed into a larger SCC still persists; it is not counted as removed. ' + inventory_text + 'Fix 2 expands the surviving Nest common SCC from five to six modules.', '',
              '| Repository / baseline SCC | Original modules | ' + ' | '.join(complete[1:]) + ' | Explanation of changed membership |',
              '| --- | ---: | ' + ' | '.join('---' for _ in complete[1:]) + ' | --- |']
    for entry in entries:
        name = entry['name']
        for number, members in enumerate(module_cycles(documents['baseline'][name]), 1):
            dispositions = [cycle_disposition(members, documents[stage][name]) for stage in complete[1:]]
            explanation = 'Unchanged through measured stages.'
            if cycle_disposition(members, documents['fix1'][name]) != 'unchanged':
                explanation = 'Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering.'
            if name == 'nest' and 'packages/common/file-stream' in members:
                explanation += ' Named barrel resolution at fix2 restores common itself to the surviving SCC (5 → 6); this is expansion, not another disappearance.'
            if 'fix3' in documents and cycle_disposition(members, documents['fix3'][name]) != cycle_disposition(members, documents['fix2'][name]):
                if explanation == 'Unchanged through measured stages.':
                    explanation = ''
                after_paths = {f['path'] for f in documents['fix3'][name]['files']}
                removed = [f['path'] for f in documents['fix2'][name]['files'] if f['module'] in members and f['path'] not in after_paths]
                examples = ', '.join('`' + path + '`' for path in sorted(removed)[:3])
                explanation += f' Fix3 directory exclusions remove {len(removed)} files in this original SCC; examples: {examples}. Next.js build code and Django testing APIs are real source excluded by directory convention; this is a scope change, not proof that those dependencies were erroneous.'
            if 'fix4' in documents and cycle_disposition(members, documents['fix4'][name]) != cycle_disposition(members, documents['fix3'][name]):
                if explanation == 'Unchanged through measured stages.':
                    explanation = ''
                explanation += ' Fix4 changes type-only or resolved runtime links; exact counts and pinned source witnesses are listed in the transition table below.'
            if 'fix5' in documents:
                classifications = [evidence for evidence in documents[latest][name].get('grabbags', []) if set(evidence['members']) == members or set(evidence['members']) <= members]
                if classifications:
                    if explanation == 'Unchanged through measured stages.':
                        explanation = ''
                    explanation += ' Fix5 verifies a cycle via grab-bag; residual real cycles are retained. See the complete/partial evidence table below.'
            if latest == 'corrected' and cycle_disposition(members, documents['corrected'][name]) != cycle_disposition(members, documents['fix5'][name]):
                if explanation == 'Unchanged through measured stages.':
                    explanation = ''
                explanation += ' Exact-case Python resolution correction changes this disposition; this is collector correctness, not an architectural change.'
            label = ', '.join(sorted(members)[:3]) + (', …' if len(members) > 3 else '')
            lines.append(f"| {name} #{number}: `{label}` | {len(members)} | " + ' | '.join(dispositions) + f' | {explanation} |')
    if 'fix4' in documents:
        verify_type_controls(documents['fix4']['nest'])
        lines += ['', '## Fix 4 runtime-link changes behind SCC membership changes', '',
                  'For every original SCC whose disposition changes between fixes 3 and 4, this table groups its old runtime edges by their actual new kind or changed target, plus new runtime links touching its modules. These are edge-level explanations, not reasons guessed from a lower cycle count. Added links can expand SCCs; they are not counted as removed edges. The complete inventory is reproducible with `transition_kinds()` and the archived JSON.', '',
                  '| Original SCC | Fix3 → fix4 disposition | Changed runtime links | Source witnesses |',
                  '| --- | --- | --- | --- |']
        for entry in entries:
            name = entry['name']
            for number, members in enumerate(module_cycles(documents['baseline'][name]), 1):
                before, after = documents['fix3'][name], documents['fix4'][name]
                prior, current = cycle_disposition(members, before), cycle_disposition(members, after)
                if prior == current:
                    continue
                groups = transition_kinds(members, before, after)
                if not groups:
                    raise RuntimeError(f'{name} SCC {number}: changed membership lacks a runtime-edge explanation')
                counts = '; '.join(f'{reason}: {len(edges)}' for reason, edges in sorted(groups.items()))
                witnesses = []
                for reason, edges in sorted(groups.items()):
                    selected = edges[:2]
                    if reason == 'new or redirected runtime link':
                        modules = {file['path']: file['module'] for file in after['files']}
                        inbound = next((edge for edge in edges if modules.get(edge['from']) in members and modules.get(edge['to']) not in members), None)
                        outbound = next((edge for edge in edges if modules.get(edge['from']) not in members and modules.get(edge['to']) in members), None)
                        if inbound is not None and outbound is not None:
                            selected = [inbound, outbound]
                    for edge in selected:
                        line = edge.get('line')
                        location = str(Path(entry['path']) / edge['from'])
                        label = location + (f':{line}' if line is not None else ' (line unavailable)')
                        url = entry['repo'] + '/blob/' + entry['commit'] + '/' + location + (f'#L{line}' if line is not None else '')
                        witnesses.append(f'[{label}]({url}) ({reason}; target `{edge["to"]}`)')
                lines.append(f'| {name} #{number} | {prior} → {current} | {counts} | ' + '<br>'.join(witnesses) + ' |')
    if 'fix4' in documents:
        calibration_stage = 'corrected' if 'corrected' in documents else 'fix4'
        all_modules = [(name, module) for name, document in documents[calibration_stage].items() for module in document['modules']]
        def candidates(cohesion=.2, fanin=5, density=.5, minimum_files=2):
            return [(name, module) for name, module in all_modules if module['files'] >= minimum_files
                    and module['cohesion'] is not None and module['cohesion'] < cohesion
                    and module['weighted_ca'] >= fanin
                    and module['internal_edges'] / module['files'] < density]
        proposed = candidates()
        cyclic = {name: set().union(*module_cycles(document), *(set(evidence['members']) for evidence in document.get('grabbags', [])))
                  for name, document in documents[calibration_stage].items()}
        in_cycles = sum(module['id'] in cyclic[name] for name, module in proposed)
        lines += ['', f'## Grab-bag candidate calibration ({calibration_stage})', '',
                  f'The chosen defaults (cohesion <0.2, weighted fan-in ≥5, raw internal edges/file <0.5, at least two files) select {len(proposed)} candidates across all 23 repositories; {in_cycles} are currently cyclic. This is candidate calibration, not a claim that all are grab-bags. Benign contexts, enums and two-file barrel modules can satisfy the metrics. Reclassification still requires the fix5 split-graph proof.', '',
                  '| One threshold changed | Candidates across 23 repositories |', '| --- | ---: |',
                  f'| Chosen defaults | {len(proposed)} |',
                  f'| Cohesion <0.1 | {len(candidates(cohesion=.1))} |',
                  f'| Weighted fan-in ≥8 | {len(candidates(fanin=8))} |',
                  f'| Weighted fan-in ≥10 | {len(candidates(fanin=10))} |',
                  f'| Raw internal edges/file <0.25 | {len(candidates(density=.25))} |',
                  f'| At least three files | {len(candidates(minimum_files=3))} |']
    if 'fix5' in documents:
        lines += ['', '## Verified grab-bag reclassifications', '',
                  'A candidate metric is not a finding. Each record below is actual split-graph evidence. Complete reclassification leaves no residual module SCC; partial reclassification retains and reports real residual SCCs and their cut witnesses. These categories are not added to the number of removed SCCs.', '',
                  '| Repository | Original modules | Candidate modules | Result | Residual SCC sizes | File witnesses |',
                  '| --- | ---: | --- | --- | --- | --- |']
        for entry in entries:
            document = documents[latest][entry['name']]
            for evidence in document.get('grabbags', []):
                result = 'partial; real residual cycles retained' if evidence['remaining_cycles'] else 'complete; no residual module SCC'
                residual = ', '.join(str(len(members)) for members in evidence['remaining_cycles']) or 'none'
                candidates = '<br>'.join('`' + candidate['module'] + f"` (cohesion {candidate['cohesion']}, weighted fan-in {candidate['fan_in']}, internal/file {candidate['internal_edges_per_file']:.4f})" for candidate in evidence['candidates'])
                by_pair = {(edge['from'], edge['to']): edge for edge in document['edges']}
                witnesses = []
                candidate_files = {file for candidate in evidence['candidates'] for file in candidate['files']}
                links = [link for link in evidence['edges'] if link['from'] in candidate_files or link['to'] in candidate_files]
                if not links:
                    raise RuntimeError(f"{entry['name']}: grab-bag finding has no candidate-file witness")
                for link in links[:3]:
                    edge = by_pair[(link['from'], link['to'])]
                    location = str(Path(entry['path']) / edge['from'])
                    line = edge.get('line')
                    label = location + (f':{line}' if line is not None else ' (line unavailable)')
                    url = entry['repo'] + '/blob/' + entry['commit'] + '/' + location + (f'#L{line}' if line is not None else '')
                    witnesses.append(f'[{label}]({url}) → `{edge["to"]}`')
                lines.append(f"| {entry['name']} | {len(evidence['members'])} | {candidates} | {result} | {residual} | " + '<br>'.join(witnesses) + ' |')
    lines += ['', '## Hand spot-checks', '',
              'Eleven source reads at the exact pins below check one complete SCC disappearance and individual membership removals in shrunken SCCs. They are **not eleven removed SCCs**. For each witness, the baseline edge is a re-export that participated in the old module projection; fix1/fix2 retain the edge as structural and omit it from the runtime projection. No corresponding import was silently erased.', '',
              '| Repository | Source evidence at pinned commit | Checked consequence | Classification |', '| --- | --- | --- | --- |']
    by_name = {entry['name']: entry for entry in entries}
    for name, path, needle, consequence, classification in HAND_CHECKS:
        entry = by_name[name]
        source = work / 'repos' / name / entry['path'] / path
        matches = [(number, line.strip()) for number, line in enumerate(source.read_text().splitlines(), 1) if needle in line]
        if len(matches) != 1:
            raise RuntimeError(f'{source}: hand-check witness changed or ambiguous')
        number, text = matches[0]
        repository_path = str(Path(entry['path']) / path)
        url = entry['repo'] + '/blob/' + entry['commit'] + '/' + repository_path + '#L' + str(number)
        lines.append(f'| {name} | [`{repository_path}:{number}`]({url}) — `{text}` | {consequence} | {classification}; barrel/re-export policy |')
    lines += ['', 'Additional positive controls confirm that named runtime imports remain runtime and explicit type imports remain type after barrel resolution:', '',
              '| Repository | Import source | Defining source | Observed kind / weight |', '| --- | --- | --- | --- |']
    for name, source_path, needle, defining_path, definition_line, expected_kind in [
        ('nest', 'packages/core/application-config.ts', 'import type { GlobalPrefixOptions }', 'packages/common/interfaces/global-prefix-options.interface.ts', 6, 'type'),
        ('excalidraw', 'actions/actionCanvas.tsx', 'import { getNormalizedZoom }', 'scene/normalize.ts', 7, 'runtime'),
        ('django', 'django/conf/urls/__init__.py', 'from django.urls import include', 'django/urls/conf.py', 17, 'runtime'),
        ('typeorm', 'commands/CommandUtils.ts', 'import type { DataSource }', 'data-source/DataSource.ts', 56, 'type'),
    ]:
        entry = by_name[name]
        source = work / 'repos' / name / entry['path'] / source_path
        matching_lines = [number for number, line in enumerate(source.read_text().splitlines(), 1) if needle in line]
        edges = [edge for edge in documents['fix2'][name]['edges'] if edge['from'] == source_path and edge['to'] == defining_path]
        if len(matching_lines) != 1 or len(edges) != 1 or edges[0]['kind'] != expected_kind:
            raise RuntimeError(f'{name}: positive-control evidence changed')
        source_path = str(Path(entry['path']) / source_path)
        defining_path = str(Path(entry['path']) / defining_path)
        base = entry['repo'] + '/blob/' + entry['commit'] + '/'
        lines.append(f"| {name} | [`{source_path}:{matching_lines[0]}`]({base}{source_path}#L{matching_lines[0]}) | [`{defining_path}:{definition_line}`]({base}{defining_path}#L{definition_line}) | {edges[0]['kind']} / {edges[0]['weight']} |")
    lines += ['', 'Fastify return-path check: `packages/platform-fastify/adapters/fastify-adapter.ts:73` and `packages/platform-fastify/decorators/route-config.decorator.ts:2` import parent-folder `constants.js`; the parent `index.ts:8,10` exports children. These are different files grouped into one parent module, explaining the projected loop.', '']
    if 'fix5' in documents:
        for entry in entries:
            for field in ['files', 'edges', 'imports', 'import_resolution', 'unresolved', 'external_nodes', 'external_edges']:
                if documents['fix5'][entry['name']].get(field) != documents['fix4'][entry['name']].get(field):
                    raise RuntimeError(f"{entry['name']}: fix5 unexpectedly changes {field} inputs")
        final = [documents[latest][entry['name']] for entry in entries[:20]]
        complete_reclassifications = sum(stats(document)['grabbag_complete'] for document in final)
        partial_reclassifications = sum(stats(document)['grabbag_reclassifications'] for document in final) - complete_reclassifications
        lines += ['## Final interpretation', '',
                  f'The published cohort has {complete_reclassifications} complete and {partial_reclassifications} partial grab-bag reclassifications. Real residual runtime SCCs remain reported. SCC count can rise when one component splits: Outline changes from one 40-module SCC to components of 11, 22 and five modules. The aggregate component count is not a monotonic quality score.', '',
                  'Through fix4, no additional baseline runtime SCC was fully removed solely by type-only refinement, test exclusions or resolver changes. The complete baseline Fastify SCC disappearance is the structural re-export policy change; shrinkage and the Zulip expansion are listed individually above. Low-confidence exclusions limit runtime-coverage claims.', '',
                  ('The archived fix5 graphs preserve exactly the fix4 input arrays. The corrected final stage changes Python resolution inputs as documented above; removing synthetic paths is a collector correctness fix, not an architectural improvement.' if latest == 'corrected' else 'All 23 final graphs preserve exactly the fix4 files, file edges, import ledgers, unresolved records and external arrays: fix5 changes cycle interpretation, not those inputs.'), '',
                  '## Verification and reproduction', '',
                  'All 23 pinned repositories completed baseline and fixes 1–5, plus any separately listed correction stage. Report generation checks full-cohort coverage, manifest commits, clean scan provenance and archived graph hashes. README generation uses the latest complete verified final cohort, including the corrected stage when present, and checks that each suggested cut is an existing runtime edge.', '',
                  '```sh', 'python3 -B -m unittest discover -s showcase/tests',
                  'python3 -B showcase/edge_quality.py --report-output showcase/edge-quality-report.md',
                  'python3 -B showcase/edge_quality.py --readme-output README.md', '```', '',
                  '## Open questions and next steps', '',
                  '- Broad directory defaults intentionally change scope. Layout include overrides restore Next.js build implementation and its cycles. Including Django/test restores its public testing API files, but they remain test-classified and outside cycles. An explicit classification override for production APIs in test-named folders is future work, not an implemented option. Payload CLI build code is also excluded by convention.',
                  '- Unresolved imports and low-confidence re-export metadata are a visible backlog. Improve the most common missing/alias cases with fixtures; dynamic imports and dependency injection still limit coverage.',
                  '- The checked Nest interface-only import is now type, while mixed imports retain runtime bindings. This verifies the concrete counterexamples; a lightweight tokenizer is not a complete TypeScript type checker.',
                  '- Candidate thresholds alone do not prove a grab-bag. Keep split-graph verification and real residual-cycle witnesses when extending calibration to other repositories.', '']
    else:
        lines += ['## Remaining validation', '',
                  '- Complete the remaining measured stages and regenerate the final report and README.',
                  '- Keep unresolved coverage and uncertain type/barrel metadata explicit; never infer a resolution denominator from graph edges.', '']
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text('\n'.join(lines))




def transition_kinds(members, before, after):
    """Explain actual runtime-link changes without inferring reasons from net SCC counts."""
    removed = cycle_edge_changes(members, before, after)['runtime_edges_removed']
    by_pair = {}
    for edge in after['edges']:
        by_pair.setdefault((edge['from'], edge['to']), set()).add(edge['kind'])
    groups = {}
    for edge in removed:
        kinds = by_pair.get((edge['from'], edge['to']), set())
        reason = 'type-only' if kinds and kinds <= {'type', 'type-reexport'} else (
            'lazy' if kinds == {'lazy'} else 'structural re-export' if kinds == {'reexport'} else 'resolution target change')
        groups.setdefault(reason, []).append(edge)
    prior_runtime = {(edge['from'], edge['to']) for edge in before['edges'] if edge['kind'] in {'runtime', 'static'}}
    modules = {file['path']: file['module'] for file in after['files']}
    for edge in after['edges']:
        if edge['kind'] in {'runtime', 'static'} and (edge['from'], edge['to']) not in prior_runtime:
            if modules.get(edge['from']) in members or modules.get(edge['to']) in members:
                groups.setdefault('new or redirected runtime link', []).append(edge)
    return groups


def verify_type_controls(document):
    implicit = [edge for edge in document['edges']
                if edge['from'] == 'packages/common/decorators/core/use-guards.decorator.ts'
                and edge['to'] == 'packages/common/interfaces/features/can-activate.interface.ts']
    mixed_runtime = [edge for edge in document['edges']
                     if edge['from'] == 'packages/core/router/router-response-controller.ts'
                     and edge['kind'] == 'runtime']
    if len(implicit) != 1 or implicit[0]['kind'] != 'type' or not mixed_runtime:
        raise RuntimeError('Nest implicit-type or mixed-runtime positive control failed')


def readme_text(source, entries, documents):
    """Publish measured counts and an existing break-set edge, without ranking new suggestions."""
    heading = '## Results on 20 well-known repositories'
    start = source.index(heading)
    end = source.index('\n## ', start + len(heading))
    original = source[start:end]
    order = re.findall(r'https://dogebonker\.github\.io/boundrail/([a-z0-9._-]+)\.html', original)
    by_name = {entry['name']: entry for entry in entries}
    if len(order) != len(entries) or set(order) != set(by_name):
        raise RuntimeError('README showcase rows do not match the published manifest cohort')
    escape = lambda value: html.escape(str(value), quote=True).replace('|', '&#124;').replace('\n', ' ')
    lines = [heading, '',
             'boundrail on 20 open-source projects at pinned commits. Each name opens its live map; the path under a name is the folder that was scanned.', '',
             '| Project | Files | Largest cycle | Suggested cut |', '|---|--:|--:|---|']
    for name in order:
        entry, document = by_name[name], documents[name]
        project = escape(entry['repo'].removeprefix('https://github.com/'))
        label = f'[{project}](https://dogebonker.github.io/boundrail/{name}.html)'
        if entry['path'] != '.':
            label += '<br><sub><code>' + escape(entry['path']) + '</code></sub>'
        cycles = [cycle for cycle in document['cycles'] if cycle['level'] == 'module']
        largest = max(cycles, key=lambda cycle: len(cycle['members']), default=None)
        if largest is None:
            size, step = 'none', 'no module cycles'
        else:
            size = f"{len(largest['members'])} modules"
            if largest.get('break'):
                edge = largest['break'][0]
                if not any(candidate.get('from') == edge['from'] and candidate.get('to') == edge['to']
                           and candidate.get('kind') in {'runtime', 'static'} for candidate in document['edges']):
                    raise RuntimeError(f'{name}: suggested cut is not an existing runtime edge')
                step = 'review <code>' + escape(edge['from']) + ' → ' + escape(edge['to']) + '</code>'
            else:
                step = 'review the cycle report'
        lines.append(f"| {label} | {len(document['files']):,} | {size} | {step} |")
    lines += ['', '<sub>Generated from the final pinned scans in [showcase/repos.json](showcase/repos.json). Runtime cycles now exclude lazy, type-only and structural re-export edges; named imports resolve through barrels. Default test/generated/vendor/example exclusions change scan scope and can exclude real source in folders such as Next.js `src/build` or Django `test`. Include overrides restore file coverage; test-classified files still stay outside runtime cycles. [The measured comparison and caveats](showcase/edge-quality-report.md) document the methodology change. Suggested cuts are existing break-set edges from the largest remaining module SCC, to review rather than apply automatically. Rebuild with [`showcase/build.sh`](showcase/).</sub>', '']
    updated = source[:start] + '\n'.join(lines) + source[end:]
    if 'next.js' in by_name:
        entry, document = by_name['next.js'], documents['next.js']
        caption = '<p align="center"><sub>Next.js (<code>' + escape(entry['path']) + '</code> at <code>' + entry['commit'][:7] + '</code>): ' + f"{len(document['files']):,} files, {len(document['modules']):,} modules, {len(document['edges']):,} internal file dependencies." + ' The static illustration shows an earlier scan; click it for the current map. Made with <code>boundrail report</code> (<a href="showcase/">how</a>).</sub></p>'
        updated, replaced = re.subn(r'<p align="center"><sub>Next\.js .*?</sub></p>', lambda _: caption, updated, count=1)
        if replaced != 1:
            raise RuntimeError('README Next.js caption was not found')
        updated = re.sub(r'alt="The boundrail map of Next\.js:[^"]*"',
                         'alt="The boundrail map of Next.js: modules as discs with cycle and finding rings; the selected module highlights its importers and imports"', updated, count=1)
    return updated


def publish_readme(work, entries, output):
    complete, documents, _ = archived_stages(work, entries)
    if 'fix5' not in complete:
        raise RuntimeError('README generation requires the complete verified fix5 cohort')
    text = readme_text((HERE.parent / 'README.md').read_text(), entries[:20], documents[complete[-1]])
    if complete[-1] == 'corrected':
        text = text.replace('Generated from the final pinned scans in ',
                            'Generated from the final pinned scans, including the exact-case Python resolver correction, in ', 1)
    output.write_text(text)


def scan(entry, args, binary_hash):
    started = time.monotonic()
    destination = args.work / args.stage / entry['name']
    destination.mkdir(parents=True, exist_ok=True)
    command = []
    fingerprint = {'engine_sha256': binary_hash, 'engine_commit': args.engine_commit,
                   'entry': entry, 'extra_flags': args.extra_flag}
    meta = destination / 'metadata.json'
    if meta.exists() and not args.force:
        saved = json.loads(meta.read_text())
        if saved.get('fingerprint') == fingerprint and saved.get('status') == 'ok':
            print(f"{args.stage}/{entry['name']}: cached", flush=True)
            return saved
        raise RuntimeError(f'{meta}: incompatible or failed result; choose a new stage or --force')
    try:
        root = prepare(entry, args.work, args.cache)
        capture = destination / 'collector'
        capture.mkdir(exist_ok=True)
        program = {'typescript': args.depcruise, 'python': args.python, 'go': args.go}[entry['language']]
        shim = destination / 'collector-wrapper'
        wrapper(shim, program)
        collector_option = {'typescript': '--depcruise', 'python': '--python', 'go': '--go'}[entry['language']]
        graph = destination / 'graph.json'
        command = [str(args.engine), 'scan', str(root / entry['path']), '--language', entry['language'],
                   collector_option, str(shim), *entry['flags'], *args.extra_flag, '-o', str(graph)]
        with (destination / 'scan.log').open('wb') as log:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                    env=dict(os.environ, BOUNDRAIL_EDGE_CAPTURE=str(capture)), timeout=args.timeout)
        if result.returncode:
            raise RuntimeError(f'scan exited {result.returncode}; see {destination / "scan.log"}')
        document = json.loads(graph.read_text())
        if document['source'].get('git', {}).get('commit') != entry['commit']:
            raise RuntimeError('scan provenance does not match manifest commit')
        if document['source']['git'].get('dirty'):
            raise RuntimeError('scan reports changed source files')
        saved = {'status': 'ok', 'fingerprint': fingerprint, 'command': command,
                 'seconds': round(time.monotonic() - started, 3), 'graph_sha256': digest(graph),
                 'stats': stats(document),
                 'collector_artifacts': [
                     {'file': str(p.relative_to(destination)), 'sha256': digest(p), 'bytes': p.stat().st_size}
                     for p in sorted(capture.iterdir()) if p.is_file()]}
    except Exception as error:
        saved = {'status': 'failed', 'fingerprint': fingerprint, 'command': command,
                 'seconds': round(time.monotonic() - started, 3), 'error': str(error)}
    write_json(meta, saved)
    print(f"{args.stage}/{entry['name']}: {saved['status']} ({saved['seconds']}s)", flush=True)
    return saved


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--engine-commit', help='Exact source commit, or a recorded staged-source identifier')
    parser.add_argument('--stage')
    outputs = parser.add_mutually_exclusive_group()
    outputs.add_argument('--report-output', type=Path, help='Generate a report from complete archived stages; no scanning')
    outputs.add_argument('--readme-output', type=Path, help='Generate README rows from the latest complete verified final cohort, including corrections')
    parser.add_argument('--work', type=Path, default=Path('/private/tmp/boundrail-edge-quality'))
    parser.add_argument('--cache', type=Path, default=Path('/private/tmp/boundrail-hotspot-validation'))
    parser.add_argument('--manifest', type=Path, default=HERE / 'repos.json')
    parser.add_argument('--depcruise', type=Path, default=Path('/private/tmp/boundrail-impact-collectors/node_modules/.bin/depcruise'))
    parser.add_argument('--python', type=Path, default=Path('/private/tmp/boundrail-impact-python/bin/python'))
    parser.add_argument('--go', type=Path, default=Path(shutil.which('go') or 'go'))
    parser.add_argument('--jobs', type=int, default=3)
    parser.add_argument('--timeout', type=int, default=1800)
    parser.add_argument('--only', action='append', default=[])
    parser.add_argument('--extra-flag', action='append', default=[])
    parser.add_argument('--force', action='store_true')
    args = parser.parse_args()
    if args.readme_output:
        publish_readme(args.work.resolve(), json.loads(args.manifest.read_text()), args.readme_output)
        return 0
    if args.report_output:
        report(args.work.resolve(), json.loads(args.manifest.read_text()), args.report_output)
        return 0
    if not args.engine or not args.engine_commit or not args.stage:
        parser.error('scanning requires --engine, --engine-commit and --stage')
    if not re.fullmatch(r'[a-zA-Z0-9_-]+', args.stage):
        parser.error('--stage must be a simple directory name')
    if not 1 <= args.jobs <= 3:
        parser.error('--jobs must be between 1 and 3')
    args.work = args.work.resolve()
    args.engine = args.engine.resolve()
    entries = json.loads(args.manifest.read_text())
    entries = [e for e in entries if not args.only or e['name'] in args.only]
    if not entries:
        parser.error('no matching entries')
    for entry in entries:
        if not re.fullmatch(r'[a-z0-9][a-z0-9._-]*', entry['name']) or '..' in entry['name']:
            parser.error('unsafe manifest name')
        if not re.fullmatch('[0-9a-f]{40}', entry['commit']):
            parser.error('manifest commits must be full hashes')
        if Path(entry['path']).is_absolute() or '..' in Path(entry['path']).parts:
            parser.error('manifest path must stay inside repository')
    engine_hash = digest(args.engine)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = list(pool.map(lambda e: scan(e, args, engine_hash), entries))
    write_json(args.work / args.stage / 'index.json', results)
    return int(any(r['status'] != 'ok' for r in results))


if __name__ == '__main__':
    sys.exit(main())
