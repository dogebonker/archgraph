from decimal import Decimal
from shop.inventory.api import on_hand


def total(subtotal, region):
    return Decimal(str(subtotal)).quantize(Decimal("0.01"))


def can_fulfill(sku, quantity):
    if quantity <= 0:
        return False
    return on_hand(sku) >= quantity
