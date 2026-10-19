def format_receipt(order):
    return f"Order {order['id']}: {order['total']:.2f}"
