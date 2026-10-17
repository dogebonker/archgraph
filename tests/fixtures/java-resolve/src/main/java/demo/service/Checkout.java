package demo.service;

import static demo.model.Order.LIMIT;
import demo.model.Factory;

public class Checkout extends Factory {
    demo.model.Order.Item item;
    public String checkout() { return create().id() + LIMIT; }
}
