"""External canonical behavior oracle. Import full packages, never extracted AST."""
from decimal import Decimal
from pathlib import Path
import sys


def check(case):
    sys.path.insert(0, str(Path.cwd() / 'src'))
    from shop import composition, pricing
    from shop.application import checkout
    from shop.adapters import tax
    from shop.inventory import api, storage
    from shop.orders import service
    from shop.notifications import formatter

    # Existing APIs and unrelated paths must continue to work for every task.
    assert api.on_hand('book') == 12 and api.on_hand('missing') == 0
    assert pricing.money('1.005') == Decimal('1.01')
    assert service.status_label('paid') == 'Payment received'
    order = dict(id='A7', total=Decimal('12.30'), status='paid')
    if case == 'tax':
        for rate in ('0.23', '0.1875', '0', '0.041'):
            tax.RATES['PT'] = Decimal(rate)
            for amount in ('100', '1.005', '-12.37', '0'):
                expected = (Decimal(amount) * (1 + Decimal(rate))).quantize(Decimal('.01'), rounding='ROUND_HALF_UP')
                assert composition.checkout(amount, 'PT') == expected, 'checkout must consume live adapter rates'
        assert composition.checkout('1.005', 'unknown') == Decimal('1.01')
        assert checkout.total('10', 'unknown') == Decimal('10.00')
        assert formatter.format_receipt(order) == 'Order A7: 12.30'
    elif case == 'inventory':
        for held in (0, 5, 12, 18):
            storage.STOCK['book']['reserved'] = held
            for quantity in (-1, 0, 1, 7, 8, 12, 13):
                expected = quantity > 0 and max(0, 12 - held) >= quantity
                assert checkout.can_fulfill('book', quantity) is expected, 'must respect live reservations'
        assert checkout.can_fulfill('unknown', 1) is False
        assert checkout.can_fulfill('pen', 1) is False
        assert composition.checkout('10', 'PT') == Decimal('10.00')
    elif case == 'receipt':
        for status, label in [('pending', 'Awaiting payment'), ('paid', 'Payment received'),
                              ('shipped', 'On its way'), ('other', 'Unknown status')]:
            order['status'] = status
            expected = f'Order A7: 12.30 [{label}]'
            assert formatter.format_receipt(order) == expected, 'formatter must include canonical status'
            assert service.receipt(order) == expected
        service.STATUS_LABELS['paid'] = 'Settled today'
        order['status'] = 'paid'
        assert formatter.format_receipt(order) == 'Order A7: 12.30 [Settled today]'
        assert service.status_label('paid') == 'Settled today'
        del order['status']
        assert service.receipt(order) == 'Order A7: 12.30 [Unknown status]'
    elif case == 'discount':
        for price, count, percent, expected in [('1.005', 1, 0, '1.01'), ('10', 3, '12.5', '26.25'),
                                               ('0.335', 3, 0, '1.01'), ('7.99', 0, 20, '0.00'),
                                               ('19.99', 2, 100, '0.00')]:
            result = pricing.discounted_total(price, count, percent)
            assert type(result) is Decimal and result == Decimal(expected), 'discount or final rounding incorrect'
        for count, percent in [(-1, 10), (1, -1), (1, 101)]:
            try:
                pricing.discounted_total('10', count, percent)
            except ValueError:
                pass
            else:
                raise AssertionError('invalid quantity/percentage must raise ValueError')
    else:
        raise ValueError(case)
    print(f'{case}: full-package behavior oracle passed')


if __name__ == '__main__':
    check(sys.argv[1])
