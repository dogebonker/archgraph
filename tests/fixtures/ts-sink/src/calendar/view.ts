import { hour } from "../shared/clock";
import { today } from "../shared/dates";

export const view = (): string => `${today()} ${hour()}`;
