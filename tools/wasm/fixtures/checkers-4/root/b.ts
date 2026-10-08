import { A, key } from "./a";
const sym2: unique symbol = Symbol();
export const v: string = A[key];
export class B extends A { #secret = "x"; get [sym2]() { return this.#secret; } }
