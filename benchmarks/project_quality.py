#!/usr/bin/env python3
"""Read-only native-engine comparison on Glarity and MoneyLust working trees.

Raw graphs and logs stay in a private artifact directory. The optional Markdown
report contains aggregate counts, hashes and commands with checkout placeholders.
"""
import argparse
from collections import Counter
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import runpy
import shutil
import statistics
import subprocess
import time

_HELPERS = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'showcase/recipes.py'))
digest = _HELPERS['digest']
source_state = _HELPERS['source_state']
git = _HELPERS['git']


class GraphParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=False)
        self.active = False
        self.blocks = []

    def handle_starttag(self, tag, attrs):
        if tag == 'script' and dict(attrs).get('id') == 'graph':
            self.active = True
            self.blocks.append('')

    def handle_endtag(self, tag):
        if tag == 'script':
            self.active = False

    def handle_data(self, data):
        if self.active:
            self.blocks[-1] += data


def graph_from_html(html):
    parser = GraphParser()
    parser.feed(html)
    if len(parser.blocks) != 1:
        raise ValueError('render must contain exactly one script#graph document')
    graph = json.loads(parser.blocks[0])
    if graph.get('kind') != 'scan':
        raise ValueError('rendered graph is not a scan document')
    return graph


def metrics(graph):
    cycles = graph.get('cycles', [])
    module_cycles = [c for c in cycles if c['level'] == 'module']
    file_cycles = [c for c in cycles if c['level'] == 'file']
    kinds = Counter({'static': 'runtime', 'dynamic': 'lazy', 'type-reexport': 'type'}.get(e.get('kind'), e.get('kind', 'runtime')) for e in graph['edges'])
    return {
        'files': len(graph['files']), 'modules': len(graph['modules']),
        'internal_edge_records': len(graph['edges']),
        'external_edge_records': len(graph.get('external_edges', [])),
        'edge_kinds': dict(sorted(kinds.items())),
        'import_resolution': graph.get('import_resolution'),
        'module_cycles': len(module_cycles),
        'largest_module_cycle': max((len(c['members']) for c in module_cycles), default=0),
        'file_cycles': len(file_cycles),
        'largest_file_cycle': max((len(c['members']) for c in file_cycles), default=0),
        'type_only_cycles': graph.get('summary', {}).get('type_only_cycles'),
        'findings': len(graph.get('findings', [])),
        'findings_by_kind': dict(sorted(Counter(f['kind'] for f in graph.get('findings', [])).items())),
        'diagnostics': len(graph.get('diagnostics', [])),
        'deep_imports': sum(len(m['deep_imports']) for m in (graph.get('surface') or {}).get('modules', [])),
    }


def fixed_layout(graph):
    """Reuse the old physical folder boundaries, independent of source-root defaults."""
    files = {f['path'] for f in graph['files']}
    ids = [m['id'] for m in graph['modules']]
    if any(any(c in name for c in '*?[]{}\\') for name in ids):
        raise ValueError('baseline module paths contain glob metacharacters; supply a literal-safe fixture')
    modules = [{'id': name, 'path': name if name in files else name.rstrip('/') + '/**' if name else '**'}
               for name in sorted(ids, key=lambda name: (-len(name.split('/')), -len(name), name))]
    return {'version': 1, 'modules': modules}


def timing(samples):
    return {'samples': samples, 'median': statistics.median(samples), 'min': min(samples), 'max': max(samples)}


def ensure_outside(path, roots):
    path = path.resolve()
    if any(path == root.resolve() or path.is_relative_to(root.resolve()) for root in roots):
        raise ValueError('artifact/report output must be outside the application checkouts')
    return path


def unchanged(before, after, name):
    if before != after:
        raise RuntimeError(f'{name}: source or Git index changed; stop and inspect the checkout')


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def call(command, destination):
    started = time.perf_counter()
    with destination.with_suffix('.stdout').open('wb') as stdout, destination.with_suffix('.stderr').open('wb') as stderr:
        process = subprocess.run([str(p) for p in command], stdout=stdout, stderr=stderr, timeout=1200, check=False)
    elapsed = round(time.perf_counter() - started, 6)
    record = {'command': [str(p) for p in command], 'seconds': elapsed, 'exit_code': process.returncode,
              'stdout_sha256': digest(destination.with_suffix('.stdout').read_bytes()),
              'stderr_sha256': digest(destination.with_suffix('.stderr').read_bytes())}
    write_json(destination.with_suffix('.run.json'), record)
    if process.returncode:
        raise RuntimeError(f'command exited {process.returncode}; inspect private {destination.with_suffix(".stderr")}')
    return record


