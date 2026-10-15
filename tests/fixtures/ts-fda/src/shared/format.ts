import { CURRENCY } from "../domains/cart/types";

export const format = (cents: number): string => `${cents / 100} ${CURRENCY}`;
