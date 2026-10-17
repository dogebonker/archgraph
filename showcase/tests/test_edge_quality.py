import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('edge_quality', Path(__file__).parents[1] / 'edge_quality.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class EdgeQualityTests(unittest.TestCase):
    def test_summary_counts_module_cycles_without_counting_file_cycles(self):
        document = {
            'files': [{'barrel': True}, {}], 'modules': [{}],
            'edges': [{'kind': 'runtime'}, {'kind': 'type'}],
            'cycles': [{'level': 'file', 'members': ['a', 'b']},
                       {'level': 'module', 'members': ['one', 'two', 'three']}],
            'source': {'collector': {'name': 'example'}}, 'summary': {},
        }
        result = MODULE.stats(document)
        self.assertEqual(result['module_cycles'], 1)
        self.assertEqual(result['largest_module_cycle'], 3)
        self.assertEqual(result['barrels'], 1)
        self.assertEqual(result['edge_kinds'], {'runtime': 1, 'type': 1})

    def test_cycle_expansion_is_not_reported_as_disappearance(self):
        document = {'cycles': [{'level': 'module', 'members': ['a', 'b', 'c']}]}
        self.assertEqual(MODULE.cycle_disposition({'a', 'b'}, document), 'persists in a larger cycle')
        self.assertEqual(MODULE.cycle_disposition({'x', 'y'}, document), 'eliminated')

    def test_cycle_shrinking_reports_remaining_members(self):
        document = {'cycles': [{'level': 'module', 'members': ['a', 'b']}]}
        self.assertIn('2 original modules remain cyclic', MODULE.cycle_disposition({'a', 'b', 'c'}, document))

    def test_runtime_edge_inventory_ignores_kind_renames(self):
        before = {
            'schema': 'archgraph/v1',
            'files': [{'path': 'a.ts', 'module': 'a'}, {'path': 'b.ts', 'module': 'b'}],
            'edges': [{'from': 'a.ts', 'to': 'b.ts', 'kind': 'static'},
                      {'from': 'b.ts', 'to': 'a.ts', 'kind': 'reexport'}],
        }
        after = dict(before, schema='archgraph/v1.1', edges=[
            {'from': 'a.ts', 'to': 'b.ts', 'kind': 'runtime'},
            {'from': 'b.ts', 'to': 'a.ts', 'kind': 'reexport'}])
        changes = MODULE.cycle_edge_changes({'a', 'b'}, before, after)
        self.assertEqual(changes['changed_kinds'], [])
        self.assertEqual(changes['runtime_edges_removed'], [before['edges'][1]])

    def test_report_rejects_incomplete_cohort(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            MODULE.write_json(root / 'baseline' / 'index.json', [])
            with self.assertRaisesRegex(RuntimeError, 'every manifest repository'):
                MODULE.report(root, [{'name': 'missing'}], root / 'report.md')
            self.assertFalse((root / 'report.md').exists())

    def test_report_rejects_modified_graph_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            row = {'status': 'ok', 'fingerprint': {'entry': {'name': 'one'}},
                   'graph_sha256': 'not-the-original-hash'}
            MODULE.write_json(root / 'baseline' / 'index.json', [row])
            MODULE.write_json(root / 'baseline' / 'one' / 'graph.json', {})
            with self.assertRaisesRegex(RuntimeError, 'archived graph hash changed'):
                MODULE.report(root, [{'name': 'one'}], root / 'report.md')
            self.assertFalse((root / 'report.md').exists())

    def test_atomic_json_and_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'stage' / 'metadata.json'
            MODULE.write_json(path, {'status': 'ok'})
            self.assertEqual(MODULE.json.loads(path.read_text()), {'status': 'ok'})
            self.assertFalse(path.with_suffix('.json.tmp').exists())
            self.assertEqual(len(MODULE.digest(path)), 64)


if __name__ == '__main__':
    unittest.main()
