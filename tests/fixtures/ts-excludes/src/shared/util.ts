import { api } from "../generated/api";
import { schema } from "./schema.generated";

export const util = (s: string): string => s + api + schema;
