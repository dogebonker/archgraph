import unittest
from unittest.mock import patch
from validate_hotspots import aggregate, prefetch


class ValidationSummary(unittest.TestCase):
    def test_weights_by_unique_fix_commits_and_reports_coverage(self):
        rows = [
            {"status": "ok", "validation": {"bugfix_commits": 10, "top_bugfix_commits": 6, "expected_random_share": .3}},
            {"status": "ok", "validation": {"bugfix_commits": 20, "top_bugfix_commits": 9, "expected_random_share": .1}},
            {"status": "ok", "validation": {"bugfix_commits": 0, "top_bugfix_commits": 0}},
            {"status": "failed", "error": "timeout"},
        ]
        result = aggregate(rows)
        self.assertEqual(result["attempted"], 4)
        self.assertEqual(result["completed"], 3)
        self.assertEqual(result["with_fix_commits"], 2)
        self.assertEqual(result["share"], .5)
        self.assertEqual(result["random_lift"], 3)
        self.assertIsNone(aggregate([])["share"])

    def test_partial_clone_objects_are_fetched_in_a_batch_without_ref_updates(self):
        oid = "a" * 40
        with patch("validate_hotspots.git_data", side_effect=["1760709515\n", "b"*40+"\n", f"?{oid}\n", ""]) as git:
            report = prefetch({"repo_path":"/tmp/repo", "commit":"b"*40}, 30)
        self.assertEqual(report["missing_objects"], 1)
        fetch = git.call_args_list[-1]
        self.assertIn("--no-write-fetch-head", fetch.args[1])
        self.assertIn("--no-tags", fetch.args[1])
        self.assertEqual(fetch.args[3], oid+"\n")


if __name__ == "__main__":
    unittest.main()
