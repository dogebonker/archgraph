"""Discarded reference implementations prove behavior/policy independence, no agents."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import prepare

HERE = Path(__file__).resolve().parent
BINARY = Path(os.environ.get('BOUNDRAIL_TEST_BINARY', HERE.parents[1] / 'target/debug/boundrail'))


def write(root, name, value):
    path = root / 'src/shop' / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value)


def replace(root, name, old, new):
    path = root / 'src/shop' / name
    original = path.read_text()
    assert old in original
    path.write_text(original.replace(old, new))


def implement(root, case, tempting=False):
    """Reference patches exist only here, outside the copied agent fixture."""
    if case == 'tax':
        replace(root, 'application/checkout.py', 'from decimal import Decimal', 'from decimal import Decimal, ROUND_HALF_UP')
        replace(root, 'application/checkout.py', 'def total(subtotal, region):\n    return Decimal(str(subtotal)).quantize(Decimal("0.01"))',
                'def total(subtotal, region, rate_for=lambda region: Decimal("0")):\n'
                '    return (Decimal(str(subtotal)) * (1 + rate_for(region))).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)')
        if tempting:
            replace(root, 'application/checkout.py', 'from shop.inventory.api import on_hand',
                    'from shop.inventory.api import on_hand\nfrom shop.adapters.tax import rate_for')
            replace(root, 'application/checkout.py', 'rate_for=lambda region: Decimal("0")', 'rate_for=rate_for')
        else:
            replace(root, 'composition.py', 'from shop.application.checkout import total, can_fulfill',
                    'from shop.application.checkout import total, can_fulfill\nfrom shop.adapters.tax import rate_for')
            replace(root, 'composition.py', 'return total(subtotal, region)', 'return total(subtotal, region, rate_for)')
    elif case == 'inventory':
        if tempting:
            replace(root, 'application/checkout.py', 'from shop.inventory.api import on_hand',
                    'from shop.inventory.api import on_hand\nfrom shop.inventory.storage import get_record')
            replace(root, 'application/checkout.py', 'return on_hand(sku) >= quantity',
                    'record = get_record(sku)\n    return max(0, record["on_hand"] - record["reserved"]) >= quantity')
        else:
            path = root / 'src/shop/inventory/api.py'
            path.write_text(path.read_text() + '\n\ndef available(sku):\n    record = get_record(sku)\n    return max(0, record["on_hand"] - record["reserved"])\n')
            replace(root, 'application/checkout.py', 'from shop.inventory.api import on_hand', 'from shop.inventory.api import available')
            replace(root, 'application/checkout.py', 'return on_hand(sku) >= quantity', 'return available(sku) >= quantity')
    elif case == 'receipt':
        if tempting:
            write(root, 'notifications/formatter.py', 'import shop.orders.service as orders\n\ndef format_receipt(order):\n'
                  '    return f"Order {order[\'id\']}: {order[\'total\']:.2f} [{orders.status_label(order.get(\'status\'))}]"\n')
        else:
            service = root / 'src/shop/orders/service.py'
            original = service.read_text()
            labels = original[original.index('STATUS_LABELS'):original.index('\n\ndef receipt')]
            write(root, 'shared/__init__.py', '')
            write(root, 'shared/status.py', labels + '\n')
            service.write_text('from shop.notifications.formatter import format_receipt\n'
                               'from shop.shared.status import STATUS_LABELS, status_label\n\ndef receipt(order):\n    return format_receipt(order)\n')
            write(root, 'notifications/formatter.py', 'from shop.shared.status import status_label\n\ndef format_receipt(order):\n'
                  '    return f"Order {order[\'id\']}: {order[\'total\']:.2f} [{status_label(order.get(\'status\'))}]"\n')
    elif case == 'discount':
        replace(root, 'pricing.py', 'return money(Decimal(str(unit_price)) * quantity)',
                'percent = Decimal(str(percent))\n    if quantity < 0 or not 0 <= percent <= 100:\n'
                '        raise ValueError("invalid quantity or percentage")\n'
                '    return money(Decimal(str(unit_price)) * quantity * (1 - percent / 100))')
    else:
        raise ValueError(case)


class UsefulnessPreflight(unittest.TestCase):
    def test_declared_challenges(self):
        self.assertTrue(BINARY.is_file(), 'build boundrail or set BOUNDRAIL_TEST_BINARY first')
        with tempfile.TemporaryDirectory(prefix='boundrail-usefulness-preflight-') as temporary:
            parent = Path(temporary)
            manifest = prepare.prepare(parent / 'prepared', Path(sys.executable))
            baseline = Path(manifest['baseline'])
            # Deterministic Git commit, independently prepared at another absolute path.
            duplicate = prepare.prepare(parent / 'duplicate', Path(sys.executable))
            self.assertEqual(manifest['baseline_commit'], duplicate['baseline_commit'])
            self.assertFalse((baseline / 'oracle.py').exists())
            baseline_check = subprocess.run([str(BINARY), '--no-daemon', 'mcp-call', '.',
                                             '--engine', 'native', '--symbols', '--package', 'shop',
                                             '--tool', 'verify', '--arguments',
                                             json.dumps(dict(expect=dict(cycles_in=['application', 'adapters', 'inventory', 'orders', 'notifications', 'composition', 'pricing'], equals=0, forbidden=0), brief=False))],
                                            cwd=baseline, capture_output=True, text=True)
            self.assertEqual(baseline_check.returncode, 0, baseline_check.stderr)
            self.assertTrue(json.loads(baseline_check.stdout)['pass'], baseline_check.stdout)
            tasks = json.loads(Path(manifest['tasks']).read_text())
            for task in tasks:
                case = task['id']
                with self.subTest(case=case, implementation='baseline'):
                    result = subprocess.run(task['tests'], cwd=baseline, capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0, 'baseline already passes requested feature')
                    self.assertIn('AssertionError', result.stderr)
            for case in prepare.CASES:
                for tempting in ([False, True] if case != 'discount' else [False]):
                    with self.subTest(case=case, tempting=tempting):
                        checkout = parent / f'{case}-{tempting}'
                        subprocess.run(['git', 'clone', '--quiet', '--no-hardlinks', str(baseline), str(checkout)], check=True)
                        implement(checkout, case, tempting)
                        result = subprocess.run([sys.executable, '-B', str(HERE / 'oracle.py'), case],
                                                cwd=checkout, capture_output=True, text=True)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        result = subprocess.run([str(BINARY), '--no-daemon', 'mcp-call', '.',
                                                 '--engine', 'native', '--symbols', '--package', 'shop',
                                                 '--tool', 'check_diff', '--arguments',
                                                 json.dumps(dict(base=manifest['baseline_commit'], brief=False))],
                                                cwd=checkout, capture_output=True, text=True)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        data = json.loads(result.stdout)
                        violations = data['new_forbidden'] + data['new_cycles'] + data['grown_cycles']
                        if not tempting:
                            self.assertEqual(violations, [], data)
                        elif case == 'receipt':
                            self.assertTrue(data['new_cycles'], data)
                            self.assertTrue(any(set(cycle['members']) == {'notifications', 'orders'}
                                                for cycle in data['new_cycles']), data)
                        else:
                            expected_rule = 'application-no-adapters' if case == 'tax' else 'entrypoints'
                            self.assertTrue(any(edge['rule'] == expected_rule
                                                for edge in data['new_forbidden']), data)
                        print(f'{case} {"tempting" if tempting else "correct"}: behavior PASS; '
                              f'native architecture {"BLOCK" if violations else "PASS"}')


if __name__ == '__main__':
    unittest.main()
