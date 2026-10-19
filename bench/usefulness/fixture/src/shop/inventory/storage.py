STOCK = {"book": {"on_hand": 12, "reserved": 5},
         "pen": {"on_hand": 4, "reserved": 4}}


def get_record(sku):
    return STOCK.get(sku, {"on_hand": 0, "reserved": 0}).copy()
