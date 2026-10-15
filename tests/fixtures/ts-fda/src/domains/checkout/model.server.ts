import { TAX } from "./types";

export const charge = (cents: number): number => cents * (1 + TAX);
