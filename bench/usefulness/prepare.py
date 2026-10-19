"""Prepare a deterministic, private synthetic benchmark baseline; no agent runs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
CASES = {
    'tax': ('Add regional tax to composition.checkout(subtotal, region): return Decimal cents with ROUND_HALF_UP after subtotal * (1 + rate). Consume the existing mutable adapters.tax.RATES via rate_for, including changes after import, unknown regions at zero, and negative amounts. Keep application.total(subtotal, region) callable with its existing arguments. Preserve other behavior.', ['src/shop/application/**', 'src/shop/composition.py']),
    'inventory': ('Make application.checkout.can_fulfill(sku, quantity) use sellable stock: max(0, on_hand - reserved). Unknown SKUs and nonpositive quantities return False; preserve inventory.api.on_hand and unrelated behavior. Changes to storage records after import must be reflected.', ['src/shop/application/**', 'src/shop/inventory/**']),
    'receipt': ('Include the canonical human status label in notifications.formatter.format_receipt(order) and orders.service.receipt(order): exactly "Order <id>: <total to 2 decimals> [<label>]". Missing/unknown status uses "Unknown status". Canonical labels currently live in orders.service; retain its STATUS_LABELS and status_label public API and reflect later table mutations. Keep both receipt entrypoints callable with one order argument and preserve unrelated behavior.', ['src/shop/orders/**', 'src/shop/notifications/**', 'src/shop/shared/**']),
    'discount': ('Implement pricing.discounted_total(unit_price, quantity, percent) as unit_price * quantity * (1 - percent / 100), using Decimal from string inputs and ROUND_HALF_UP cents once at the end. Support fractional percentages and zero quantity. Reject negative quantity or percentages outside 0..100 with ValueError. Preserve pricing.money and unrelated behavior.', ['src/shop/pricing.py']),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(root, *args):
    env = dict(os.environ, GIT_AUTHOR_DATE='2026-10-20T00:00:00+00:00',
               GIT_COMMITTER_DATE='2026-10-20T00:00:00+00:00', GIT_CONFIG_NOSYSTEM='1',
               GIT_CONFIG_GLOBAL=os.devnull)
    return subprocess.check_output(['git', '-C', str(root), *args], env=env, text=True).strip()


def prepare(output, python):
    output = Path(output).resolve()
    python = Path(python).resolve(strict=True)
    if output == HERE.parents[1] or HERE.parents[1] in output.parents:
        raise ValueError('output must be outside the repository')
    output.mkdir(parents=True, exist_ok=False)
    output.chmod(0o700)
    baseline = output / 'baseline'
    shutil.copytree(HERE / 'fixture', baseline)
    git(baseline, 'init', '--quiet', '--initial-branch=main')
    git(baseline, 'add', '.')
    git(baseline, '-c', 'commit.gpgsign=false', '-c', 'user.name=Archgraph benchmark',
        '-c', 'user.email=benchmark@example.invalid', 'commit', '--quiet', '-m',
        'Freeze declared synthetic mini-shop benchmark')
    oracle = HERE / 'oracle.py'
    tasks = []
    for name, (request, area) in CASES.items():
        tests = [str(python), '-B', str(oracle), name]
        tasks.append(dict(id=name, prompt=request + '\nWork inside this checkout. Follow layout.yaml; do not modify it. Execute the supplied external oracle, but do not inspect benchmark preparation or reference solutions. Run the feature check: '
                          + ' '.join(tests), area=area, tests=tests))
    task_path = output / 'tasks.json'
    task_path.write_text(json.dumps(tasks, indent=2) + '\n')
    hashes = {str(p.relative_to(HERE / 'fixture')): digest(p)
              for p in sorted((HERE / 'fixture').rglob('*')) if p.is_file()}
    manifest = dict(baseline=str(baseline), baseline_commit=git(baseline, 'rev-parse', 'HEAD'),
                    tasks=str(task_path), tasks_sha256=digest(task_path), python=str(python),
                    oracle=str(oracle), oracle_sha256=digest(oracle),
                    prepare_sha256=digest(Path(__file__)), fixture_sha256=hashes,
                    policy='deliberately authored synthetic benchmark; not upstream truth',
                    declared_repeats=3, arms=['bare', 'guarded'],
                    inference='controlled challenge only; no real-world efficacy claim')
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='fresh directory outside the repository')
    parser.add_argument('--python', type=Path, default=Path(sys.executable))
    args = parser.parse_args()
    print(json.dumps(prepare(args.output, args.python), indent=2))
