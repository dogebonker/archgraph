#!/usr/bin/env python3
"""Paired, isolated agent runs. Python standard library only."""
import argparse
from datetime import datetime, timezone
import fnmatch
import hashlib
import importlib.util
import html
import json
import os
import platform
from pathlib import Path
import re
import shutil
import signal
import statistics
import subprocess
import tempfile
import time
import uuid
import sys

CONFIGS = ('.claude/settings.json', '.claude/settings.local.json', '.codex/hooks.json',
           '.cursor/hooks.json', '.opencode/plugins')
ISOLATION = ('HOME', 'CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'XDG_CONFIG_HOME')


def invoke(argv, cwd, timeout, env=None):
    started = time.monotonic()
    env = dict(os.environ if env is None else env, GIT_OPTIONAL_LOCKS='0')
    try:
        proc = subprocess.Popen(list(map(str, argv)), cwd=cwd, env=env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                text=True, errors='replace', start_new_session=os.name == 'posix')
        try:
            stdout, stderr = proc.communicate(timeout=timeout)
            code = proc.returncode
        except subprocess.TimeoutExpired:
            if os.name == 'posix':
                os.killpg(proc.pid, signal.SIGKILL)
            else:
                proc.kill()
            stdout, stderr = proc.communicate()
            stderr += f'\nCommand timed out after {timeout} seconds'
            code = None
        return dict(argv=list(map(str, argv)), code=code, stdout=stdout,
                    stderr=stderr, seconds=time.monotonic() - started)
    except OSError as error:
        return dict(argv=list(map(str, argv)), code=None, stdout='', stderr=str(error),
                    seconds=time.monotonic() - started)


def required(argv, cwd, timeout, env=None):
    result = invoke(argv, cwd, timeout, env)
    if result['code'] != 0:
        raise RuntimeError(f"{argv}: {result['stderr']}")
    return result['stdout'].strip()


def json_lines(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def usage(stdout):
    """Only use terminal response usage; never sum streaming/cumulative counters."""
    found = None
    for number, line in enumerate(stdout.splitlines()):
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if not isinstance(value, dict):
            continue
        if value.get('type') == 'result' and isinstance(value.get('usage'), dict):
            counts = value['usage']
            keys = ('input_tokens', 'output_tokens', 'cache_creation_input_tokens',
                    'cache_read_input_tokens')
        elif value.get('type') == 'turn.completed' and isinstance(value.get('usage'), dict):
            counts = value['usage']
            keys = ('input_tokens', 'output_tokens')  # cached_input_tokens is a subset.
        else:
            continue
        if all(key in counts for key in ('input_tokens', 'output_tokens')) and all(
                type(counts.get(key, 0)) is int and counts.get(key, 0) >= 0 for key in keys):
            found = {'total': sum(counts.get(key, 0) for key in keys), 'raw': counts,
                     'source': f'runner stdout JSONL line {number + 1}, {value["type"]}.usage'}
    return found


def violation_count(value):
    groups = [value[key] for key in ('new_forbidden', 'new_cycles', 'grown_cycles')]
    if not all(isinstance(group, list) for group in groups):
        raise ValueError('incomplete architecture result')
    # Grown cycles also appear in new_cycles; count each resulting SCC once.
    cycles = [*groups[1], *(item['after'] for item in groups[2])]
    unique = {(item['level'], tuple(sorted(item['members']))) for item in cycles}
    return len(groups[0]) + len(unique)


def app_server_module():
    spec = importlib.util.spec_from_file_location('boundrail_bench_codex',
                                                Path(__file__).with_name('codex_app_server.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def runner_argv(runner, prompt, context=None):
    adapter = runner.get('adapter')
    if adapter == 'codex-app-server':
        argv = [sys.executable, str(Path(__file__).with_name('codex_app_server.py').resolve()),
                '--model', '{model}', '--prompt', '{prompt}', '--mode', '{mode}', '--run-id', '{run_id}',
                '--boundrail', '{boundrail}', '--boundrail-sha256', '{boundrail_sha256}',
                '--artifact-dir', '{artifact_dir}']
        if runner.get('trust_reviewed_boundrail') is True:
            argv.append('--trust-reviewed-boundrail')
        if runner.get('auth_file'):
            argv.extend(['--auth-file', runner['auth_file']])
        if runner.get('codex'):
            argv.extend(['--codex', runner['codex']])
    elif adapter == 'claude':
        argv = ['claude', '-p', '{prompt}', '--model', '{model}', '--output-format', 'json']
    elif adapter == 'codex':
        argv = ['codex', 'exec', '--model', '{model}', '--sandbox', 'workspace-write', '--json', '{prompt}']
    else:
        argv = runner.get('argv')
    if not isinstance(argv, list) or not argv or not all(isinstance(x, str) for x in argv):
        raise ValueError('runner argv must be a nonempty string array')
    if not all(any('{' + key + '}' in arg for arg in argv) for key in ('model', 'prompt')):
        raise ValueError('runner argv must explicitly pass {model} and {prompt}')
    seeded = any('{seed}' in arg for arg in argv)
    if seeded and 'seed' not in runner:
        raise ValueError('runner argv requests seed but runner has no seed')
    values = {'model': runner['model'], 'prompt': prompt, 'seed': str(runner.get('seed', '')),
              **{key: str(value) for key, value in (context or {}).items()}}
    # Deliberate replacement leaves unrelated braces in agent prompts/argv untouched.
    return [re.sub(r'\{(model|prompt|seed|mode|run_id|boundrail|boundrail_sha256|artifact_dir)\}', lambda m: values.get(m[1], m[0]), arg) for arg in argv], seeded


def remove_configs(root):
    root = root.resolve(strict=True)
    rules = ('CLAUDE.md', 'AGENTS.md', '.cursorrules', '.github/copilot-instructions.md')
    codex = []
    for parent, directories, files in os.walk(root, followlinks=False):
        directories[:] = [name for name in directories if name not in ('.git', '.boundrail')]
        if '.codex' in directories or '.codex' in files:
            codex.append(str((Path(parent) / '.codex').relative_to(root)))
            if '.codex' in directories:
                directories.remove('.codex')
    targets = tuple(dict.fromkeys((*CONFIGS, *codex)))
    # Preflight all targets and directory contents before reading or mutating any file.
    for relative in (*rules, *targets):
        path = root
        for component in Path(relative).parts:
            path = path / component
            if path.is_symlink():
                raise ValueError(f'configuration path contains a symlink: {relative}')
        path.resolve(strict=False).relative_to(root)
        if path.is_dir():
            for parent, directories, files in os.walk(path, followlinks=False):
                if any((Path(parent) / name).is_symlink() for name in (*directories, *files)):
                    raise ValueError(f'configuration directory contains a symlink: {relative}')
    removed, updated = {}, []
    for relative in targets:
        path = root / relative
        paths = [path] if path.is_file() else sorted(path.rglob('*')) if path.is_dir() else []
        for item in paths:
            if item.is_file():
                name = str(item.relative_to(root))
                removed[name] = hashlib.sha256(item.read_bytes()).hexdigest()
    for relative in rules:
        path = root / relative
        if path.is_file():
            before = path.read_bytes()
            after = re.sub(rb'<!-- boundrail:start -->.*?<!-- boundrail:end -->', b'', before, flags=re.S)
            if before != after:
                updated.append({'path': relative, 'before_sha256': hashlib.sha256(before).hexdigest(),
                                'after_sha256': hashlib.sha256(after).hexdigest()})
                path.write_bytes(after)
    for relative in targets:
        path = root / relative
        if path.is_file():
            path.unlink()
        elif path.is_dir():
            shutil.rmtree(path)
    return {'removed_configs': [{'path': name, 'sha256': value} for name, value in sorted(removed.items())],
            'rule_blocks_updated': updated}


def stage_changes(root, base, timeout, env):
    required(['git', 'add', '-A'], root, timeout, env)
    # Negative add pathspecs fail on ignored .boundrail; restore only its index entries.
    required(['git', 'reset', '-q', base, '--', '.boundrail'], root, timeout, env)


def diff_metrics(root, base, timeout, environment=None):
    # A private index includes untracked files without changing the agent's index.
    with tempfile.TemporaryDirectory(prefix='boundrail-diff-index-') as work:
        env = dict(os.environ if environment is None else environment, GIT_INDEX_FILE=str(Path(work) / 'index'))
        required(['git', 'read-tree', base], root, timeout, env)
        stage_changes(root, base, timeout, env)
        diff = invoke(['git', 'diff', '--cached', '--no-renames', '--numstat', '-z', base], root, timeout, env)
        if diff['code'] != 0:
            raise RuntimeError('cannot measure private-index diff')
        entries = diff['stdout'].split('\0')
        files, added, removed, binary = [], 0, 0, 0
        for entry in entries:
            if not entry:
                continue
            plus, minus, name = entry.split('\t', 2)
            files.append(name)
            if plus == '-':
                binary += 1
            else:
                added += int(plus)
                removed += int(minus)
        return dict(files=files, additions=added, deletions=removed, binary_files=binary,
                    changed_lines=added + removed)


def policy_state(root):
    path = root / 'layout.yaml'
    if path.is_symlink():
        return {'kind': 'symlink', 'target': os.readlink(path)}
    if path.is_file():
        return {'kind': 'file', 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
    return {'kind': 'other' if path.exists() else 'absent'}


def redacted(value, secrets):
    if isinstance(value, dict):
        return {key: redacted(item, secrets) for key, item in value.items()}
    if isinstance(value, list):
        return [redacted(item, secrets) for item in value]
    if isinstance(value, str):
        for secret in secrets:
            if secret:
                value = value.replace(secret, '[REDACTED]')
    return value


def one_run(repo, commit, task, runner, binary, arm, repeat, timeout, artifact_dir=None, package=None):
    package_args = ['--package', package] if package else []
    result = dict(task=task['id'], arm=arm, repeat=repeat, model=runner['model'],
                  seed=runner.get('seed'), expected_area=task['area'], commands=[], errors=[],
                  hook_activation='unknown' if arm == 'guarded' else 'not_applicable')
    with tempfile.TemporaryDirectory(prefix='boundrail-paired-') as temporary:
        temporary = Path(temporary)
        root = temporary / 'repo'
        credential_names = {key for key in os.environ if any(marker in key.upper()
            for marker in ('TOKEN', 'SECRET', 'PASSWORD', 'API_KEY', 'ACCESS_KEY'))}
        credential_names.update(runner.get('env', {}))
        env = {key: value for key, value in os.environ.items() if key not in credential_names}
        env.pop('BOUNDRAIL_BYPASS', None)
        for name in ISOLATION:
            location = temporary / name.lower()
            location.mkdir(mode=0o700)
            env[name] = str(location)
        agent_env = dict(env)
        agent_env.update({key: os.environ[key] for key in credential_names if key in os.environ})
        agent_env.update(runner.get('env', {}))
        required(['git', '-c', 'core.hooksPath=/dev/null', 'clone', '--local', '--no-hardlinks',
                  '--no-checkout', str(repo), str(root)], repo, timeout, env)
        required(['git', 'config', 'core.hooksPath', '/dev/null'], root, timeout, env)
        required(['git', 'checkout', '--detach', commit], root, timeout, env)
        reviewed_policy = policy_state(root)
        result['reviewed_policy'] = reviewed_policy
        result['configuration_cleanup'] = remove_configs(root)
        # State and historical receipts must never leak into either arm.
        state = root / '.boundrail'
        if state.is_symlink() or state.is_file():
            state.unlink()
        elif state.exists():
            shutil.rmtree(state)
        result['isolation_overrides'] = [key for key in ISOLATION if key in runner.get('env', {})]
        def record(argv, agent=False):
            item = invoke(argv, root, timeout, agent_env if agent else env)
            result['commands'].append(item)
            return item
        if arm == 'guarded':
            for command in ([binary, 'hook', 'install', '--root', root, '--agent', runner['agent'], *package_args],
                            [binary, 'sync-rules']):
                setup = record(command)
                if setup['code'] != 0:
                    result['errors'].append('guarded setup failed; agent was not run')
                    break
        # Freeze setup so reported agent diff excludes installation changes.
        stage_changes(root, commit, timeout, env)
        setup_tree = required(['git', 'write-tree'], root, timeout, env)
        setup_commit = required(['git', '-c', 'user.name=Boundrail benchmark', '-c',
                                 'user.email=benchmark@example.invalid', '-c', 'commit.gpgsign=false',
                                 'commit-tree', setup_tree, '-p', commit, '-m', 'benchmark setup'], root, timeout, env)
        if not result['errors']:
            context = {'mode': arm, 'run_id': uuid.uuid4().hex, 'boundrail': binary,
                       'boundrail_sha256': hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                       'artifact_dir': str(artifact_dir)}
            expected_config = (hashlib.sha256((root / '.codex/hooks.json').read_bytes()).hexdigest()
                if arm == 'guarded' and (root / '.codex/hooks.json').is_file() else 'absent')
            argv, seeded = runner_argv(runner, task['prompt'], context)
            result['seed_supported'] = seeded
            agent = record(argv, agent=True)
            result['agent_exit'] = agent['code']
            result['wall_seconds'] = agent['seconds']
            result['tokens'] = usage(agent['stdout'])
            if runner.get('adapter') == 'codex-app-server' and agent['code'] == 0:
                try:
                    receipt = json.loads(agent['stdout'])
                    expected = {key: context[key] for key in ('run_id', 'mode', 'boundrail_sha256', 'boundrail')}
                    expected.update(model=runner['model'], cwd=str(root.resolve()),
                                    hook_config_sha256=expected_config)
                    result['hook_activation'] = app_server_module().validate_result(receipt, expected)
                    result['activation_evidence'] = receipt
                    totals = receipt.get('usage')
                    if isinstance(totals, dict) and type(totals.get('totalTokens')) is int and totals['totalTokens'] >= 0:
                        result['tokens'] = {'total': totals['totalTokens'], 'raw': totals,
                                            'source': 'Codex thread/tokenUsage/updated total'}
                except (ValueError, KeyError, TypeError) as error:
                    result['errors'].append(str(error))
            if agent['code'] != 0:
                result['errors'].append('agent command failed')
        if runner.get('adapter') == 'codex-app-server':
            auth_paths = {Path(values['CODEX_HOME']) / 'auth.json' for values in (env, agent_env)}
            if any(path.exists() or path.is_symlink() for path in auth_paths):
                result['errors'].append('auth cleanup failed; evaluators and tests skipped')
                result['comparison_eligible'] = False
                return redacted(result, runner.get('env', {}).values())
        result['policy_changed'] = policy_state(root) != reviewed_policy
        if result['policy_changed']:
            result['errors'].append('reviewed layout.yaml changed; run excluded from efficacy comparison')
        result['comparison_eligible'] = (not result['policy_changed'] and 'agent_exit' in result
            and ((arm == 'bare' and (runner.get('adapter') != 'codex-app-server'
                                      or result['hook_activation'] == 'verified_absent'))
                 or result['hook_activation'] == 'verified_active'))
        # Installation alone does not prove native hook trust/activation in a fresh profile.
        # Always check actual final sources, including after failed/partial agent runs.
        check = record([binary, '--no-daemon', 'mcp-call', '.', '--engine', 'native', '--symbols', *package_args,
                        '--tool', 'check_diff', '--arguments', json.dumps({'base': commit, 'brief': False})])
        try:
            if check['code'] != 0:
                raise ValueError('final architecture command failed')
            check_data = json.loads(check['stdout'])
            result['new_violations'] = None if result['policy_changed'] else violation_count(check_data)
            result['architecture'] = check_data
        except (ValueError, KeyError, TypeError) as error:
            result['new_violations'] = None
            result['errors'].append(str(error))
        try:
            fires = json_lines(root / '.boundrail/fires.log')
            if not all(isinstance(x, dict) and all(k in x for k in ('rule', 'file', 'agent', 'ts')) for x in fires):
                raise ValueError('malformed fires log')
            result['edit_blocks'] = len(fires)
            result['distinct_blocked_findings'] = len({x['finding_id'] for x in fires if 'finding_id' in x})
            result['fires'] = fires
        except (ValueError, KeyError, TypeError, OSError) as error:
            result['edit_blocks'] = None
            result['errors'].append(str(error))
        # File presence is not proof of a valid Rust Receipt or an automatically applied fix.
        result['raw_receipt_files'] = sorted(path.name for path in (root / '.boundrail/applied').glob('*.json'))
        result['raw_receipt_file_count'] = len(result['raw_receipt_files'])
        result['recipes_applied'] = 0
        result['verified_recipe_receipts'] = []
        for name in result['raw_receipt_files']:
            finding = name[:-5]
            verify = record([binary, '--no-daemon', 'verify', finding, '--root', root,
                             '--engine', 'native', '--symbols', *package_args])
            try:
                proof = json.loads(verify['stdout'])
                if (verify['code'] != 0 or proof.get('kind') != 'verification'
                        or proof.get('finding_id') != finding or proof.get('verify', {}).get('pass') is not True):
                    raise ValueError('receipt failed Rust schema/current-source verification')
                result['verified_recipe_receipts'].append(finding)
            except (ValueError, TypeError, AttributeError):
                result['recipes_applied'] = None
                result['errors'].append('invalid or unverified apply receipt; application count unknown')
        if result['recipes_applied'] is not None:
            result['recipes_applied'] = len(result['verified_recipe_receipts'])
        try:
            result['diff'] = diff_metrics(root, setup_commit, timeout, env)
            result['outside_expected_area'] = [name for name in result['diff']['files']
                if not any(fnmatch.fnmatchcase(name, area) for area in task['area'])]
            result['expected_area_touched'] = any(name not in result['outside_expected_area']
                                                  for name in result['diff']['files'])
            if not result['expected_area_touched']:
                result['comparison_eligible'] = False
                result['errors'].append('expected touched area unchanged; feature work unverified')
        except (ValueError, RuntimeError) as error:
            result['errors'].append(str(error))
        tests = record(task['tests'])
        result['tests_pass'] = tests['code'] == 0
        if not result['tests_pass']:
            result['errors'].append('project tests failed')
    secrets = [runner.get('auth_file', ''), *runner.get('env', {}).values(),
               *(os.environ[key] for key in credential_names if key in os.environ)]
    return redacted(result, secrets)


def summaries(runs):
    result = {}
    for arm in ('bare', 'guarded'):
        rows = [row for row in runs if row['arm'] == arm]
        result[arm] = {'runs': len(rows), 'agent_runs': sum('agent_exit' in row for row in rows),
                       'failed_runs': sum(bool(row['errors']) for row in rows),
                       'comparison_eligible': sum(row.get('comparison_eligible', False) for row in rows)}
        for key in ('new_violations', 'edit_blocks', 'recipes_applied', 'wall_seconds'):
            samples = [row[key] for row in rows if row.get('comparison_eligible') and isinstance(row.get(key), (int, float))]
            result[arm][key] = dict(measured=len(samples), mean=statistics.mean(samples) if samples else None,
                stdev=statistics.stdev(samples) if len(samples) > 1 else None)
    return result


def render(data, path):
    raw = path.with_suffix('.json')
    raw.write_text(json.dumps(data, indent=2) + '\n')
    chart = path.with_suffix('.svg')
    rows = data['runs']
    scale = max([row.get('new_violations') or 0 for row in rows] + [1])
    height = 50 + len(rows) * 24
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="800" height="{height}" role="img" aria-label="Final new graph violations for every run">',
             '<rect width="100%" height="100%" fill="white"/>',
             '<text x="10" y="20" font-family="sans-serif">Final new graph violations (unknown shown as ?)</text>']
    for i, row in enumerate(rows):
        y = 45 + i * 24
        label = html.escape(f'{row["task"]} / {row["arm"]} / {row["repeat"]}')
        value = row.get('new_violations')
        width = 400 * (value or 0) / scale
        color = '#4979b7' if row['arm'] == 'bare' else '#20906b'
        parts += [f'<text x="10" y="{y}" font-size="12" font-family="sans-serif">{label}</text>',
                  f'<rect x="300" y="{y - 12}" width="{width}" height="16" fill="{color}"/>',
                  f'<text x="{310 + width}" y="{y}" font-size="12">{value if value is not None else "?"}</text>']
    chart.write_text('\n'.join(parts + ['</svg>']) + '\n')
    lines = [f'# {data["repo_name"]}: paired agent benchmark', '',
             f'Commit `{data["commit"]}`; model `{data["runner"]["model"]}`; {data["repeats"]} repetitions.',
             f'Architecture package: `{data.get("package") or "automatic selection"}`.', '',
             'All runs are included. Unknown measurements are `?`; failed runs remain visible.', '',
             '| Task | Arm | Run | New violations | Edit blocks | Applied recipes | Raw receipt files | Hook activation | Eligible | Expected area touched | Tests | Tokens | Agent seconds | Diff lines | Errors |',
             '|---|---|---:|---:|---:|---:|---:|---|---|---|---|---:|---:|---:|---|']
    def cell(value):
        return '?' if value is None else str(value).replace('|', '\\|').replace('\n', ' ')
    for row in rows:
        fields = [row['task'], row['arm'], row['repeat'], row.get('new_violations'), row.get('edit_blocks'),
                  row.get('recipes_applied'), row.get('raw_receipt_file_count'), row.get('hook_activation'),
                  row.get('comparison_eligible', False), row.get('expected_area_touched'), row.get('tests_pass'), (row.get('tokens') or {}).get('total'),
                  round(row['wall_seconds'], 3) if 'wall_seconds' in row else None,
                  row.get('diff', {}).get('changed_lines'), '; '.join(row['errors']) or 'none']
        lines.append('| ' + ' | '.join(map(cell, fields)) + ' |')
    lines += ['', f'![Final graph violations]({chart.name})', '', 'Means ± sample standard deviation (comparison-eligible runs only; all other runs remain in the table):', '']
    for arm, summary in data['summary'].items():
        metrics = []
        for key in ('new_violations', 'edit_blocks', 'recipes_applied', 'wall_seconds'):
            item = summary[key]
            metrics.append(f'{key}: {cell(item["mean"])} ± {cell(item["stdev"])} (n={item["measured"]})')
        lines.append(f'- {arm}: ' + '; '.join(metrics) + f'; failed runs={summary["failed_runs"]}.')
    lines += ['', 'Repeat variance by task (not pooled across different tasks):', '',
              '| Task | Arm | Measured | Mean new violations | Sample SD |',
              '|---|---|---:|---:|---:|']
    for task, summary in data['per_task_summary'].items():
        for arm, metrics in summary.items():
            item = metrics['new_violations']
            lines.append('| ' + ' | '.join(map(cell, (task, arm, item['measured'], item['mean'], item['stdev']))) + ' |')
    lines += ['', f'[Raw measurements, command output and provenance]({raw.name}).', '',
              'Guarded activation requires the reviewed app-server protocol and correlated native notifications; installation is insufficient. Unknown activation and policy mutations are excluded from efficacy means. Tokens are nullable terminal usage counters; applied recipes require Rust receipt/current-source verification. '
              'Graph counts are new forbidden edges plus new/grown cycles against the original commit. '
              'Synthetic/mock results establish harness behavior only. Isolation overrides and missing '
              'seed support are recorded per run. Review the raw command output to verify host hook trust.', '']
    path.write_text('\n'.join(lines))


def benchmark(repo, tasks, runner, binary, output, repeats=3, timeout=900, package=None):
    repo, binary = Path(repo).resolve(), Path(binary).resolve()
    if not binary.is_file():
        raise ValueError('boundrail must be an existing executable file')
    if not isinstance(runner.get('model'), str) or not runner['model'] or runner.get('agent') not in ('claude', 'codex'):
        raise ValueError('runner requires exact model and agent claude|codex')
    if not isinstance(runner.get('env', {}), dict) or not all(
            isinstance(key, str) and isinstance(value, str) for key, value in runner.get('env', {}).items()):
        raise ValueError('runner env must be a string mapping')
    if runner.get('env', {}).get('BOUNDRAIL_BYPASS'):
        raise ValueError('benchmark runner cannot bypass architecture hooks')
    runner_argv(runner, 'validation')
    if not isinstance(tasks, list) or not tasks or repeats < 1:
        raise ValueError('provide tasks and at least one repetition')
    ids = set()
    for task in tasks:
        if not re.fullmatch(r'[A-Za-z0-9_.-]+', task.get('id', '')) or task['id'] in ids:
            raise ValueError('task IDs must be unique safe names')
        ids.add(task['id'])
        if not isinstance(task.get('prompt'), str) or not task['prompt'].strip():
            raise ValueError('task prompt required')
        for key in ('area', 'tests'):
            if not isinstance(task.get(key), list) or not task[key] or not all(isinstance(x, str) and x for x in task[key]):
                raise ValueError(f'task {key} must be a nonempty string array')
    if required(['git', 'status', '--porcelain', '--untracked-files=all'], repo, timeout):
        raise ValueError('source repository must be clean, including untracked files')
    commit = required(['git', 'rev-parse', 'HEAD'], repo, timeout)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    name = re.sub(r'[^A-Za-z0-9_.-]', '_', repo.name)
    path = output / f'{name}-{datetime.now(timezone.utc).date()}.md'
    if any(path.with_suffix(suffix).exists() for suffix in ('.md', '.json', '.svg')):
        raise ValueError('report already exists; choose a new output directory to preserve evidence')
    data = dict(schema='boundrail/paired-benchmark/v1', repo_name=repo.name, commit=commit, package=package,
                created=datetime.now(timezone.utc).isoformat(), repeats=repeats,
                binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                runner=redacted({key: value for key, value in runner.items() if key not in ('env', 'auth_file')},
                                [runner.get('auth_file', ''), *runner.get('env', {}).values()]),
                auth_file_supplied=bool(runner.get('auth_file')),
                runner_env_keys=sorted(runner.get('env', {})), tasks=tasks, runs=[],
                host={'system': platform.system(), 'machine': platform.machine(),
                      'python': platform.python_version()},
                boundrail_version=invoke([str(binary), '--version'], repo, timeout))
    for task in tasks:
        for repeat in range(1, repeats + 1):
            # Alternate order to reduce a systematic first-arm cache/time effect.
            arms = ('bare', 'guarded') if repeat % 2 else ('guarded', 'bare')
            for arm in arms:
                try:
                    artifact_dir = output / '.private-events' / uuid.uuid4().hex
                    artifact_dir.parent.mkdir(mode=0o700, exist_ok=True)
                    if artifact_dir.parent.is_symlink() or artifact_dir.parent.stat().st_mode & 0o077:
                        raise ValueError('private event root must not be a symlink or publicly accessible')
                    artifact_dir.mkdir(mode=0o700)
                    row = one_run(repo, commit, task, runner, str(binary), arm, repeat, timeout, artifact_dir.resolve(), package)
                except (RuntimeError, ValueError, OSError) as error:
                    row = dict(task=task['id'], arm=arm, repeat=repeat, errors=[str(error)])
                row = redacted(row, [runner.get('auth_file', ''), *runner.get('env', {}).values()])
                data['runs'].append(row)
                data['summary'] = summaries(data['runs'])
                data['per_task_summary'] = {task['id']: summaries([row for row in data['runs']
                    if row['task'] == task['id']]) for task in tasks}
                render(data, path)
    return path, data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('repo', 'tasks', 'runner', 'boundrail'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--output', default=str(Path(__file__).parent / 'results'))
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('--package', help='Explicit architecture package for monorepos')
    args = parser.parse_args()
    path, data = benchmark(args.repo, json.loads(Path(args.tasks).read_text()),
                           json.loads(Path(args.runner).read_text()), args.boundrail,
                           args.output, args.repeats, args.timeout, args.package)
    print(path)
    return int(any(row['errors'] for row in data['runs']))


if __name__ == '__main__':
    raise SystemExit(main())
