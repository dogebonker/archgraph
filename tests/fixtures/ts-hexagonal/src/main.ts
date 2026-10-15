import { handle } from "./adapters/in/http";
import { insert } from "./adapters/out/pg";

handle(insert("boot"));
