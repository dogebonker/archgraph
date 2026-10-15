import { rule } from "./rules";
import { rate } from "./rates";

export const quote = (): number => rule() * rate();
