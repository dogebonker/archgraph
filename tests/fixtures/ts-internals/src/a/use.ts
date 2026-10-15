import { make } from "../b/make";
import { tool } from "../c/tool";

export const use = (): string => make() + tool();
