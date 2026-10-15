import { order } from "../domain/order";
import { y } from "../../b/domain/y";

export const x = (): string => order() + y();
