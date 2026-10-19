from shop.application.checkout import total, can_fulfill
from shop.orders.service import receipt
from shop.pricing import discounted_total


def checkout(subtotal, region):
    return total(subtotal, region)
