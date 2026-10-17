"""Protocol and filesystem checks for the optional context adapters."""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "integrations" / "context-hook.py"
SPEC = importlib.util.spec_from_file_location("context_hook", SCRIPT)
HOOK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HOOK)


class ContextHookTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        (self.root / "a.ts").write_text("export const a = 1;\n")
        (self.root / "-odd.ts").write_text("export {};\n")
        self.report = {"entries": [{"tier": "constraints", "path": "layout.yaml", "mode": "rules",
                                    "reason": "architecture constraints", "tokens": 10,
                                    "content": "layers: []"},
                                   {"tier": "must-read", "path": "a.ts", "reason": "explicit seed",
                                    "tokens": 6, "mode": "body"}],
                       "budget": 40000, "used_tokens": 16, "limitations": ["estimates"]}
        self.environment = mock.patch.dict(os.environ, {"ARCHGRAPH_BINARY": "/opt/archgraph",
                                                       "ARCHGRAPH_CONTEXT_BUDGET": "40k",
                                                       "ARCHGRAPH_CONTEXT_FILES": "[]"})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.collect = mock.patch.object(HOOK.subprocess, "run", return_value=subprocess.CompletedProcess(
            [], 0, json.dumps(self.report), ""))
        self.process = self.collect.start()
        self.addCleanup(self.collect.stop)

    def event(self, client="claude", **changes):
        event = {"prompt": 'fix a.ts; $(touch ESCAPED) `echo no` — 東京', "cwd": str(self.root),
                 "session_id": "session-one", "conversation_id": "conversation-one",
                 "workspace_roots": [str(self.root)], "generation_id": "generation-one",
                 "hook_event_name": "UserPromptSubmit", "attachments": []}
        event.update(changes)
        return event

    def manifest(self, client="claude", event=None):
        _, session = HOOK.workspace(client, event or self.event())
        return json.loads((self.root / ".archgraph" / "context-hooks" / session / "manifest.json").read_text())

    def test_claude_protocol_argv_and_immutable_artifacts(self):
        with mock.patch.dict(os.environ, {"ARCHGRAPH_CONTEXT_FILES": '["a.ts", "-odd.ts"]'}):
            result = HOOK.run("claude", self.event())
        args, options = self.process.call_args
        self.assertEqual(args[0][:2], ["/opt/archgraph", "context"])
        self.assertEqual(args[0][args[0].index("--task") + 1], self.event()["prompt"])
        self.assertEqual(args[0][-3:], ["--files", "./-odd.ts", "a.ts"])
        self.assertEqual(options["timeout"], 8)
        self.assertNotIn("shell", options)
        self.assertFalse((self.root / "ESCAPED").exists())
        self.assertEqual(result["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit")
        self.assertIn("layers: []", result["hookSpecificOutput"]["additionalContext"])
        first = self.manifest()
        self.assertEqual(first["status"], "ready")
        self.assertEqual(first["seed_files"], ["-odd.ts", "a.ts"])
        HOOK.run("claude", self.event())
        second = self.manifest()
        self.assertNotEqual(first["report_artifact"], second["report_artifact"])
        self.assertTrue(Path(first["report_artifact"]).is_file())

    def test_cursor_only_continues_and_filters_attachments(self):
        outside = self.root.parent / "outside-context-test.ts"
        outside.write_text("do not read")
        self.addCleanup(outside.unlink)
        (self.root / "escape.ts").symlink_to(outside)
        attachments = [{"type": "file", "file_path": str(self.root / "a.ts")},
                       {"type": "file", "file_path": str(outside)},
                       {"type": "file", "file_path": "escape.ts"},
                       {"type": "folder", "file_path": "a.ts"},
                       {"type": "file", "file_path": "missing.ts"}]
        result = HOOK.run("cursor", self.event(attachments=attachments))
        self.assertEqual(result, {"continue": True})
        manifest = self.manifest("cursor")
        self.assertEqual(manifest["seed_files"], ["a.ts"])
        self.assertEqual(manifest["generation"], "generation-one")

    def test_failure_and_timeout_invalidate_only_current_session(self):
        HOOK.run("claude", self.event())
        other = self.event(session_id="other")
        HOOK.run("claude", other)
        for failure in (subprocess.TimeoutExpired("archgraph", 8),
                        subprocess.CalledProcessError(2, "archgraph"), FileNotFoundError()):
            self.process.side_effect = failure
            with mock.patch("sys.stderr", io.StringIO()):
                self.assertEqual(HOOK.run("claude", self.event(prompt="new request")), {})
            current = self.manifest()
            self.assertEqual(current["status"], "unavailable")
            self.assertEqual(current["task"], "new request")
            self.assertNotIn("report_artifact", current)
            self.assertTrue(Path(current["unavailable_artifact"]).is_file())
            self.assertEqual(self.manifest(event=other)["status"], "ready")

    def test_invalid_seed_environment_invalidates_previous_report(self):
        HOOK.run("claude", self.event())
        with mock.patch.dict(os.environ, {"ARCHGRAPH_CONTEXT_FILES": "not json"}), \
                mock.patch("sys.stderr", io.StringIO()):
            self.assertEqual(HOOK.run("claude", self.event()), {})
        self.assertEqual(self.manifest()["status"], "unavailable")

    def test_generation_and_workspace_isolation(self):
        HOOK.run("cursor", self.event())
        old = self.manifest("cursor")
        HOOK.run("cursor", self.event(generation_id="generation-two"))
        new = self.manifest("cursor")
        self.assertNotEqual(old["task_fingerprint"], new["task_fingerprint"])
        second = self.root / "another-workspace"
        second.mkdir()
        _, original_session = HOOK.workspace("cursor", self.event())
        _, second_session = HOOK.workspace("cursor", self.event(workspace_roots=[str(second)]))
        self.assertNotEqual(original_session, second_session)

    def test_symlink_artifact_directory_cannot_escape(self):
        outside = self.root / "outside"
        outside.mkdir()
        (self.root / ".archgraph").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(OSError):
            HOOK.run("claude", self.event())
        self.assertEqual(list(outside.iterdir()), [])

    def test_manifest_symlink_is_replaced_without_touching_target(self):
        HOOK.run("claude", self.event())
        _, session = HOOK.workspace("claude", self.event())
        manifest = self.root / ".archgraph" / "context-hooks" / session / "manifest.json"
        target = self.root / "untouched.json"
        target.write_text("untouched")
        manifest.unlink()
        manifest.symlink_to(target)
        HOOK.run("claude", self.event())
        self.assertFalse(manifest.is_symlink())
        self.assertEqual(target.read_text(), "untouched")

    def test_large_report_caps_claude_context_and_puts_constraints_first(self):
        self.report["omitted"] = [{"path": "missing.ts", "reason": "missing"}]
        self.report["entries"].insert(0, {"tier": "skim", "path": "huge.ts", "mode": "exports",
                                          "content": "x" * 50000, "reason": "far dependency"})
        self.process.return_value.stdout = json.dumps(self.report)
        result = HOOK.run("claude", self.event())["hookSpecificOutput"]["additionalContext"]
        self.assertLess(len(result), 10000)
        self.assertLess(result.index("constraints | layout.yaml"), result.index("skim | huge.ts"))
        self.assertIn(self.manifest()["markdown_artifact"], result)
        self.assertIn("Omitted entries: 1", result)
        self.assertIn(self.manifest()["report_artifact"], result)
        self.assertIn("Omitted entries: 1. Use --format json", HOOK.markdown_report(self.report))
        self.assertIn("x" * 50000, Path(self.manifest()["markdown_artifact"]).read_text())

    def test_cursor_session_guidance_does_not_run_collector(self):
        result = HOOK.run("cursor-session", self.event())
        self.assertEqual(set(result), {"additional_context"})
        self.assertIn("exactly match", result["additional_context"])
        self.process.assert_not_called()

    def test_missing_session_and_invalid_input_soft_fail_with_json_only(self):
        events = [("claude", "not JSON"), ("claude", json.dumps(self.event(session_id=None))),
                  ("cursor", json.dumps(self.event(conversation_id=None)))]
        for client, event in events:
            with mock.patch("sys.argv", [str(SCRIPT), client]), mock.patch("sys.stdin", io.StringIO(event)), \
                    mock.patch("sys.stdout", io.StringIO()) as output, mock.patch("sys.stderr", io.StringIO()):
                HOOK.main()
                self.assertEqual(json.loads(output.getvalue()), {"continue": True} if client == "cursor" else {})

    def test_invalid_report_soft_fails(self):
        self.process.return_value.stdout = '{"entries": [{"tier": "skim"}]}'
        with mock.patch("sys.stderr", io.StringIO()):
            self.assertEqual(HOOK.run("cursor", self.event()), {"continue": True})
        self.assertEqual(self.manifest("cursor")["status"], "unavailable")

    def test_real_executable_protocol_preserves_unicode_and_task_as_data(self):
        self.collect.stop()
        binary = self.root / "fake archgraph"
        trace = self.root / "arguments.json"
        binary.write_text("#!" + sys.executable + "\nimport json, pathlib, sys\n"
                          + "pathlib.Path(" + repr(str(trace)) + ").write_text(json.dumps(sys.argv))\n"
                          + "print(" + repr(json.dumps(self.report)) + ")\n")
        binary.chmod(0o700)
        environment = dict(os.environ, ARCHGRAPH_BINARY=str(binary), ARCHGRAPH_CONTEXT_FILES='["a.ts"]')
        for client in ("claude", "cursor"):
            completed = subprocess.run([sys.executable, str(SCRIPT), client],
                                       input=json.dumps(self.event(), ensure_ascii=False), text=True,
                                       capture_output=True, env=environment, check=True)
            result = json.loads(completed.stdout)
            self.assertEqual(completed.stderr, "")
            if client == "cursor":
                self.assertEqual(result, {"continue": True})
            else:
                self.assertIn("additionalContext", result["hookSpecificOutput"])
            arguments = json.loads(trace.read_text())
            self.assertEqual(arguments[arguments.index("--task") + 1], self.event()["prompt"])
        self.assertFalse((self.root / "ESCAPED").exists())

    def test_example_configurations_parse(self):
        integrations = SCRIPT.parent
        claude = json.loads((integrations / "claude-context.settings.json").read_text())
        self.assertEqual(claude["hooks"]["UserPromptSubmit"][0]["hooks"][0]["timeout"], 10)
        cursor = json.loads((integrations / "cursor-context.hooks.json").read_text())
        self.assertEqual(cursor["version"], 1)
        self.assertIn("sessionStart", cursor["hooks"])
        self.assertFalse(cursor["hooks"]["beforeSubmitPrompt"][0]["failClosed"])


if __name__ == "__main__":
    unittest.main()
