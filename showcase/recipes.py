#!/usr/bin/env python3
"""Audit real CLI recipes against the original 20 pinned TS/Python showcase checkouts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time
import sys

ORIGINAL = 'nest next.js excalidraw immich activepieces payload typeorm vite outline strapi trpc django fastapi flask pydantic httpx scrapy celery zulip sqlalchemy'.split()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def git(root, *args):
    env = dict(os.environ, GIT_OPTIONAL_LOCKS='0')
    command = ['git', '-C', str(root), '-c', 'core.fsmonitor=false', '-c', 'core.hooksPath=/dev/null', '-c', 'diff.autoRefreshIndex=false', *args]
    return subprocess.check_output(command, env=env)


def source_state(checkout):
    """Hash Git-visible file bytes/modes and index, excluding only .archgraph metadata."""
    files = sorted(set(git(checkout, 'ls-files', '-z', '--cached', '--others', '--exclude-standard').split(b'\0')) - {b''})
    state = hashlib.sha256()
    count = 0
    for name in files:
        relative = os.fsdecode(name)
        if '.archgraph' in Path(relative).parts:
            continue
        path = checkout / relative
        try:
            info = path.lstat()
        except FileNotFoundError:
            data = b'<missing>'
            mode = 0
        else:
            mode = stat.S_IMODE(info.st_mode)
            if stat.S_ISLNK(info.st_mode):
                data = b'link:' + os.fsencode(os.readlink(path))
            elif stat.S_ISREG(info.st_mode):
                data = path.read_bytes()
            elif stat.S_ISDIR(info.st_mode):
                data = b'<gitlink-directory>'
            else:
                raise RuntimeError(f'unsupported source file kind: {path}')
        state.update(len(name).to_bytes(8, 'big'))
        state.update(name)
        state.update(mode.to_bytes(4, 'big'))
        state.update(len(data).to_bytes(8, 'big'))
        state.update(data)
        count += 1
    index = Path(os.fsdecode(git(checkout, 'rev-parse', '--path-format=absolute', '--git-path', 'index')).rstrip('\r\n'))
    tracked_clean = not git(checkout, 'status', '--porcelain=v1', '--untracked-files=no').strip()
    return {'files': count, 'tracked_clean': tracked_clean, 'source_sha256': state.hexdigest(), 'index_sha256': digest(index.read_bytes()) if index.exists() else None}


def call(command):
    started = time.perf_counter()
    process = subprocess.run([str(part) for part in command], capture_output=True, timeout=600, check=False)
    try:
        value = json.loads(process.stdout)
    except (ValueError, UnicodeDecodeError):
        value = None
    return {'exit_code': process.returncode, 'elapsed_seconds': round(time.perf_counter() - started, 6), 'stdout_sha256': digest(process.stdout), 'stderr': process.stderr.decode(errors='replace').strip(), 'value': value}


def compact_recipe(recipe):
    if not recipe:
        return '—'
    return '; '.join(f"{step['kind']} {step['file']} → {step['to']}" for step in recipe['steps'])


def cell(value):
    return str(value).replace('|', '\\|').replace('\n', ' ').replace('\r', '')


def render(report):
    lines = [
        '# Pinned showcase fix recipes', '',
        'Actual CLI `propose` and `apply --dry-run` results for the original 20 TypeScript/Python showcase repositories. Findings are recomputed and ranked by the current engine from the saved edge-quality graphs; the source collector is not rerun. No recipes were applied.', '',
        f"Binary SHA-256: `{report['binary_sha256']}`. Manifest SHA-256: `{report['manifest_sha256']}`. Full recipes, exact commands, input graph hashes and source/index checks are in [`recipes-results.json`](recipes-results.json).", '',
        'The executables are frozen private copies for the whole audit, even if Cargo rebuilds concurrently. The JSON records the working revision and source-input hashes; binary hashes identify the measured engine, independently of uncommitted or concurrent source edits.', '',
        '“Ready” means the source-aware mechanical planner accepted the dry-run, not that runtime behavior or compilation has been proved. “Blocked” records the planner’s real reason; an agent must perform those semantic changes. Unknown predicted deltas are shown as `?`.', '',
        '| Repository | Pin | Top finding / recipe | Confidence | Predicted pairs removed | Actual dry-run |',
        '| --- | --- | --- | --- | ---: | --- |',
    ]
    for row in report['repos']:
        recipe = row.get('dry_run', {}).get('planned_recipe') or row.get('recipe')
        recipe_text = compact_recipe(recipe)
        if row.get('finding'):
            recipe_text = f"{row['finding']['kind']}: {recipe_text}"
        if row.get('error'):
            outcome = 'ERROR: ' + row['error']
        elif row.get('dry_run', {}).get('can_apply'):
            outcome = 'Ready (exit 0)'
        elif row.get('dry_run'):
            outcome = 'Blocked (exit ' + str(row['dry_run']['exit_code']) + '): ' + '; '.join(row['dry_run']['pending'])
        else:
            outcome = 'No actionable finding'
        if len(outcome) > 360:
            outcome = outcome[:357] + '… (full reason in JSON)'
        pairs = recipe.get('expected', {}).get('pairs_removed') if recipe else None
        lines.append('| ' + ' | '.join(cell(value) for value in [row['name'], f"`{row['pin'][:12]}`", recipe_text, recipe['confidence'] if recipe else '—', '?' if pairs is None else pairs, outcome]) + ' |')
    ready = sum(bool(row.get('dry_run', {}).get('can_apply')) for row in report['repos'])
    errors = sum(bool(row.get('error')) for row in report['repos'])
    lines += ['', f'Measured: {len(report["repos"])} repositories; {ready} mechanically ready dry-runs; {sum(bool(row.get("dry_run")) for row in report["repos"]) - ready} blocked; {sum(not row.get("finding") and not row.get("error") for row in report["repos"])} with no actionable finding; {errors} audit errors. Source bytes/modes and the real Git index were unchanged around each audit.', '',
              'The unchanged-source check hashes tracked and nonignored untracked files from `git ls-files`, including symlink targets and file modes. `.archgraph/` metadata is explicitly allowed; ignored build/install artifacts and submodule contents are outside this check. Each graph’s recorded commit and the checkout HEAD must both equal the manifest pin. Tracked-clean status is recorded for each checkout; any existing dirtiness is preserved rather than discarded.', '']
    return '\n'.join(lines)


def code_inputs(root):
    state = hashlib.sha256()
    files = []
    for name in ['src', 'schema', 'assets', 'collectors', 'queries', 'examples']:
        directory = root / name
        if directory.is_dir():
            files.extend(path for path in directory.rglob('*') if path.is_file() and not path.is_symlink())
    files.extend(path for name in ['Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml'] if (path := root / name).is_file())
    for path in sorted(files):
        name = str(path.relative_to(root)).encode()
        data = path.read_bytes()
        state.update(len(name).to_bytes(8, 'big'))
        state.update(name)
        state.update(len(data).to_bytes(8, 'big'))
        state.update(data)
    return state.hexdigest()


def audit(args):
    started = time.perf_counter()
    manifest = json.loads(args.manifest.read_bytes())
    entries = {entry['name']: entry for entry in manifest}
    report = {'methodology': 'current ranked recipes over saved corrected graphs; actual CLI propose and apply --dry-run; no source collection or apply --yes',
              'binary_sha256': digest(args.archgraph.read_bytes()), 'inventory_sha256': digest(args.inventory.read_bytes()),
              'manifest_sha256': digest(args.manifest.read_bytes()), 'script_sha256': digest(Path(__file__).read_bytes()),
              'working_revision': git(args.code_root, 'rev-parse', 'HEAD').decode().strip(),
              'code_input_sha256': code_inputs(args.code_root), 'binary_source': args.binary_source,
              'inventory_source': args.inventory_source, 'repos': []}
    for name in ORIGINAL:
        entry = entries[name]
        checkout = args.repos / name
        root = checkout / entry['path']
        graph_path = args.graphs / name / 'graph.json'
        graph_bytes = graph_path.read_bytes()
        graph = json.loads(graph_bytes)
        head = git(checkout, 'rev-parse', 'HEAD').decode().strip()
        if head != entry['commit'] or graph['source']['git']['commit'] != entry['commit']:
            raise RuntimeError(f'{name}: graph/checkout/manifest commit mismatch; restore the pinned inputs first')
        before = source_state(checkout)
        row = {'name': name, 'repo': entry['repo'], 'pin': head, 'path': entry['path'], 'input_schema': graph['schema'], 'input_collector': graph['source']['collector'], 'graph_sha256': digest(graph_bytes), 'source_before': before}
        try:
            found = call([args.inventory, graph_path])
            if found['exit_code'] != 0 or not isinstance(found['value'], list):
                raise RuntimeError('inventory failed: ' + found['stderr'])
            if found['value']:
                finding = found['value'][0]
                row['finding'] = finding
                identifier = finding['finding_id']
                propose = [args.archgraph, '--no-daemon', 'propose', identifier, '--graph', graph_path, '--root', root]
                dry = [args.archgraph, '--no-daemon', 'apply', identifier, '--graph', graph_path, '--root', root, '--dry-run']
                row['commands'] = [[str(part) for part in command] for command in [propose, dry]]
                proposed = call(propose)
                row['propose'] = {key: value for key, value in proposed.items() if key != 'value'}
                if proposed['exit_code'] != 0 or not isinstance(proposed['value'], dict) or proposed['value'].get('kind') != 'recipe':
                    raise RuntimeError('propose failed: ' + proposed['stderr'])
                row['recipe'] = {key: value for key, value in proposed['value'].items() if key not in ('schema', 'kind', 'finding_id')}
                planned = call(dry)
                plan = planned['value']
                if not isinstance(plan, dict) or plan.get('kind') != 'apply-plan':
                    raise RuntimeError('dry-run failed without a plan: ' + planned['stderr'])
                expected_exit = 0 if plan['can_apply'] else 2
                if planned['exit_code'] != expected_exit:
                    raise RuntimeError(f'dry-run exit {planned["exit_code"]} disagrees with can_apply={plan["can_apply"]}')
                row['dry_run'] = {key: value for key, value in planned.items() if key != 'value'}
                row['dry_run'].update(can_apply=plan['can_apply'], pending=plan['pending'], moves=plan['moves'], planned_recipe=plan['recipe'], changes=[{key: value for key, value in change.items() if key not in ('before', 'after')} for change in plan['changes']])
        except (RuntimeError, subprocess.TimeoutExpired) as error:
            row['error'] = str(error)
        after = source_state(checkout)
        row['source_after'] = after
        if before != after:
            raise RuntimeError(f'{name}: source/index changed during the audit; stop and inspect the checkout')
        report['repos'].append(row)
        print(f"{name}: {'ERROR' if row.get('error') else 'ready' if row.get('dry_run', {}).get('can_apply') else 'blocked' if row.get('dry_run') else 'no finding'}", file=sys.stderr, flush=True)
        args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
        args.markdown.write_text(render(report))
    report['elapsed_seconds'] = round(time.perf_counter() - started, 6)
    report['code_input_sha256_after'] = code_inputs(args.code_root)
    report['code_changed_during_audit'] = report['code_input_sha256'] != report['code_input_sha256_after']
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    args.markdown.write_text(render(report))
    return any(row.get('error') for row in report['repos'])


def self_test():
    assert len(ORIGINAL) == len(set(ORIGINAL)) == 20
    assert digest(b'abc') == 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
    assert cell('a|b\nc') == 'a\\|b c'
    assert compact_recipe({'steps': [{'kind': 'move_file', 'file': 'a.ts', 'to': 'b.ts'}]}) == 'move_file a.ts → b.ts'
    value = render({'binary_sha256': 'a', 'manifest_sha256': 'b', 'repos': [{'name': 'fixture', 'pin': '1' * 40, 'recipe': {'steps': [], 'confidence': 'low', 'expected': {}}, 'dry_run': {'can_apply': False, 'exit_code': 2, 'pending': ['manual']}}]})
    assert 'Blocked (exit 2): manual' in value and '0 mechanically ready' in value
    print('recipes.py self-test passed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--manifest', type=Path, default=Path(__file__).with_name('repos.json'))
    parser.add_argument('--graphs', type=Path, default=Path('/private/tmp/archgraph-edge-quality/corrected'))
    parser.add_argument('--repos', type=Path, default=Path('/private/tmp/archgraph-edge-quality/repos'))
    parser.add_argument('--archgraph', type=Path, default=Path('target/debug/archgraph'))
    parser.add_argument('--inventory', type=Path, default=Path('target/debug/examples/recipe_inventory'))
    parser.add_argument('--output', type=Path, default=Path(__file__).with_name('recipes-results.json'))
    parser.add_argument('--markdown', type=Path, default=Path(__file__).with_name('recipes.md'))
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    args.archgraph = args.archgraph.resolve()
    args.inventory = args.inventory.resolve()
    args.repos = args.repos.resolve()
    args.graphs = args.graphs.resolve()
    args.code_root = Path(__file__).resolve().parents[1]
    args.binary_source = str(args.archgraph)
    args.inventory_source = str(args.inventory)
    # Snapshot executable bytes so concurrent cargo builds cannot change the measured engine.
    with tempfile.TemporaryDirectory(prefix='archgraph-recipe-audit-') as directory:
        for name in ['archgraph', 'inventory']:
            original = getattr(args, name)
            frozen = Path(directory) / name
            frozen.write_bytes(original.read_bytes())
            frozen.chmod(0o700)
            setattr(args, name, frozen)
        return int(audit(args))


if __name__ == '__main__':
    sys.exit(main())
