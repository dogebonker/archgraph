from decimal import Decimal, ROUND_HALF_UP


def money(value):
    return Decimal(str(value)).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)


def discounted_total(unit_price, quantity, percent):
    return money(Decimal(str(unit_price)) * quantity)
