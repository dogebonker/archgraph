import { fetchCart } from "./client";
import { now } from "../../lib/clock";

export const loadCart = async (): Promise<string> => (await fetchCart()) + now();
