import { valid } from "./rules";
import { insert } from "../adapters/out/pg";

export const user = (name: string): string => (valid(name) ? insert(name) : "");
