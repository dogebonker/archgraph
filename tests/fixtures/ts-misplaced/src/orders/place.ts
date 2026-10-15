import { validate } from "./validate";
import { discount } from "./discount";

export const place = (n: number): number => (validate(n) ? discount(n) : 0);