def public_command(command, replacements):
    result = []
    for arg in command:
        for source, replacement in replacements:
            arg = arg.replace(str(source), replacement)
        result.append(arg)
    return result


def render_report(result):
    lines = ['# Native input-quality regression comparison', '',
             'Same actual working trees; no source edits or recipes applied. Frozen binary copies identify each engine. Times include native collection, Git co-change/hotspot work and analysis; repeated scans alternate engine order and benefit from OS caches.', '',
             '**Grouping:** MoneyLust backend uses old `--depth 3` versus new `--depth 2` because Rust now normalizes the `src/` root. A separate saved-graph render applies identical old physical-folder module rules to both graphs; this comparison layout is a measurement device, not an approved architecture policy.', '',
             'Raw graphs, source paths, recipe buffers and logs remain in the private artifact directory. Source/index proofs cover Git-visible tracked and nonignored untracked files, modes and symlinks; `.archgraph/`, ignored install/build artifacts and submodule contents are outside the reused preservation check.', '',
             f"Before binary SHA-256: `{result['binaries']['before']}`. After: `{result['binaries']['after']}`.", '',
             '| Scan | Engine | Files | Modules | Internal edges | Resolved % | Module SCCs / largest | Deep imports | Median seconds (min–max) |',
             '| --- | --- | ---: | ---: | ---: | ---: | --- | ---: | --- |']
    for name, comparison in result['scans'].items():
        for engine in ['before', 'after']:
            row = comparison[engine]
            m, t = row['metrics'], row['timing_seconds']
            coverage = (m['import_resolution'] or {}).get('resolved_percent', '?')
            lines.append(f"| {name} | {engine} | {m['files']} | {m['modules']} | {m['internal_edge_records']} | {coverage} | {m['module_cycles']} / {m['largest_module_cycle']} | {m['deep_imports']} | {t['median']:.3f} ({t['min']:.3f}–{t['max']:.3f}) |")
    lines += ['', '| Scan | Engine | Imports / unresolved | Edge kinds | File SCCs / largest | Findings / diagnostics |',
              '| --- | --- | ---: | --- | --- | --- |']
    for name, comparison in result['scans'].items():
        for engine in ['before', 'after']:
            m = comparison[engine]['metrics']
            r = m['import_resolution'] or {}
            kinds = ', '.join(f'{kind}: {count}' for kind, count in m['edge_kinds'].items())
            lines.append(f"| {name} | {engine} | {r.get('imports', '?')} / {r.get('unresolved', '?')} | {kinds} | {m['file_cycles']} / {m['largest_file_cycle']} | {m['findings']} / {m['diagnostics']} |")
    lines += ['', '| Same-folder Rust comparison | Modules | Module SCCs | Largest |', '| --- | ---: | ---: | ---: |']
    for engine, row in result['rust_same_boundaries'].items():
        lines.append(f"| {engine} | {row['modules']} | {row['module_cycles']} | {row['largest_module_cycle']} |")
    lines += ['', '## Preservation hashes', '', '| Checkout | Source SHA-256 | Git index SHA-256 | Unchanged |', '| --- | --- | --- | --- |']
    for name, proof in result['preservation'].items():
        lines.append(f"| {name} | `{proof['before']['source_sha256']}` | `{proof['before']['index_sha256']}` | {proof['before'] == proof['after']} |")
    lines += ['', '## Commands', '', 'Each scan is repeated with both frozen binaries. Placeholders keep local paths private.', '', '```sh']
    lines.extend(' '.join(command) for command in result['commands'])
    lines += ['```', '', 'Counts describe static dependency components, not runtime initialization cycles. A smaller SCC alone does not establish correctness; Rust test/containment and Python package/facade regressions are checked separately by fixture tests. No application build or production architecture-policy audit was performed.', '']
    return '\n'.join(lines)


