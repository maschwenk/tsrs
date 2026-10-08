export const key: unique symbol = Symbol();
export class A { #secret = 1; static readonly [key] = 2; has(o: unknown) { return #secret in (o as object); } }
