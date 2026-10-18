import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

from native_engine import agreement, compare, prepare


def graph(files=("a.ts", "b.ts"), edges=None):
    return {"files": [{"path": path} for path in files],
            "edges": edges if edges is not None else [{"from": "a.ts", "to": "b.ts"}]}


class NativeComparisonTests(unittest.TestCase):
    def test_missing_file_does_not_inflate_all_file_recall(self):
        result = compare(graph(), graph(files=("a.ts",), edges=[]))
        self.assertEqual(result["missing_files"], ["b.ts"])
        self.assertEqual(result["metrics"]["all_files"]["endpoints"]["recall"], 0)
        self.assertEqual(result["metrics"]["shared_files"]["endpoints"]["recall"], 1)
        self.assertFalse(result["numeric_gate_passes"])

    def test_kind_aliases_default_and_original_target_are_separate(self):
        reference = graph(edges=[{"from": "a.ts", "to": "b.ts", "kind": "static", "line": 4}])
        native = graph(edges=[{"from": "a.ts", "to": "b.ts", "kind": "runtime", "line": 8}])
        self.assertEqual(compare(reference, native)["metrics"]["all_files"]["kind_and_original_target"]["recall"], 1)
        native["edges"][0]["original_to"] = "index.ts"
        result = compare(reference, native)
        self.assertEqual(result["metrics"]["all_files"]["endpoints"]["recall"], 1)
        self.assertEqual(result["metrics"]["all_files"]["kind_and_original_target"]["recall"], 0)
        self.assertEqual(result["disagreements"]["kind_and_original_target"]["reference_only"][0]["evidence"][0]["line"], 4)

    def test_duplicates_and_external_edges_are_not_internal_edges(self):
        reference = graph()
        native = graph(edges=[{"from": "a.ts", "to": "b.ts"}, {"from": "a.ts", "to": "b.ts"},
                              {"from": "a.ts", "to": "external:typescript:react"}])
        result = compare(reference, native)
        self.assertTrue(result["numeric_gate_passes"])
        self.assertFalse(result["default_flip_ready"])
        self.assertIsNone(result["native"]["resolution"])

    def test_precision_and_recall_are_both_required(self):
        result = agreement({("a", "b")}, {("a", "b"), ("a", "c")})
        self.assertEqual(result["recall"], 1)
        self.assertEqual(result["precision"], .5)

    def test_pin_mismatch_does_not_modify_cached_repository(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            root = work / "repos/example"
            root.mkdir(parents=True)
            with patch("native_engine._showcase.git", return_value="wrong") as git:
                with self.assertRaisesRegex(RuntimeError, "unexpected commit"):
                    prepare({"name": "example", "commit": "a" * 40}, work, work / "cache")
                git.assert_called_once_with(root, "rev-parse", "HEAD")


if __name__ == "__main__":
    unittest.main()
