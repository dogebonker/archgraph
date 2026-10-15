import { create } from "../../application/create";
import { insert } from "../out/pg";

export const handle = (name: string): string => create(name) + insert(name);
