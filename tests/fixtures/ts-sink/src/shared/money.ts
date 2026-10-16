import { round } from "./round";

export const cents = (n: number): number => round(n * 100);
