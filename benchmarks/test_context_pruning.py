"""Tests for the pre-registered retrieval experiment, independent of graph ranking."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "context_pruning", Path(__file__).with_name("context_pruning.py")
)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


class ProtocolTests(unittest.TestCase):
    def test_holdout_is_order_independent(self):
        self.assertEqual(
            M.heldout("owner/repo", 42, ["b.ts", "a.ts"]),
            M.heldout("owner/repo", 42, ["a.ts", "b.ts"]),
        )
        self.assertIn(M.heldout("owner/repo", 42, ["a.ts", "b.ts"]), ["a.ts", "b.ts"])

    def test_root_boundary_and_source_extensions(self):
        e = {"path": "packages/server", "language": "typescript"}
        self.assertEqual(M.relative_source("packages/server/src/a.ts", e), "src/a.ts")
        self.assertIsNone(M.relative_source("packages/serverish/a.ts", e))
        self.assertIsNone(M.relative_source("packages/server/README.md", e))
        self.assertEqual(
            M.relative_source("module.py", {"path": ".", "language": "python"}),
            "module.py",
        )

    def test_title_keywords_are_case_insensitive_and_unique(self):
        self.assertEqual(
            M.keywords("fix: BILLING and billing context"), ["billing", "context"]
        )

    def test_baseline_is_budgeted_seed_first_and_never_reads_heldout_label(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a").mkdir()
            for name in ["a/seed.ts", "a/target.ts", "other.ts", "unused.ts"]:
                (root / name).write_text("billing" if name == "other.ts" else "abcdefg")
            graph = root / "graph.json"
            graph.write_text(
                json.dumps(
                    {
                        "files": [
                            {"path": n}
                            for n in [
                                "a/seed.ts",
                                "a/target.ts",
                                "other.ts",
                                "unused.ts",
                            ]
                        ]
                    }
                )
            )
            case = {
                "root": str(root),
                "graph": str(graph),
                "seeds": ["a/seed.ts"],
                "title": "billing",
                "heldout": "unused.ts",
            }
            budget = M.cost("a/seed.ts", "explicit seed", 7) + M.cost(
                "other.ts", "grep task keywords", 7
            )
            first = M.baseline(case, budget)
            self.assertEqual(first["paths"], ["a/seed.ts", "other.ts"])
            self.assertEqual(first["tokens"], budget)
            case["heldout"] = "a/target.ts"
            self.assertEqual(M.baseline(case, budget), first)
            all_paths = M.baseline(case, 1000)["paths"]
            self.assertIn("a/target.ts", all_paths)
            self.assertNotIn("unused.ts", all_paths)

    def test_folder_distance_uses_components_not_prefix_text(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = ["a/seed.ts", "a/child/close.ts", "aish/far.ts"]
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("bill")
            graph = root / "graph.json"
            graph.write_text(json.dumps({"files": [{"path": n} for n in names]}))
            case = {
                "root": str(root),
                "graph": str(graph),
                "seeds": ["a/seed.ts"],
                "title": "bill",
            }
            budget = M.cost("a/seed.ts", "explicit seed", 4) + M.cost(
                "a/child/close.ts", "grep task keywords", 4
            )
            self.assertEqual(
                M.baseline(case, budget)["paths"], ["a/seed.ts", "a/child/close.ts"]
            )

    def test_constraints_charge_the_same_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "seed.py").write_text("four")
            graph = root / "graph.json"
            graph.write_text(json.dumps({"files": [{"path": "seed.py"}]}))
            case = {
                "root": str(root),
                "graph": str(graph),
                "seeds": ["seed.py"],
                "title": "anything",
                "constraints": [{"path": "layout.yaml", "tokens": 3}],
            }
            self.assertEqual(
                M.baseline(case, 3), {"paths": ["layout.yaml"], "tokens": 3}
            )

    def test_prepare_rejects_clone_symlink_outside_experiment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            work = root / "work"
            clone = work / "repos/repo"
            clone.parent.mkdir(parents=True)
            outside = root / "shared-cache"
            outside.mkdir()
            clone.symlink_to(outside, target_is_directory=True)
            with (
                mock.patch.object(M, "WORK", work),
                mock.patch.object(M, "run") as command,
            ):
                with self.assertRaisesRegex(ValueError, "inside its work directory"):
                    M.prepare({"name": "repo", "repo": "https://github.com/owner/repo"})
                command.assert_not_called()

    def test_prepare_rejects_shared_git_directory_before_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            work = root / "work"
            (work / "repos/repo").mkdir(parents=True)
            with (
                mock.patch.object(M, "WORK", work),
                mock.patch.object(
                    M, "run", return_value=str(root / "cache/.git")
                ) as command,
            ):
                with self.assertRaisesRegex(ValueError, "shared cache"):
                    M.prepare({"name": "repo", "repo": "https://github.com/owner/repo"})
                self.assertEqual(command.call_count, 1)

    def test_frozen_inventory_has_thirty_observations_from_exact_twenty_repos(self):
        frozen = json.loads((M.REPO / "benchmarks/context-prs.json").read_text())
        cohort = json.loads((M.REPO / "showcase/repos.json").read_text())[:20]
        self.assertEqual(len(frozen["cases"]), 30)
        self.assertEqual(
            {c["name"] for c in frozen["cases"]}, {c["name"] for c in cohort}
        )
        for index, repo in enumerate(cohort):
            cases = [c for c in frozen["cases"] if c["name"] == repo["name"]]
            self.assertEqual(len(cases), 2 if index < 10 else 1)
            for case in cases:
                self.assertEqual(
                    case["heldout"],
                    M.heldout(case["repo"], case["number"], case["eligible"]),
                )
                self.assertEqual(
                    set(case["seeds"]), set(case["eligible"]) - {case["heldout"]}
                )
                self.assertTrue(case["base_is_ancestor_of_merge"])
                self.assertEqual(
                    case["scan_metadata"]["source"]["git"]["commit"], case["base"]
                )
                self.assertFalse(case["scan_metadata"]["source"]["git"]["dirty"])

    def test_export_retrieval_is_not_reported_as_full_body(self):
        case = {
            "name": "repo",
            "number": 1,
            "repo": "owner/repo",
            "base": "a" * 40,
            "url": "https://github.com/owner/repo/pull/1",
            "heldout": "x.py",
            "changes": [{"status": "modified"}],
            "eligible": ["x.py", "y.py"],
        }
        row = {
            "name": "repo",
            "pr": 1,
            "status": "ok",
            "context_hit": True,
            "context_body_hit": False,
            "baseline_hit": False,
            "context_tokens": 100,
            "baseline_tokens": 90,
        }
        result = {
            "results": [
                {
                    "budget": 40000,
                    "denominator": 1,
                    "context_hits": 1,
                    "context_body_hits": 0,
                    "baseline_hits": 0,
                    "context_only": 1,
                    "baseline_only": 0,
                    "cases": [row],
                }
            ]
        }
        output = M.results_markdown(result, {"cases": [case]})
        self.assertIn("1/1 (100.0%) | 0/1 (0.0%)", output)
        self.assertIn("| `x.py` | exports | miss |", output)


if __name__ == "__main__":
    unittest.main()
