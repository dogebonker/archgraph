import { writeSync } from "node:fs";
import pad from "left-pad";

export function log(message: string): void {
  writeSync(1, pad(message, 8) + "\n");
}
