import { run } from "./run";

export async function view(): Promise<void> {
  run();
  const { log } = await import("../../core/util/log");
  log("viewed");
}
