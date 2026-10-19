from shop.inventory.storage import get_record


def on_hand(sku):
    return get_record(sku)["on_hand"]
