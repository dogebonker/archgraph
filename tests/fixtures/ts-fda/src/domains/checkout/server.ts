import { charge } from "./model.server";

export const chargeOrder = (cents: number): number => charge(cents);
