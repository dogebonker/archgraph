import type { Item } from "./types";
import { DEFAULT_ID } from "./types";
import { log } from "../util/log";

export function save(item: Item): void {
  log(item.id === DEFAULT_ID ? "default" : item.id);
}
