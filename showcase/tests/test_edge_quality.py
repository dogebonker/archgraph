import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

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
            'config': {'excluded_files': ['test_a.py', 'generated.ts']},
            'type_only_cycles': [{'level': 'module', 'contains_runtime_cycle': False},
                                 {'level': 'module', 'contains_runtime_cycle': True},
                                 {'level': 'file', 'contains_runtime_cycle': False}],
        }
        result = MODULE.stats(document)
        self.assertEqual(result['module_cycles'], 1)
        self.assertEqual(result['largest_module_cycle'], 3)
        self.assertEqual(result['barrels'], 1)
        self.assertEqual(result['excluded_files'], 2)
        self.assertEqual(result['deferred_module_cycles'], 2)
        self.assertEqual(result['nonruntime_module_cycles'], 1)
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

    def test_import_occurrences_and_unknown_coverage_have_distinct_units(self):
        document = {'files': [], 'modules': [], 'edges': [], 'cycles': [],
                    'source': {'collector': {}}, 'summary': {}}
        self.assertIsNone(MODULE.stats(document)['import_resolution'])
        self.assertIsNone(MODULE.stats(document)['import_statement_kinds'])
        document.update(imports=[{'kind': 'runtime'}, {'kind': 'type'}],
                        import_resolution={'imports': 2, 'resolved': 1, 'unresolved': 1,
                                           'resolved_percent': 50.0},
                        external_nodes=[{'external': True, 'package': 'example'}])
        result = MODULE.stats(document)
        self.assertEqual(result['import_resolution']['imports'], 2)
        self.assertEqual(result['edges'], 0)
        self.assertEqual(result['import_statement_kinds'], {'runtime': 1, 'type': 1})
        self.assertEqual(result['external_packages'], 1)

    def test_readme_numbers_use_graphs_and_break_paths_are_escaped(self):
        source = ('intro\n<p align="center"><sub>Next.js old caption</sub></p>\n'
                  '## Results on 20 well-known repositories\n\n'
                  '| Project | Files |\n|---|---|\n'
                  '| [old](https://dogebonker.github.io/archgraph/next.js.html) | old |\n'
                  '\n## Keep this section\nunchanged\n')
        entry = {'name': 'next.js', 'repo': 'https://github.com/vercel/next.js',
                 'path': 'packages/next', 'commit': 'a' * 40}
        document = {'files': [{}, {}], 'modules': [{}, {}, {}], 'edges': [{'from': 'a|<src>.ts', 'to': 'b&.ts', 'kind': 'runtime'}, {}, {}, {}],
                    'cycles': [{'level': 'module', 'members': ['a', 'b'],
                                'break': [{'from': 'a|<src>.ts', 'to': 'b&.ts'}]}]}
        text = MODULE.readme_text(source, [entry], {'next.js': document})
        self.assertIn('2 files, 3 modules, 4 internal file dependencies.', text)
        self.assertIn('| 2 | 2 modules | review', text)
        self.assertIn('a&#124;&lt;src&gt;.ts → b&amp;.ts', text)
        self.assertIn('## Keep this section\nunchanged', text)
        document['cycles'] = []
        text = MODULE.readme_text(source, [entry], {'next.js': document})
        self.assertIn('| 2 | none | no module cycles |', text)

    def test_readme_refuses_a_partial_final_cohort(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            MODULE.write_json(root / 'baseline' / 'index.json', [])
            with self.assertRaisesRegex(RuntimeError, 'complete verified fix5 cohort'):
                MODULE.publish_readme(root, [], root / 'README.md')
            self.assertFalse((root / 'README.md').exists())

    def test_transition_reasons_use_new_edge_kind_and_added_targets(self):
        before = {'schema': 'archgraph/v1.1',
                  'files': [{'path': 'a', 'module': 'one'}, {'path': 'b', 'module': 'two'}],
                  'edges': [{'from': 'a', 'to': 'b', 'kind': 'runtime'}]}
        after = dict(before, edges=[{'from': 'a', 'to': 'b', 'kind': 'type'},
                                    {'from': 'b', 'to': 'a', 'kind': 'runtime'}])
        reasons = MODULE.transition_kinds({'one', 'two'}, before, after)
        self.assertEqual(reasons['type-only'], [before['edges'][0]])
        self.assertEqual(reasons['new or redirected runtime link'], [after['edges'][1]])

    def test_grabbag_counts_distinguish_complete_and_partial_proofs(self):
        document = {'files': [], 'modules': [], 'edges': [], 'cycles': [],
                    'source': {'collector': {}}, 'summary': {},
                    'grabbags': [{'remaining_cycles': []}, {'remaining_cycles': [['a', 'b']]}]}
        result = MODULE.stats(document)
        self.assertEqual(result['grabbag_reclassifications'], 2)
        self.assertEqual(result['grabbag_complete'], 1)

    def test_correction_stage_is_final_without_rewriting_fix_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entries = [{'name': 'one', 'commit': 'a' * 40}]
            for stage in MODULE.ARCHIVE_STAGES:
                graph = root / stage / 'one' / 'graph.json'
                MODULE.write_json(graph, {'source': {'git': {'commit': 'a' * 40}},
                                          'files': [{'path': stage + '.py'}]})
                MODULE.write_json(root / stage / 'index.json', [{
                    'status': 'ok', 'fingerprint': {'entry': entries[0]},
                    'graph_sha256': MODULE.digest(graph)}])
            original = (root / 'fix5' / 'one' / 'graph.json').read_bytes()
            complete, documents, _ = MODULE.archived_stages(root, entries)
            self.assertEqual(complete[-1], 'corrected')
            self.assertEqual(documents['fix5']['one']['files'], [{'path': 'fix5.py'}])
            self.assertEqual(documents['corrected']['one']['files'], [{'path': 'corrected.py'}])
            self.assertEqual((root / 'fix5' / 'one' / 'graph.json').read_bytes(), original)
            (root / 'README.md').write_text('existing readme')
            with patch.object(MODULE, 'HERE', root / 'showcase'), patch.object(
                    MODULE, 'readme_text', return_value='Generated from the final pinned scans in manifest') as render:
                MODULE.publish_readme(root, entries, root / 'generated.md')
            self.assertEqual(render.call_args.args[2], documents['corrected'])
            self.assertIn('including the exact-case Python resolver correction',
                          (root / 'generated.md').read_text())
            self.assertEqual((root / 'README.md').read_text(), 'existing readme')

    def test_deployed_comparison_rejects_divergent_final_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entry = {'name': 'one', 'commit': 'a' * 40}
            document = {'files': [], 'modules': [], 'edges': [], 'cycles': [],
                        'source': {'collector': {}}, 'summary': {'forbidden': 0}, 'findings': []}
            index = {'repos': [{'name': 'one', 'commit': entry['commit'], 'status': 'ok',
                               'stats': {'files': 0, 'modules': 0, 'imports': 0,
                                         'module_cycles': 0, 'largest_module_cycle': 0,
                                         'file_cycles': 0, 'forbidden': 0, 'import_resolution': None,
                                         'external_packages': 0, 'findings': {}}}]}
            MODULE.write_json(root / 'published-index.json', index)
            result = MODULE.compare_published(root, [entry], {'one': document})
            self.assertEqual(result['repositories'], 1)
            self.assertEqual(len(result['sha256']), 64)
            index['repos'][0]['stats']['files'] = 1
            MODULE.write_json(root / 'published-index.json', index)
            with self.assertRaisesRegex(RuntimeError, 'deployed files differs'):
                MODULE.compare_published(root, [entry], {'one': document})

    def test_atomic_json_and_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'stage' / 'metadata.json'
            MODULE.write_json(path, {'status': 'ok'})
            self.assertEqual(MODULE.json.loads(path.read_text()), {'status': 'ok'})
            self.assertFalse(path.with_suffix('.json.tmp').exists())
            self.assertEqual(len(MODULE.digest(path)), 64)


if __name__ == '__main__':
    unittest.main()
