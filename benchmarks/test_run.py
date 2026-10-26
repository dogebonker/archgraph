import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from run import check, GRAPH_FIELDS, main


class BenchmarkChecks(unittest.TestCase):
    def setUp(self):
        self.graph = {k: [] for k in GRAPH_FIELDS}
        self.graph.update({"kind": "scan", "files": [{"path": "A.java"}],
                           "edges": [{"from": "A.java", "to": "B.java"}],
                           "source": {"collector": {"unresolved": 0}}})

    def test_source_oracle_detects_missing_edges_counts_and_package_noise(self):
        case = {"files": 1, "edges": [["A.java", "B.java"]], "unresolved": 0, "no_package_edges": True}
        self.assertEqual(check(case, self.graph), [])
        broken = copy.deepcopy(self.graph)
        broken["edges"] = [{"from": "A.java", "to": "package/package-info.java"}]
        broken["files"] = []
        self.assertEqual(len(check(case, broken)), 3)

    def test_history_oracle_distinguishes_source_deletions_from_docs_changes(self):
        first = copy.deepcopy(self.graph)
        first["files"].append({"path": "Deleted.java"})
        document = {"kind": "history", "snapshots": [{"graph": g} for g in [first, self.graph, copy.deepcopy(self.graph)]]}
        case = {"files": [2, 1, 1], "removed_file": "Deleted.java", "unchanged": [1, 2]}
        self.assertEqual(check(case, document), [])
        document["snapshots"][2]["graph"]["edges"] = []
        self.assertEqual(len(check(case, document)), 1)

    def test_failed_fetch_leaves_no_checkout_and_can_be_retried(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "boundrail"
            binary.write_bytes(b"test binary; subprocesses are mocked")
            (root / "repos.json").write_text(json.dumps([{
                "name": "retry", "repo": "https://github.com/example/retry",
                "commit": "0" * 40, "language": "java",
            }]))
            fetches = []

            def fail_fetch(command, cwd=None):
                if "fetch" in command:
                    fetches.append(command)
                    raise RuntimeError("temporary network failure")
                return ""

            argv = ["run.py", "--work", str(root / "work"), "--output", str(root / "results.json"), "--binary", str(binary)]
            with patch("run.HERE", root), patch("sys.argv", argv), patch("run.run", side_effect=fail_fetch), patch("builtins.print"):
                for _ in range(2):
                    self.assertEqual(main(), 1)
                    self.assertFalse((root / "work/repos/retry").exists())
            self.assertEqual(len(fetches), 2)


if __name__ == "__main__":
    unittest.main()
