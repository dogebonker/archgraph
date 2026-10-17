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
    new = {(e['from'], e['to'], e.get('kind')) for e in after['edges']}
    endpoints = {(e['from'], e['to']) for e in after['edges']}
    result = {'excluded_files': sorted({p for e in old for p in (e['from'], e['to']) if p not in kept}),
              'removed_or_redirected': [], 'changed_kinds': []}
    for edge in old:
        pair = (edge['from'], edge['to'])
        if pair not in endpoints and all(p in kept for p in pair):
            result['removed_or_redirected'].append(edge)
        elif pair in endpoints and (*pair, edge.get('kind')) not in new:
            result['changed_kinds'].append(edge)
    return result


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
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--engine-commit', required=True, help='Exact source commit, or a recorded staged-source identifier')
    parser.add_argument('--stage', required=True)
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
