// make(...) takes about 5,027,000 instantiations, over the 5M per-expression limit. In Go, b.ts's `id(big)` is also
// charged for them when it is the first to check this initializer (b.ts checked on a checker that has not checked
// a.ts), and reports TS2589 too (notes/open-history-dependence.md section 5).
type D = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";
type U = `${D}${D}${D}${D}`;
type Chain<T, N extends unknown[] = []> = N["length"] extends 100 ? { value: T } : T extends string ? Chain<T, [...N, 0]> : never;
declare function make<T>(f: (x: T) => void): Chain<T>;
const big = make((x: U) => {});
