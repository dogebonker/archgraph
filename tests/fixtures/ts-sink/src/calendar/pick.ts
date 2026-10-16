import { hour } from "../shared/clock";
import { today } from "../shared/dates";

export const pick = (): string => `${today()} ${hour()}`;
