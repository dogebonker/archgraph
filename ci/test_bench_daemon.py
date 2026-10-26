#!/usr/bin/env python3
"""Small checks for benchmark selection and latency accounting; no repositories required."""
import unittest

from bench_daemon import edit_plan, quantiles


class DaemonBenchmark(unittest.TestCase):
    def test_nearest_rank_small_sample_quantiles(self):
        self.assertEqual(quantiles([30, 10, 20]), {"p50_ms": 20, "p95_ms": 30})

    def test_typescript_imports_are_new_and_each_file_is_distinct(self):
        doc = {"files": [{"path": "src/leaf.ts"}, *({"path": f"src/mod{i}/file.ts"} for i in range(50))],
               "edges": [], "config": {}}
        plan = edit_plan(doc, "typescript", 50)
        self.assertEqual(len(plan), 50)
        self.assertEqual(len({source for source, _, _ in plan}), 50)
        self.assertTrue(all(source != target for source, target, _ in plan))
        self.assertTrue(all(addition.startswith(b"\nimport ") for _, _, addition in plan))

    def test_python_import_is_a_full_module_name(self):
        doc = {"files": [{"path": "django/a.py"}, {"path": "django/b.py"}],
               "edges": [{"from": "django/a.py", "to": "django/b.py"}], "config": {}}
        plan = edit_plan(doc, "python", 1)
        self.assertEqual(plan, [("django/b.py", "django/a.py", b"\nimport django.a  # boundrail daemon benchmark\n")])

    def test_barrels_are_not_changed_into_runtime_files_by_the_probe(self):
        doc = {"files": [{"path": "src/a.ts", "barrel": True},
                         {"path": "src/b.ts"}, {"path": "src/c.ts"}], "edges": []}
        plan = edit_plan(doc, "typescript", 1)
        self.assertEqual((plan[0][0], plan[0][1]), ("src/c.ts", "src/b.ts"))

    def test_insufficient_sources_is_an_error_not_a_smaller_benchmark(self):
        with self.assertRaisesRegex(RuntimeError, "select 50"):
            edit_plan({"files": [{"path": "a.ts"}], "edges": []}, "typescript", 50)


if __name__ == "__main__":
    unittest.main()
