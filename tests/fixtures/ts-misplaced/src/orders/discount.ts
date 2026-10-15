import { rule } from "../pricing/rules";
import { rate } from "../pricing/rates";

export const discount = (n: number): number => n * rule() * rate();
