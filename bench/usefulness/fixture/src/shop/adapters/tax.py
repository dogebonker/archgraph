from decimal import Decimal

RATES = {"PT": Decimal("0.23"), "US": Decimal("0.07")}


def rate_for(region):
    return RATES.get(region, Decimal("0"))
