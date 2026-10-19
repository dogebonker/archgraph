import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('codex_bench_runner', Path(__file__).with_name('codex_app_server.py'))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)
FIXTURE = Path(__file__).parent / 'fixtures/mock_codex_server.py'


class AppServerTests(unittest.TestCase):
    def setup(self, work, mode='guarded'):
        root, profile, artifact = (work / name for name in ('repo', 'profile', 'artifacts'))
        for path in (root, profile, artifact):
            path.mkdir(mode=0o700)
        binary = work / 'archgraph binary'
        binary.write_bytes(b'mock architecture binary')
        if mode == 'guarded':
            (root / '.codex').mkdir()
            (root / '.codex/hooks.json').write_text('{"hooks":"fixture"}')
        args = argparse.Namespace(archgraph=str(binary), archgraph_sha256=runner.digest(binary),
            artifact_dir=str(artifact), auth_file=None, mode=mode, trust_reviewed_archgraph=True,
            codex='UNUSED-no-real-codex', model='MOCK-model', prompt='MOCK task', run_id='fixture-run', timeout=10)
        return root, profile, artifact, binary, args

    def execute(self, root, profile, binary, args, extra=None):
        before = Path.cwd()
        with patch.dict(os.environ, {'CODEX_HOME': str(profile), 'MOCK_ARCHGRAPH_BINARY': str(binary), **(extra or {})}):
            try:
                os.chdir(root)
                return runner.run(args, command=[sys.executable, str(FIXTURE.resolve())])
            finally:
                os.chdir(before)

    def test_exact_trust_correlated_hooks_usage_and_private_rpc_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, profile, artifact, binary, args = self.setup(Path(temporary))
            value = self.execute(root, profile, binary, args)
            self.assertEqual(value['activation'], 'verified_active')
            self.assertEqual(value['usage']['totalTokens'], 15)
            expected = {'run_id': args.run_id, 'mode': args.mode, 'model': args.model,
                'cwd': str(root.resolve()), 'archgraph': str(binary.resolve()),
                'archgraph_sha256': args.archgraph_sha256, 'hook_config_sha256': runner.digest(root / '.codex/hooks.json')}
            self.assertEqual(runner.validate_result(value, expected), 'verified_active')
            edits = [json.loads(line) for line in (profile / 'mock-config-edits.jsonl').read_text().splitlines()]
            self.assertEqual([edit['edits'][0]['keyPath'] for edit in edits], ['features.hooks', 'projects', 'hooks.state'])
            self.assertEqual((artifact / 'rpc.jsonl').stat().st_mode & 0o077, 0)
            self.assertEqual(len(value['hook_events']), 6)
            value['hook_events'][0]['threadId'] = 'wrong-thread'
            with self.assertRaises(ValueError):
                runner.validate_result(value, expected)

    def test_bare_proves_absence_and_never_writes_hook_trust(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, profile, artifact, binary, args = self.setup(Path(temporary), 'bare')
            value = self.execute(root, profile, binary, args)
            self.assertEqual(value['activation'], 'verified_absent')
            self.assertEqual(value['hook_events'], [])
            self.assertNotIn('hooks.state', (profile / 'mock-config-edits.jsonl').read_text())

    def test_command_tampering_and_missing_opt_in_never_start_turn(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, profile, artifact, binary, args = self.setup(Path(temporary))
            with self.assertRaisesRegex(ValueError, 'exact reviewed'):
                self.execute(root, profile, binary, args, {'MOCK_TAMPER_COMMAND': '1'})
            raw = (artifact / 'rpc.jsonl').read_text()
            self.assertNotIn('turn/start', raw)
            self.assertNotIn('hooks.state', raw)
        with tempfile.TemporaryDirectory() as temporary:
            root, profile, artifact, binary, args = self.setup(Path(temporary))
            args.trust_reviewed_archgraph = False
            with self.assertRaisesRegex(ValueError, 'explicit'):
                self.execute(root, profile, binary, args)
            self.assertEqual(list(artifact.iterdir()), [])

    def test_auth_is_metadata_checked_and_symlinked_without_reading_tokens(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root, profile, artifact, binary, args = self.setup(work)
            source = work / 'operator-auth.json'
            source.write_bytes(b'not parsed, not copied, not emitted')
            source.chmod(0o600)
            with patch.object(Path, 'read_bytes', side_effect=AssertionError('must not read auth contents')):
                runner.link_auth(profile, source)
            self.assertTrue((profile / 'auth.json').is_symlink())
            self.assertEqual(os.readlink(profile / 'auth.json'), str(source.resolve()))
            self.assertEqual(source.read_bytes(), b'not parsed, not copied, not emitted')
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            profile = work / 'profile'
            profile.mkdir(mode=0o700)
            source = work / 'public-auth.json'
            source.write_text('no')
            source.chmod(0o644)
            with self.assertRaisesRegex(ValueError, 'private owned'):
                runner.link_auth(profile, source)
            self.assertFalse((profile / 'auth.json').exists())

    def test_effective_settings_and_mcp_mismatches_never_start_feature_turn(self):
        for mismatch in ('provider', 'approval', 'sandbox', 'mcp'):
            with self.subTest(mismatch=mismatch), tempfile.TemporaryDirectory() as temporary:
                root, profile, artifact, binary, args = self.setup(Path(temporary))
                with self.assertRaises(ValueError):
                    self.execute(root, profile, binary, args, {'MOCK_BAD_RESPONSE': mismatch})
                self.assertNotIn('turn/start', (artifact / 'rpc.jsonl').read_text())
                edits = (profile / 'mock-config-edits.jsonl').read_text()
                self.assertNotIn(str(Path(temporary) / 'outside-config'), edits)

    def test_project_codex_config_is_rejected_before_auth_or_rpc(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root, profile, artifact, binary, args = self.setup(work)
            config = root / '.codex/config.toml'
            config.write_text('[mcp_servers.unreviewed]\ncommand = "do-not-run"\n')
            with self.assertRaisesRegex(ValueError, 'removed before trust'):
                self.execute(root, profile, binary, args)
            self.assertEqual(list(profile.iterdir()), [])
            self.assertEqual(list(artifact.iterdir()), [])
            self.assertIn('do-not-run', config.read_text())

    def test_auth_cleanup_after_success_setup_failure_turn_failure_and_sdk_replacement(self):
        for scenario in ('success', 'setup', 'turn', 'replacement'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temporary:
                work = Path(temporary)
                root, profile, artifact, binary, args = self.setup(work)
                source = work / 'source-auth.json'
                source.write_bytes(b'private unchanged fixture auth')
                source.chmod(0o600)
                args.auth_file = str(source)
                extra = ({'MOCK_TAMPER_COMMAND': '1'} if scenario == 'setup' else
                         {'MOCK_BAD_RESPONSE': 'turn'} if scenario == 'turn' else
                         {'MOCK_REPLACE_AUTH': '1'} if scenario == 'replacement' else {})
                if scenario in ('setup', 'turn'):
                    with self.assertRaises(ValueError):
                        self.execute(root, profile, binary, args, extra)
                else:
                    self.execute(root, profile, binary, args, extra)
                self.assertFalse((profile / 'auth.json').exists())
                self.assertFalse((profile / 'auth.json').is_symlink())
                self.assertEqual(source.read_bytes(), b'private unchanged fixture auth')

    def test_unreviewed_binary_and_public_profile_fail_before_rpc(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, profile, artifact, binary, args = self.setup(Path(temporary))
            args.archgraph_sha256 = '0' * 64
            with self.assertRaisesRegex(ValueError, 'digest'):
                self.execute(root, profile, binary, args)
            self.assertEqual(list(artifact.iterdir()), [])
            args.archgraph_sha256 = runner.digest(binary)
            profile.chmod(0o755)
            with self.assertRaisesRegex(ValueError, 'private owned'):
                self.execute(root, profile, binary, args)
            self.assertEqual(list(artifact.iterdir()), [])

    def test_incomplete_or_failed_hook_events_cannot_claim_activation(self):
        safe, active = runner.evidence([], Path('/repo'), 'thread', 'turn', 'guarded')
        self.assertEqual(active, 'unknown')
        event = {'method': 'hook/completed', 'params': {'threadId': 'thread', 'turnId': 'turn',
                 'run': {'id': 'bad', 'eventName': 'stop', 'sourcePath': '/repo/.codex/hooks.json',
                         'handlerType': 'command', 'status': 'failed'}}}
        with self.assertRaises(ValueError):
            runner.evidence([event], Path('/repo'), 'thread', 'turn', 'guarded')


if __name__ == '__main__':
    unittest.main()
