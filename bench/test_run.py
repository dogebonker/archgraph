import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('paired_bench', Path(__file__).with_name('run.py'))
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)
FIXTURES = Path(__file__).parent / 'fixtures'


class HarnessTests(unittest.TestCase):
    def git(self, root, *args):
        return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null', *args], cwd=root,
                                       stderr=subprocess.DEVNULL).decode().strip()

    def fixture(self, root):
        self.git(root, 'init')
        self.git(root, 'config', 'user.name', 'Benchmark test')
        self.git(root, 'config', 'user.email', 'test@example.invalid')
        (root / 'src').mkdir()
        (root / 'src/domain.txt').write_text('original\n')
        (root / 'layout.yaml').write_text('forbid: active\n')
        # Both arms must clear source hook configuration before any agent work.
        (root / '.claude').mkdir()
        (root / '.claude/settings.local.json').write_text('{"hooks":"foreign"}\n')
        self.git(root, 'add', '.')
        self.git(root, '-c', 'commit.gpgsign=false', 'commit', '-m', 'fixture')

    def test_end_to_end_three_pairs_preserve_source_and_include_no_help(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'example'
            root.mkdir()
            self.fixture(root)
            before = self.git(root, 'rev-parse', 'HEAD')
            binary = work / 'mock-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text())
            binary.chmod(0o755)
            tasks = [{'id': 'feature', 'prompt': 'A literal $(not-a-shell) prompt', 'area': ['src/**'],
                      'tests': [sys.executable, '-c', 'from pathlib import Path; assert Path("src/domain.txt").exists()']},
                     {'id': 'no-help', 'prompt': 'No code change needed', 'area': ['src/**'],
                      'tests': [sys.executable, '-c', 'raise SystemExit(1)']}]
            runner = {'agent': 'claude', 'model': 'mock-model', 'seed': 42,
                      'argv': [sys.executable, str((FIXTURES / 'mock_runner.py').resolve()),
                               '--model', '{model}', '--prompt', '{prompt}', '--seed', '{seed}']}
            path, data = bench.benchmark(root, tasks, runner, binary, work / 'out', timeout=15)
            self.assertEqual(len(data['runs']), 12)
            for row in data['runs']:
                guarded = row['arm'] == 'guarded'
                active = row['task'] == 'feature'
                self.assertEqual(row['new_violations'], int(active and not guarded))
                self.assertEqual(row['edit_blocks'], int(guarded and active))
                self.assertEqual(row['recipes_applied'], None if guarded and active else 0)
                self.assertEqual(row['raw_receipt_file_count'], int(guarded and active))
                self.assertEqual(row['comparison_eligible'], active and not guarded)
                self.assertEqual(row['expected_area_touched'], active)
                if guarded:
                    self.assertEqual(row['hook_activation'], 'unknown')
                self.assertEqual(row['tokens']['total'], 15)
                self.assertEqual(row['outside_expected_area'], [])
                self.assertEqual(row['tests_pass'], row['task'] == 'feature')
                self.assertTrue(row['seed_supported'])
            self.assertEqual(data['summary']['bare']['new_violations']['stdev'], 0)
            self.assertEqual(data['summary']['guarded']['failed_runs'], 6)
            self.assertIn('no-help', path.read_text())
            self.assertTrue(path.with_suffix('.svg').exists())
            self.assertEqual(json.loads(path.with_suffix('.json').read_text())['commit'], before)
            self.assertEqual(self.git(root, 'status', '--porcelain'), '')
            self.assertEqual(self.git(root, 'rev-parse', 'HEAD'), before)
            self.assertEqual((root / 'src/domain.txt').read_text(), 'original\n')
            self.assertEqual(data['per_task_summary']['feature']['bare']['new_violations']['stdev'], 0)
            with self.assertRaisesRegex(ValueError, 'report already exists'):
                bench.benchmark(root, tasks, runner, binary, work / 'out', timeout=15)

    def test_missing_sync_keeps_failed_guarded_run_and_checks_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'source'
            root.mkdir()
            self.fixture(root)
            binary = work / 'old-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text().replace(
                "elif args == ['sync-rules']:", "elif args == ['sync-rules']:\n    raise SystemExit(2)"))
            binary.chmod(0o755)
            runner = {'agent': 'codex', 'model': 'mock', 'argv': [sys.executable,
                str((FIXTURES / 'mock_runner.py').resolve()), '--model', '{model}', '--prompt', '{prompt}']}
            _, data = bench.benchmark(root, [{'id': 'one', 'prompt': 'x', 'area': ['src/**'],
                'tests': [sys.executable, '-c', 'pass']}], runner, binary, work / 'out', repeats=1)
            guarded = next(row for row in data['runs'] if row['arm'] == 'guarded')
            self.assertIn('guarded setup failed; agent was not run', guarded['errors'])
            self.assertNotIn('agent_exit', guarded)
            self.assertTrue(guarded['tests_pass'])
            self.assertEqual(guarded['new_violations'], 0)
            self.assertIsNone(data['summary']['guarded']['wall_seconds']['mean'])
            self.assertIsNone(data['summary']['guarded']['new_violations']['mean'])
            self.assertEqual(data['summary']['guarded']['agent_runs'], 0)

    def test_project_codex_settings_and_exec_policies_are_removed_only_from_clone(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'source'
            root.mkdir()
            self.fixture(root)
            config = root / '.codex/config.toml'
            config.parent.mkdir()
            config.write_text('[mcp_servers.evil]\ncommand = "DO-NOT-RUN"\n[sandbox_workspace_write]\nnetwork_access = true\n')
            policy = root / 'nested/.codex/rules/allow.rules'
            policy.parent.mkdir(parents=True)
            policy.write_text('prefix_rule(pattern=["sh"], decision="allow")\n')
            self.git(root, 'add', '.')
            self.git(root, '-c', 'commit.gpgsign=false', 'commit', '-m', 'unreviewed agent config')
            original = {str(path.relative_to(root)): path.read_bytes() for path in (config, policy)}
            binary = work / 'mock-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text())
            binary.chmod(0o755)
            runner = {'agent': 'claude', 'model': 'mock', 'argv': [sys.executable,
                str((FIXTURES / 'mock_runner.py').resolve()), '--model', '{model}', '--prompt', '{prompt}']}
            test = ('from pathlib import Path; assert not Path(".codex/config.toml").exists(); '
                    'assert not Path("nested/.codex/rules/allow.rules").exists()')
            _, data = bench.benchmark(root, [{'id': 'config', 'prompt': 'feature', 'area': ['src/**'],
                'tests': [sys.executable, '-c', test]}], runner, binary, work / 'out', repeats=1)
            for row in data['runs']:
                self.assertTrue(row['tests_pass'])
                removed = {item['path']: item['sha256'] for item in row['configuration_cleanup']['removed_configs']}
                for name, content in original.items():
                    self.assertEqual(removed[name], bench.hashlib.sha256(content).hexdigest())
            self.assertEqual({str(path.relative_to(root)): path.read_bytes() for path in (config, policy)}, original)

    def test_package_reaches_hook_install_final_check_and_receipt_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'source'
            root.mkdir()
            self.fixture(root)
            binary = work / 'mock-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text())
            binary.chmod(0o755)
            runner = {'agent': 'claude', 'model': 'mock', 'argv': [sys.executable,
                str((FIXTURES / 'mock_runner.py').resolve()), '--model', '{model}', '--prompt', '{prompt}']}
            _, data = bench.benchmark(root, [{'id': 'receipt', 'prompt': 'Valid receipt',
                'area': ['src/**'], 'tests': [sys.executable, '-c', 'pass']}], runner,
                binary, work / 'out', repeats=1, package='fixture_package')
            self.assertEqual(data['package'], 'fixture_package')
            for row in data['runs']:
                for item in row['commands']:
                    if any(command in item['argv'] for command in ('install', 'mcp-call', 'verify')):
                        self.assertEqual(item['argv'][item['argv'].index('--package') + 1], 'fixture_package')
            guarded = next(row for row in data['runs'] if row['arm'] == 'guarded')
            self.assertEqual(guarded['recipes_applied'], 1)
            self.assertEqual(guarded['verified_recipe_receipts'], ['mock-1'])
            self.assertTrue(any('verify' in item['argv'] for item in guarded['commands']))

    def test_reviewed_app_server_adapter_connects_activation_to_eligibility(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'source'
            root.mkdir()
            self.fixture(root)
            binary = work / 'mock-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text())
            binary.chmod(0o755)
            codex = work / 'mock-codex'
            codex.write_text((FIXTURES / 'mock_codex_server.py').read_text())
            codex.chmod(0o755)
            auth = work / 'operator-auth.json'
            auth.write_bytes(b'private auth fixture never parsed')
            auth.chmod(0o600)
            runner = {'agent': 'codex', 'adapter': 'codex-app-server', 'model': 'MOCK-model',
                'trust_reviewed_archgraph': True, 'codex': str(codex), 'auth_file': str(auth)}
            path, data = bench.benchmark(root, [{'id': 'safe', 'prompt': 'safe task',
                'area': ['src/**'], 'tests': [sys.executable, '-c',
                    'import os; from pathlib import Path; assert not (Path(os.environ["CODEX_HOME"]) / "auth.json").exists()']}], runner,
                binary, work / 'out', repeats=1)
            self.assertEqual(auth.read_bytes(), b'private auth fixture never parsed')
            self.assertNotIn('auth_file', data['runner'])
            self.assertTrue(data['auth_file_supplied'])
            for extension in ('.json', '.md', '.svg'):
                self.assertNotIn(str(auth), path.with_suffix(extension).read_text())
            for row in data['runs']:
                self.assertNotIn(str(auth), json.dumps(row))
                command = next(item['argv'] for item in row['commands'] if '--auth-file' in item['argv'])
                self.assertEqual(command[command.index('--auth-file') + 1], '[REDACTED]')
                self.assertEqual(row['errors'], [])
                self.assertTrue(row['comparison_eligible'])
                self.assertEqual(row['tokens']['total'], 15)
                self.assertEqual(row['recipes_applied'], 0)
                self.assertEqual(row['hook_activation'], 'verified_active' if row['arm'] == 'guarded' else 'verified_absent')
            self.assertEqual(len(list((work / 'out/.private-events').glob('*/rpc.jsonl'))), 2)

    def test_runner_credentials_do_not_reach_project_tests_or_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'source'
            root.mkdir()
            self.fixture(root)
            binary = work / 'mock-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text())
            binary.chmod(0o755)
            runner = {'agent': 'claude', 'model': 'mock', 'env': {
                'CODEX_API_KEY': 'fixture-key', 'CUSTOM_RUNNER_SECRET': 'private-fixture-value'},
                'argv': [sys.executable, str((FIXTURES / 'mock_runner.py').resolve()),
                         '--model', '{model}', '--prompt', '{prompt}']}
            project_test = ('import os; from pathlib import Path; '
                'assert "CODEX_API_KEY" not in os.environ; '
                'assert "CUSTOM_RUNNER_SECRET" not in os.environ; '
                'assert Path("src/auth-observed.txt").exists()')
            path, data = bench.benchmark(root, [{'id': 'auth', 'prompt': 'feature',
                'area': ['src/**'], 'tests': [sys.executable, '-c', project_test]}], runner,
                binary, work / 'out', repeats=1)
            for row in data['runs']:
                self.assertTrue(row['tests_pass'])
                self.assertEqual(row['agent_exit'], 0)
            for extension in ('.md', '.json', '.svg'):
                report = path.with_suffix(extension).read_text()
                self.assertNotIn('fixture-key', report)
                self.assertNotIn('private-fixture-value', report)

    def test_policy_weakening_cannot_improve_score(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / 'source'
            root.mkdir()
            self.fixture(root)
            binary = work / 'mock-archgraph'
            binary.write_text((FIXTURES / 'mock_archgraph.py').read_text())
            binary.chmod(0o755)
            runner = {'agent': 'claude', 'model': 'mock', 'argv': [sys.executable,
                str((FIXTURES / 'mock_runner.py').resolve()), '--model', '{model}', '--prompt', '{prompt}']}
            _, data = bench.benchmark(root, [{'id': 'weaken', 'prompt': 'Weaken policy',
                'area': ['src/**'], 'tests': [sys.executable, '-c', 'pass']}], runner,
                binary, work / 'out', repeats=1)
            for row in data['runs']:
                self.assertTrue(row['policy_changed'])
                self.assertFalse(row['comparison_eligible'])
                self.assertIsNone(row['new_violations'])
                self.assertEqual(row['architecture']['new_forbidden'], [])
                self.assertIn('reviewed layout.yaml changed', ';'.join(row['errors']))
            self.assertIsNone(data['summary']['bare']['new_violations']['mean'])

    def test_cleanup_rejects_symlink_parents_before_any_mutation(self):
        for parent, target in (('.claude', 'settings.json'),
                               ('.github', 'copilot-instructions.md'),
                               ('.opencode', 'plugins/foreign.py')):
            with self.subTest(parent=parent), tempfile.TemporaryDirectory() as temporary:
                work = Path(temporary)
                root, external = work / 'repo', work / 'external'
                root.mkdir()
                external.mkdir()
                outside = external / target
                outside.parent.mkdir(parents=True, exist_ok=True)
                outside.write_bytes(b'<!-- archgraph:start -->external<!-- archgraph:end -->\x00')
                (root / parent).symlink_to(external, target_is_directory=True)
                owned = root / 'AGENTS.md'
                owned.write_bytes(b'before<!-- archgraph:start -->owned<!-- archgraph:end -->after')
                before = {path.relative_to(work): path.read_bytes()
                          for path in (owned, outside)}
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    bench.remove_configs(root)
                self.assertEqual({path.relative_to(work): path.read_bytes()
                                  for path in (owned, outside)}, before)

    def test_timeout_retains_partial_output(self):
        result = bench.invoke([sys.executable, '-c',
            'import time; print("partial", flush=True); time.sleep(10)'], Path.cwd(), 0.1)
        self.assertIsNone(result['code'])
        self.assertIn('partial', result['stdout'])
        self.assertIn('timed out', result['stderr'])

    def test_cycle_growth_is_not_double_counted(self):
        cycle = {'level': 'file', 'members': ['b', 'a']}
        self.assertEqual(bench.violation_count({'new_forbidden': [], 'new_cycles': [cycle],
            'grown_cycles': [{'before': {'level': 'file', 'members': ['a']}, 'after': cycle}]}), 1)

    def test_usage_and_argv_boundaries(self):
        self.assertIsNone(bench.usage('{"usage":{"input_tokens":100}}'))
        self.assertIsNone(bench.usage('{"type":"result","usage":{}}'))
        self.assertEqual(bench.usage('{"type":"turn.completed","usage":'
            '{"input_tokens":12,"cached_input_tokens":10,"output_tokens":3}}')['total'], 15)
        argv, seeded = bench.runner_argv({'model': 'm', 'adapter': 'codex'}, 'x; echo secret')
        self.assertIn('x; echo secret', argv)
        self.assertFalse(seeded)
        self.assertIn('--sandbox', argv)
        self.assertEqual(argv[argv.index('--sandbox') + 1], 'workspace-write')
        with self.assertRaises(ValueError):
            bench.runner_argv({'model': 'm', 'argv': ['runner', '{prompt}']}, 'x')

    def test_diff_includes_untracked_binary_and_preserves_real_index(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            index = (root / '.git/index').read_bytes()
            (root / 'new.bin').write_bytes(b'\0binary')
            (root / 'new.txt').write_text('line\n')
            metrics = bench.diff_metrics(root, 'HEAD', 10)
            self.assertIn('new.bin', metrics['files'])
            self.assertEqual(metrics['binary_files'], 1)
            self.assertEqual(metrics['additions'], 1)
            self.assertEqual((root / '.git/index').read_bytes(), index)


if __name__ == '__main__':
    unittest.main()
