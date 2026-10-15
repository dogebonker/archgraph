import { user } from "../domain/user";

export const create = (name: string): string => user(name);
