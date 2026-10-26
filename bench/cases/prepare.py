"""Prepare clean local benchmark baselines without modifying the upstream clones."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SOURCES = {row['name']: row for row in json.loads((ROOT / 'showcase/repos.json').read_text())}
CASES = {
    'flask': ('src/flask/helpers.py', 'src/flask/app.py', 'src/flask/**',
              'Extend get_load_dotenv so FLASK_SKIP_DOTENV is stripped of surrounding whitespace before interpretation. Whitespace-only values use the default argument, like an unset value. Preserve case-insensitive 0/false/no behavior and all other existing behavior.'),
    'httpx': ('httpx/_utils.py', 'httpx/_client.py', 'httpx/**',
              'Extend to_bytes to accept bytearray as well as str and bytes. Return an immutable bytes copy for bytearray inputs, including empty ones; subsequent mutations must not affect the returned bytes. Preserve str encoding behavior and bytes object identity; update the input annotation.'),
    'pydantic': ('pydantic/_internal/_utils.py', 'pydantic/main.py', 'pydantic/**',
                 'Extend unique_list with a keyword-only keep_first: bool = False option. When true, retain the first item for each name_factory result in first-name order; when false or omitted, preserve existing last-value replacement and first-name ordering. Keep tuple input and custom name_factory support; do not mutate input.'),
    'trpc': ('packages/server/src/unstable-core-do-not-import/utils.ts', 'packages/server/src/unstable-core-do-not-import/router.ts', 'packages/server/src/**',
             'Tighten isAsyncIterable so an object qualifies only when its Symbol.asyncIterator member is callable. An object with that symbol set to a number or undefined must return false. Preserve null rejection, normal object rejection, valid async generators and callable custom iterator support, and the existing type predicate signature.'),
}


def git(root, *args):
    environment = dict(os.environ, GIT_AUTHOR_DATE='2026-10-19T00:00:00+00:00', GIT_COMMITTER_DATE='2026-10-19T00:00:00+00:00')
    return subprocess.check_output(['git', '-C', str(root), *args], text=True, env=environment).strip()


def prepare(source_dir, destination, python, node):
    destination.mkdir(parents=True, exist_ok=True)
    result = []
    for name, (file, outer, package, prompt) in CASES.items():
        upstream = SOURCES[name]
        source = source_dir / name
        assert git(source, 'rev-parse', 'HEAD') == upstream['commit'], f'{name}: wrong source pin'
        target = destination / name
        if target.exists():
            raise FileExistsError(f'refusing to replace {target}')
        subprocess.run(['git', 'clone', '--quiet', '--no-hardlinks', '--no-checkout', str(source), str(target)], check=True)
        git(target, 'checkout', '--quiet', '--detach', upstream['commit'])
        if (target / 'layout.yaml').exists() or (target / '.benchmark').exists():
            raise FileExistsError('upstream already owns benchmark output paths')
        fixture = target / '.benchmark'
        fixture.mkdir()
        shutil.copyfile(HERE / 'check.py', fixture / 'check.py')
        # This is an explicitly benchmark-authored utility-to-API boundary, not upstream intent.
        layout = (f'# Synthetic benchmark policy; not an upstream architectural contract.\nversion: 1\n'
                  f'exclude: [".benchmark/**", "tests/**", "docs/**", "examples/**", "pydantic/v1/**"]\n'
                  f'modules:\n- id: {name}\n  path: "{package}"\n'
                  f'forbid:\n- id: utility-no-outer-api\n  from: "{file}"\n  to: "{outer}"\n'
                  f'  reason: Keep the utility independent of the package outer API in this synthetic benchmark.\n')
        (target / 'layout.yaml').write_text(layout)
        provenance = dict(upstream=upstream['repo'], upstream_commit=upstream['commit'],
                          role='showcase' if name in ('flask', 'trpc') else 'public substitute; Perf tasks unavailable',
                          policy='benchmark-authored; not upstream intent',
                          layout_sha256=hashlib.sha256(layout.encode()).hexdigest(),
                          oracle='isolated feature check; not upstream test suite')
        (fixture / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
        git(target, 'add', 'layout.yaml', '.benchmark')
        git(target, '-c', 'commit.gpgsign=false', '-c', 'user.name=Boundrail benchmark', '-c', 'user.email=benchmark@example.invalid', 'commit', '--quiet', '-m', 'Prepare declared synthetic benchmark policy and oracle')
        tests = [str(python), '-B', str(HERE / 'check.py'), name]
        if name == 'trpc':
            tests.append(str(node))
        tasks = [dict(id=f'{name}-utility', prompt=f'{prompt} Expected implementation area: {file}. Do not change the benchmark oracle or layout.yaml. Run the isolated feature check: ' + ' '.join(tests), area=[file], tests=tests)]
        (destination / f'{name}-tasks.json').write_text(json.dumps(tasks, indent=2) + '\n')
        result.append(dict(name=name, baseline=str(target), baseline_commit=git(target, 'rev-parse', 'HEAD'), **provenance))
    (destination / 'manifest.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--python', type=Path, required=True)
    parser.add_argument('--node', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.sources.resolve(), args.output.resolve(), args.python.resolve(), args.node.resolve()), indent=2))
