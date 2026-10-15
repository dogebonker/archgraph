import { loadCart } from "../controller";
import { TAX } from "../../checkout";
import { submit } from "../../checkout/client";

export const Cart = async (): Promise<string> => (await loadCart()) + TAX + submit();
