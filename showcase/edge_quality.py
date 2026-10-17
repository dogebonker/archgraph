#!/usr/bin/env python3
"""Reproduce pinned edge-quality scans without installing into target repositories.

Run each engine with a separate stage name. JSON, collector stdout, commands, logs,
engine hashes and timing are retained outside the repository (default /tmp).
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent


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
p=pathlib.Path(os.environ['ARCHGRAPH_EDGE_CAPTURE'])/str(time.time_ns())
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
    return {
        'files': len(doc['files']), 'modules': len(doc['modules']),
        'edges': len(doc['edges']), 'edge_kinds': kinds,
        'collector': doc['source']['collector'],
        'module_cycles': len(cycles),
        'largest_module_cycle': max((len(c['members']) for c in cycles), default=0),
        'cycle_members': [c['members'] for c in cycles],
        'deferred_module_cycles': sum(c.get('level') == 'module' for c in doc.get('type_only_cycles', [])),
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
            before['schema'] == 'archgraph/v1' and edge.get('kind') == 'reexport')
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


def report(work, entries, output):
    stages = ['baseline', 'fix1', 'fix2', 'fix3', 'fix4', 'fix5']
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
            documents[stage][entry['name']] = json.loads(path.read_text())
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
             '**Counting units:** edges are canonical file-dependency records, not source import statements. One named import can resolve to several defining files and increase the edge count. Runtime/lazy/type/re-export columns below count edges, and legacy static/dynamic names are mapped to runtime/lazy for comparison. Import-statement totals and resolution percentages remain pending until the resolution ledger exists; neither is inferred from graph edges or the old unresolved counter.', '',
             'The baseline already excluded type-only and dynamic edges from runtime cycles. Fix 1 additionally excludes structural re-exports, so an observed decrease is not automatically a newly detected type-only cycle. Deferred SCCs containing an existing runtime SCC are reported separately from wholly non-runtime SCCs. JavaScript re-exports can still cause module evaluation: these results describe the requested graph policy, not proof that all runtime execution is acyclic.', '',
             '## Engine provenance', '', '| Stage | Source commit | Binary SHA-256 |', '| --- | --- | --- |']
    for stage in complete:
        fingerprints = {(row['fingerprint']['engine_commit'], row['fingerprint']['engine_sha256']) for row in metadata[stage].values()}
        if len(fingerprints) != 1:
            raise RuntimeError(f'{stage}: inconsistent engine provenance')
        commit, binary_hash = fingerprints.pop()
        lines.append(f'| {stage} | `{commit}` | `{binary_hash}` |')
    lines += ['', 'Collector versions are recorded per scan in `metadata.json`. Reproduce a stage with:', '', '```sh', 'python3 -B showcase/edge_quality.py --engine /absolute/frozen-archgraph \\', '  --engine-commit FULL_SOURCE_COMMIT --stage fixN', 'python3 -B showcase/edge_quality.py --report-output showcase/edge-quality-report.md', '```', '', '## Published 20 repositories', '',
              'Within each arrow cell the stages are ' + ' → '.join(complete) + '. SCC means a strongly connected component with a cycle; largest is its module count.', '',
              '| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (' + latest + ') | Barrels (' + latest + ') |',
              '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    def table_rows(cohort):
        for entry in cohort:
            name = entry['name']
            snapshots = [stats(documents[stage][name]) for stage in complete]
            arrow = lambda key: ' → '.join(str(snapshot[key]) for snapshot in snapshots)
            kinds = {}
            for edge in documents[latest][name]['edges']:
                kind = {'static': 'runtime', 'dynamic': 'lazy', 'type-reexport': 'type'}.get(edge['kind'], edge['kind'])
                kinds[kind] = kinds.get(kind, 0) + 1
            counts = ' / '.join(str(kinds.get(kind, 0)) for kind in ['runtime', 'lazy', 'type', 'reexport'])
            lines.append(f"| {name} | {arrow('files')} | {arrow('edges')} | {arrow('module_cycles')} | {arrow('largest_module_cycle')} | {counts} | {snapshots[-1]['barrels']} |")
    table_rows(entries[:20])
    lines += ['', '## Supplemental Go repositories', '', '| Repository | Files | File edges | Module SCCs | Largest SCC | Runtime / lazy / type / re-export edges (' + latest + ') | Barrels (' + latest + ') |', '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    table_rows(entries[20:])
    lines += ['', '## Resolution and exclusions', '',
              'Source import-statement totals, resolved percentages, default-excluded file counts, and grab-bag reclassification counts: **pending fixes 3–5**. Old collector dependency counts have a different denominator and are not presented as import-statement coverage.', '',
              '## Every baseline module SCC', '',
              'This inventory includes all original SCCs, including shrinkage. An old SCC absorbed into a larger SCC still persists; it is not counted as removed. Fixes 1–2 have one fully eliminated original SCC, six shrunken original SCCs, and 26 unchanged original SCCs. Fix 2 also expands the surviving Nest common SCC from five to six modules.', '',
              '| Repository / baseline SCC | Original modules | ' + ' | '.join(complete[1:]) + ' | Explanation of changed membership |',
              '| --- | ---: | ' + ' | '.join('---' for _ in complete[1:]) + ' | --- |']
    for entry in entries:
        name = entry['name']
        for number, members in enumerate(module_cycles(documents['baseline'][name]), 1):
            dispositions = [cycle_disposition(members, documents[stage][name]) for stage in complete[1:]]
            explanation = 'Unchanged through measured stages.'
            if any(disposition != 'unchanged' for disposition in dispositions):
                explanation = 'Structural re-export/barrel policy at fix1; see source witnesses below. No baseline runtime SCC disappeared solely because of type/lazy filtering.'
            if name == 'nest' and 'packages/common/file-stream' in members:
                explanation += ' Named barrel resolution at fix2 restores common itself to the surviving SCC (5 → 6); this is expansion, not another disappearance.'
            label = ', '.join(sorted(members)[:3]) + (', …' if len(members) > 3 else '')
            lines.append(f"| {name} #{number}: `{label}` | {len(members)} | " + ' | '.join(dispositions) + f' | {explanation} |')
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
    lines += ['', 'Fastify return-path check: `packages/platform-fastify/adapters/fastify-adapter.ts:73` and `packages/platform-fastify/decorators/route-config.decorator.ts:2` import parent-folder `constants.js`; the parent `index.ts:8,10` exports children. These are different files grouped into one parent module, explaining the projected loop.', '',
              'No wholly removed SCC attributable to type-only imports, test exclusions, resolution fixes, or grab-bag splitting has yet been measured. Those categories remain pending rather than being inferred from a net count.', '',
              '## Remaining validation', '',
              '- Measure fixes 3–5 and update the full SCC inventory with stage-specific edge/file evidence.',
              '- Fix the implicit TypeScript type-use gap found in Nest: `packages/common/decorators/core/use-guards.decorator.ts:2,29` imports `CanActivate` only as a type; its interface declaration is `packages/common/interfaces/features/can-activate.interface.ts:14`, but the fix2 snapshot still marks the edge runtime. Mixed type/value imports must retain their runtime bindings. This gap is pending bounded refinement, not counted as solved.',
              '- Fill the import-statement ledger, resolution coverage, exclusion counts, and grab-bag classifications from actual engine fields.',
              '- Rebuild generated showcase outputs and README numbers only after all stages pass.',
              '- Fix any source witness or resolution failure that contradicts the intended dependency graph before publishing final claims.', '']
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text('\n'.join(lines))


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
                                    env=dict(os.environ, ARCHGRAPH_EDGE_CAPTURE=str(capture)), timeout=args.timeout)
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
    parser.add_argument('--report-output', type=Path, help='Generate a report from complete archived stages; no scanning')
    parser.add_argument('--work', type=Path, default=Path('/private/tmp/archgraph-edge-quality'))
    parser.add_argument('--cache', type=Path, default=Path('/private/tmp/archgraph-hotspot-validation'))
    parser.add_argument('--manifest', type=Path, default=HERE / 'repos.json')
    parser.add_argument('--depcruise', type=Path, default=Path('/private/tmp/archgraph-impact-collectors/node_modules/.bin/depcruise'))
    parser.add_argument('--python', type=Path, default=Path('/private/tmp/archgraph-impact-python/bin/python'))
    parser.add_argument('--go', type=Path, default=Path(shutil.which('go') or 'go'))
    parser.add_argument('--jobs', type=int, default=3)
    parser.add_argument('--timeout', type=int, default=1800)
    parser.add_argument('--only', action='append', default=[])
    parser.add_argument('--extra-flag', action='append', default=[])
    parser.add_argument('--force', action='store_true')
    args = parser.parse_args()
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
