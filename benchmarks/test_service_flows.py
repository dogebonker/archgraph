import copy
import hashlib
import json
from types import SimpleNamespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from service_flows import compare, run, validate_truth


def edge(to="b", operation="GET /x", line=1):
    return {"from": "a", "to": to, "kind": "http", "operation": operation,
            "source": {"file": "caller.ts", "line": line}}


def truth():
    return {"version": 1, "repositories": [{"name": "app"}], "flows": [
        {"id": "F1", "repository": "app", "source": {"file": "caller.ts", "line": 1},
         "expected": {"from": "a", "to": "b", "kind": "http", "operation": "GET /x"},
         "reason": "a literal call and route"}], "controls": [
        {"id": "C1", "repository": "app", "source": {"file": "caller.ts", "line": 3},
         "forbid": {"kind": "http"}, "reason": "an external HTTP call"}]}


def report(edges):
    return {"app": {"schema": "archgraph/v1.2", "kind": "services",
                    "source": {"collector": "inferred", "complete": False},
                    "services": ["a", "b", "c"], "edges": edges, "summary": {"calls": len(edges)}}}


class ServiceFlowComparisonTests(unittest.TestCase):
    def test_missing_edge_is_a_miss_without_claiming_perfect_precision(self):
        result = compare(truth(), report([]))
        self.assertEqual(result["matched_flows"], 0)
        self.assertEqual(result["recall"], 0)
        self.assertIsNone(result["precision"])
        self.assertEqual(result["misses"][0]["id"], "F1")
        self.assertFalse(result["all_labelled_flows_recovered"])

    def test_destination_swap_is_detected_without_changing_total_count(self):
        good = compare(truth(), report([edge()]))
        bad = compare(truth(), report([edge(to="c")]))
        self.assertTrue(good["all_labelled_flows_recovered"])
        self.assertEqual(good["unique_predictions"], bad["unique_predictions"])
        self.assertEqual(bad["matched_flows"], 0)
        self.assertEqual(len(bad["false_positives"]), 1)
        self.assertEqual(bad["recall"], 0)
        self.assertEqual(bad["precision"], 0)

    def test_duplicates_cannot_inflate_coverage(self):
        result = compare(truth(), report([edge(), edge()]))
        self.assertEqual(result["matched_flows"], 1)
        self.assertEqual(result["unique_predictions"], 1)
        self.assertEqual(result["duplicate_predictions"], 1)
        self.assertEqual(result["recall"], 1)

    def test_external_control_prediction_is_false_positive(self):
        result = compare(truth(), report([edge(), edge(line=3)]))
        self.assertEqual(result["controls"]["violated"], ["C1"])
        self.assertEqual(result["precision"], 0.5)
        self.assertTrue(result["all_labelled_flows_recovered"])

    def test_unlabelled_predictions_require_review_and_are_not_true_positives(self):
        result = compare(truth(), report([edge(), edge(line=8)]))
        self.assertEqual(result["matched_flows"], 1)
        self.assertEqual(result["precision"], 1)
        self.assertEqual(len(result["unreviewed_predictions"]), 1)
        self.assertTrue(result["needs_prediction_review"])
        self.assertTrue(result["all_labelled_flows_recovered"])

    def test_missing_or_inconsistent_report_is_not_a_match(self):
        with self.assertRaisesRegex(ValueError, "every labelled repository"):
            compare(truth(), {})
        invalid = report([edge()])
        invalid["app"]["summary"]["calls"] = 0
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            compare(truth(), invalid)
        invalid = report([edge()])
        invalid["app"]["source"]["collector"] = "declarative"
        with self.assertRaisesRegex(ValueError, "live inferred"):
            compare(truth(), invalid)


class ServiceFlowEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repos = Path(self.temp.name)
        (self.repos / "app").mkdir()
        self.file = self.repos / "app/caller.ts"
        self.file.write_text("fetch('/x');\napp.get('/x', handler);\n")
        self.data = truth()
        self.data["controls"] = []
        self.data["repositories"][0].update(commit="a" * 40, files={"caller.ts": self.hash()})
        self.data["flows"][0].update(target={"file": "caller.ts", "line": 2}, evidence=[
            {"file": "caller.ts", "line": 1, "text": "fetch('/x');"},
            {"file": "caller.ts", "line": 2, "text": "app.get('/x', handler);"}])

    def hash(self):
        return hashlib.sha256(self.file.read_bytes()).hexdigest()

    def test_valid_evidence_reports_its_narrow_provenance(self):
        status = validate_truth(self.data, self.repos)
        self.assertEqual(status["app"]["verified_files"], 1)
        self.assertEqual(status["app"]["provenance"], "annotated_files_only")

    def test_git_head_mismatch_is_rejected_without_running_status(self):
        (self.repos / "app/.git").mkdir()
        with patch("service_flows.subprocess.run", return_value=SimpleNamespace(stdout="b" * 40)) as git:
            with self.assertRaisesRegex(ValueError, "must be at"):
                validate_truth(self.data, self.repos)
            self.assertEqual(git.call_args.args[0][-2:], ["rev-parse", "HEAD"])
            self.assertEqual(git.call_count, 1)
            self.assertIn("--no-optional-locks", git.call_args.args[0])
            git.return_value.stdout = "a" * 40
            result = validate_truth(self.data, self.repos)
            self.assertEqual(result["app"]["provenance"], "git_head_and_annotated_files")

    def test_missing_and_stale_files_are_rejected(self):
        self.file.write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "stale source"):
            validate_truth(self.data, self.repos)
        self.file.unlink()
        with self.assertRaisesRegex(ValueError, "stale source"):
            validate_truth(self.data, self.repos)

    def test_changed_annotation_is_rejected_even_with_fresh_file_hash(self):
        self.file.write_text("fetch('/other');\napp.get('/x', handler);\n")
        self.data["repositories"][0]["files"]["caller.ts"] = self.hash()
        with self.assertRaisesRegex(ValueError, "witness moved or changed"):
            validate_truth(self.data, self.repos)

    def test_missing_receiver_and_duplicate_truth_cannot_inflate_recall(self):
        del self.data["flows"][0]["target"]
        with self.assertRaisesRegex(ValueError, "missing receiver"):
            validate_truth(self.data, self.repos)
        self.setUp()
        duplicate = copy.deepcopy(self.data["flows"][0])
        duplicate["id"] = "F2"
        self.data["flows"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "duplicate flow"):
            validate_truth(self.data, self.repos)

    def test_boundary_override_cannot_supply_declared_calls(self):
        (self.repos / "app/a").mkdir()
        self.data["repositories"][0]["boundary_manifest"] = {"version": 1, "services": ["a"], "edges": [edge()]}
        with self.assertRaisesRegex(ValueError, "edges: \\[\\]"):
            validate_truth(self.data, self.repos)
        self.data["repositories"][0]["boundary_manifest"]["edges"] = []
        self.assertIn("app", validate_truth(self.data, self.repos))

    def args(self):
        truth_path = self.repos / "truth.json"
        truth_path.write_text(json.dumps(self.data))
        tool = self.repos / "fake archgraph"
        tool.write_text("#!/usr/bin/env python3\nimport json\nprint(" + repr(json.dumps(report([edge()])["app"])) + ")\n")
        tool.chmod(0o755)
        return SimpleNamespace(truth=truth_path, repos=self.repos, archgraph=tool,
                               output=self.repos / "evaluation.json", timeout=10)

    def test_runner_freezes_the_binary_for_every_repository(self):
        second = copy.deepcopy(self.data["repositories"][0])
        second["name"] = "second"
        self.data["repositories"].append(second)
        (self.repos / "second").mkdir()
        (self.repos / "second/caller.ts").write_bytes(self.file.read_bytes())
        second_case = copy.deepcopy(self.data["flows"][0])
        second_case.update(id="F2", repository="second")
        self.data["flows"].append(second_case)
        args = self.args()
        script = ("#!/usr/bin/env python3\nfrom pathlib import Path\n"
                  + f"Path({str(args.archgraph)!r}).write_text('#!/bin/sh\\nexit 9\\n')\n"
                  + "print(" + repr(json.dumps(report([edge()])["app"])) + ")\n")
        args.archgraph.write_text(script)
        expected_hash = hashlib.sha256(script.encode()).hexdigest()
        result = run(args)
        self.assertEqual(result["comparison"]["matched_flows"], 2)
        self.assertEqual(result["binary_sha256"], expected_hash)
        self.assertNotEqual(hashlib.sha256(args.archgraph.read_bytes()).hexdigest(), expected_hash)
        self.assertEqual(len({entry["command"][0] for entry in result["runs"].values()}), 1)

    def test_runner_refuses_artifacts_inside_or_symlinked_into_sources(self):
        args = self.args()
        args.output = self.repos / "app/result.json"
        with self.assertRaisesRegex(ValueError, "outside the checkout"):
            run(args)
        args.output = self.repos / "evaluation.json"
        (self.repos / "evaluation-reports").symlink_to(self.repos / "app", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "outside the checkout"):
            run(args)
        self.assertEqual(sorted(p.name for p in (self.repos / "app").iterdir()), ["caller.ts"])

    def test_evidence_symlink_cannot_escape_source_root(self):
        outside = self.repos / "outside.ts"
        outside.write_text(self.file.read_text())
        self.file.unlink()
        try:
            self.file.symlink_to(outside)
        except (OSError, NotImplementedError):
            self.skipTest("symbolic links unavailable")
        with self.assertRaisesRegex(ValueError, "escapes repository"):
            validate_truth(self.data, self.repos)


if __name__ == "__main__":
    unittest.main()
