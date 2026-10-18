import argparse
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import project_quality as quality


def graph():
    return {'kind': 'scan', 'files': [{'path': 'src/a.rs'}, {'path': 'src/b.rs'}],
            'modules': [{'id': 'src'}, {'id': 'src/a.rs'}],
            'edges': [{'from': 'src/a.rs', 'to': 'src/b.rs', 'kind': 'static'},
                      {'from': 'src/b.rs', 'to': 'src/a.rs', 'kind': 'type'}],
            'cycles': [{'level': 'module', 'members': ['a', 'b']}, {'level': 'file', 'members': ['x', 'y', 'z']}],
            'findings': [{'kind': 'cycle'}],
            'import_resolution': {'imports': 5, 'resolved': 4, 'unresolved': 1, 'resolved_percent': 80.0},
            'surface': {'modules': [{'deep_imports': [{}, {}]}]}, 'summary': {'type_only_cycles': 2}}


class QualityComparisonTests(unittest.TestCase):
    def test_metrics_keep_module_and_file_cycles_and_occurrences_separate(self):
        measured = quality.metrics(graph())
        self.assertEqual(measured['internal_edge_records'], 2)
        self.assertEqual(measured['edge_kinds'], {'runtime': 1, 'type': 1})
        self.assertEqual((measured['module_cycles'], measured['largest_module_cycle']), (1, 2))
        self.assertEqual((measured['file_cycles'], measured['largest_file_cycle']), (1, 3))
        self.assertEqual(measured['deep_imports'], 2)
        self.assertEqual(measured['import_resolution']['unresolved'], 1)

    def test_embedded_graph_parser_rejects_missing_duplicate_and_non_scan(self):
        doc = graph()
        doc['private_text'] = '<thing>&lt;'
        html = '<script id="graph" type="application/json">' + json.dumps(doc).replace('<', '\\u003c') + '</script>'
        self.assertEqual(quality.graph_from_html(html), doc)
        for invalid in ['', html + html, '<script id="graph">{"kind":"diff"}</script>']:
            with self.assertRaises(ValueError):
                quality.graph_from_html(invalid)

    def test_fixed_layout_uses_longest_physical_path_first_and_exact_files(self):
        layout = quality.fixed_layout(graph())
        self.assertEqual(layout, {'version': 1, 'modules': [
            {'id': 'src/a.rs', 'path': 'src/a.rs'}, {'id': 'src', 'path': 'src/**'}]})
        doc = graph()
        doc['modules'][0]['id'] = 'src/[id]'
        with self.assertRaisesRegex(ValueError, 'glob metacharacters'):
            quality.fixed_layout(doc)

    def test_source_change_and_inside_checkout_output_are_rejected(self):
        with self.assertRaisesRegex(RuntimeError, 'source or Git index changed'):
            quality.unchanged({'index': 'a'}, {'index': 'b'}, 'fixture')
        with self.assertRaisesRegex(ValueError, 'outside'):
            quality.ensure_outside(Path('/tmp/app/output'), [Path('/tmp/app')])
        self.assertEqual(quality.ensure_outside(Path('/tmp/artifacts'), [Path('/tmp/app')]), Path('/tmp/artifacts').resolve())

    def test_comparison_runs_four_scans_both_engines_and_fixed_layout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before, after = root / 'before', root / 'after'
            before.write_bytes(b'old-binary')
            after.write_bytes(b'new-binary')
            glarity, moneylust = root / 'glarity', root / 'moneylust'
            glarity.mkdir()
            moneylust.mkdir()
            args = argparse.Namespace(before=before, after=after, glarity=glarity, moneylust=moneylust,
                                      out=root / 'artifacts', report=root / 'report.md', repeats=2)
            commands = []

            def call(command, destination):
                command = [str(p) for p in command]
                commands.append(command)
                target = Path(command[-1])
                if command[2] == 'scan':
                    target.write_text(json.dumps(graph()))
                else:
                    target.write_text('<script id="graph">' + json.dumps(graph()) + '</script>')
                return {'command': command, 'seconds': 1.25, 'exit_code': 0}

            state = {'files': 1, 'tracked_clean': False, 'source_sha256': 'source', 'index_sha256': 'index'}
            with patch.object(quality, 'git', side_effect=lambda checkout, *args: str(checkout).encode()), \
                    patch.object(quality, 'source_state', return_value=state), patch.object(quality, 'call', side_effect=call):
                result = quality.compare(args)
            self.assertEqual(len(commands), 18)
            self.assertEqual(len(result['scans']), 4)
            self.assertEqual(result['scans']['glarity-backend-symbols']['before']['timing_seconds']['samples'], [1.25, 1.25])
            rust = [c for c in commands if 'moneylust-backend-' in c[-1] and c[2] == 'scan']
            self.assertEqual([c[c.index('--depth') + 1] for c in rust], ['3', '2', '2', '3'])
            markdown = args.report.read_text()
            self.assertNotIn(str(root), markdown)
            self.assertNotIn('private_text', markdown)
            self.assertIn('<glarity>', markdown)
            self.assertIn('not an approved architecture policy', markdown)
            self.assertEqual((args.out / 'before-archgraph').read_bytes(), b'old-binary')
            self.assertEqual((args.out / 'after-archgraph').read_bytes(), b'new-binary')
            self.assertEqual((args.out.stat().st_mode & 0o777), 0o700)

    def test_positive_repeat_count_required(self):
        with self.assertRaisesRegex(ValueError, '--repeats'):
            quality.compare(argparse.Namespace(repeats=0))


if __name__ == '__main__':
    unittest.main()
