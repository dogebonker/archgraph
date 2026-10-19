import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).with_name('prepare.py'))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class PrepareTest(unittest.TestCase):
    def test_pinned_clean_independent_reproducible_clones_and_external_oracle(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'sources' / 'flask'
            source.mkdir(parents=True)
            subprocess.run(['git', 'init', '--quiet', str(source)], check=True)
            (source / 'upstream.py').write_text('x = 1\n')
            prepare.git(source, 'add', '.')
            prepare.git(source, '-c', 'commit.gpgsign=false', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--quiet', '-m', 'fixture')
            pin = prepare.git(source, 'rev-parse', 'HEAD')
            with patch.object(prepare, 'SOURCES', {'flask': dict(repo='https://example.invalid/flask', commit=pin)}), patch.object(prepare, 'CASES', {'flask': prepare.CASES['flask']}):
                first = prepare.prepare(source.parent, root / 'one', Path(sys.executable), Path('/node'))
                second = prepare.prepare(source.parent, root / 'two', Path(sys.executable), Path('/node'))
                self.assertEqual(first[0]['baseline_commit'], second[0]['baseline_commit'])
                self.assertEqual(prepare.git(source, 'rev-parse', 'HEAD'), pin)
                self.assertEqual(prepare.git(source, 'status', '--porcelain'), '')
                self.assertEqual(prepare.git(root / 'one/flask', 'status', '--porcelain'), '')
                original = source / '.git/objects' / pin[:2] / pin[2:]
                cloned = root / 'one/flask/.git/objects' / pin[:2] / pin[2:]
                self.assertNotEqual(original.stat().st_ino, cloned.stat().st_ino)
                task = json.loads((root / 'one/flask-tasks.json').read_text())[0]
                self.assertEqual(task['tests'][2], str(prepare.HERE / 'check.py'))
                with self.assertRaises(FileExistsError):
                    prepare.prepare(source.parent, root / 'one', Path(sys.executable), Path('/node'))


if __name__ == '__main__':
    unittest.main()
