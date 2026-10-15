import type { Item } from "../../core/model/types";
import { save } from "../../core/model/store";
import { log } from "../../core/util/log";

export function run(): void {
  const item: Item = { id: "1" };
  save(item);
  log("ran");
}