def compare(args):
    if args.repeats < 1:
        raise ValueError('--repeats must be at least 1')
    checkouts = {name: Path(os.fsdecode(git(path.resolve(), 'rev-parse', '--show-toplevel')).strip()).resolve()
                 for name, path in [('glarity', args.glarity), ('moneylust', args.moneylust)]}
    out = ensure_outside(args.out, list(checkouts.values()))
    if args.report:
        ensure_outside(args.report, list(checkouts.values()))
    if out.exists() and any(out.iterdir()):
        raise ValueError('--out must be a new or empty private artifact directory')
    out.mkdir(parents=True, mode=0o700, exist_ok=True)
    if out.stat().st_mode & 0o077:
        raise ValueError('--out must have private permissions (chmod 700)')
    binaries = {}
    for engine in ['before', 'after']:
        target = out / f'{engine}-archgraph'
        shutil.copyfile(getattr(args, engine).resolve(), target)
        target.chmod(0o500)
        binaries[engine] = target
    result = {'methodology': 'native actual-working-tree scans; fixed old Rust folder grouping rendered separately',
              'repeats': args.repeats, 'binaries': {k: digest(v.read_bytes()) for k, v in binaries.items()},
              'script_sha256': digest(Path(__file__).read_bytes()), 'scans': {}, 'rust_same_boundaries': {},
              'preservation': {}, 'commands': []}
    replacements = [(v, f'<{k}-binary>') for k, v in binaries.items()] + [(out, '<artifacts>')] + [(path, f'<{name}>') for name, path in checkouts.items()]
    jobs = [
        ('glarity', 'glarity-backend-symbols', args.glarity.resolve(), ['--language', 'python', '--package', 'bitxchange', '--cochange=300', '--hotspots', '--hotspot-authors', '--symbols']),
        ('glarity', 'glarity-backend-no-symbols', args.glarity.resolve(), ['--language', 'python', '--package', 'bitxchange', '--cochange=300', '--hotspots', '--hotspot-authors']),
        ('moneylust', 'moneylust-backend', args.moneylust.resolve() / 'backend', ['--language', 'rust']),
        ('moneylust', 'moneylust-frontend', args.moneylust.resolve() / 'frontend', ['--language', 'typescript']),
    ]
    for name, checkout in checkouts.items():
        before = source_state(checkout)
        try:
            for owner, label, root, flags in jobs:
                if owner != name:
                    continue
                samples = {'before': [], 'after': []}
                result['scans'][label] = {}
                for repeat in range(args.repeats):
                    for engine in (['before', 'after'] if repeat % 2 == 0 else ['after', 'before']):
                        prefix = out / f'{label}-{engine}-{repeat + 1}'
                        graph_path = prefix.with_suffix('.json')
                        depth = ['--depth', '3' if engine == 'before' else '2'] if label == 'moneylust-backend' else []
                        command = [binaries[engine], '--no-daemon', 'scan', root, '--engine', 'native', *flags, *depth, '-o', graph_path]
                        run = call(command, prefix)
                        graph = json.loads(graph_path.read_bytes())
                        measured = metrics(graph)
                        previous = result['scans'][label].setdefault(engine, {'metrics': measured, 'runs': []})
                        if previous['metrics'] != measured:
                            raise RuntimeError(f'{label}/{engine}: aggregate counts changed between repeats')
                        previous['runs'].append({**run, 'graph_sha256': digest(graph_path.read_bytes())})
                        samples[engine].append(run['seconds'])
                        if repeat == 0:
                            result['commands'].append(public_command(run['command'], replacements))
                for engine in ['before', 'after']:
                    result['scans'][label][engine]['timing_seconds'] = timing(samples[engine])
                if label == 'moneylust-backend':
                    old = json.loads((out / f'{label}-before-1.json').read_bytes())
                    rules = out / 'rust-same-boundaries.json'
                    write_json(rules, fixed_layout(old))
                    for engine in ['before', 'after']:
                        html = out / f'rust-same-boundaries-{engine}.html'
                        command = [binaries[engine], '--no-daemon', 'render', out / f'{label}-{engine}-1.json', '--rules', rules, '-o', html]
                        run = call(command, out / f'rust-same-boundaries-{engine}')
                        result['commands'].append(public_command(run['command'], replacements))
                        result['rust_same_boundaries'][engine] = metrics(graph_from_html(html.read_text()))
        finally:
            after = source_state(checkout)
            result['preservation'][name] = {'before': before, 'after': after}
            write_json(out / 'preservation.json', result['preservation'])
            unchanged(before, after, name)
    if any(digest(path.read_bytes()) != result['binaries'][engine] for engine, path in binaries.items()):
        raise RuntimeError('a frozen binary changed during the comparison')
    write_json(out / 'results.json', result)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(render_report(result))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['before', 'after', 'glarity', 'moneylust', 'out']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--repeats', type=int, default=3)
    args = parser.parse_args()
    try:
        result = compare(args)
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f'comparison failed: {error}\n')
    print(json.dumps({'scans': len(result['scans']), 'source_unchanged': True, 'binaries': result['binaries']}, sort_keys=True))


if __name__ == '__main__':
    main()
