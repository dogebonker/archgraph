from shop.notifications.formatter import format_receipt

STATUS_LABELS = {"pending": "Awaiting payment", "paid": "Payment received",
                 "shipped": "On its way"}


def status_label(status):
    return STATUS_LABELS.get(status, "Unknown status")


def receipt(order):
    return format_receipt(order)
