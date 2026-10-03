/// <reference path="./other.ts" />
/// <reference types="node" />
/// <reference lib="es2022" />
import type { A, B as C } from "./types";
import * as ns from "./ns";
import def, { type D, e as f } from "./mod" with { type: "json" };
import x = require("legacy");
export * from "./re";
export * as nsx from "./re2";
export { g as default, h } from "./re3";
declare module "augmented" {
    interface Augment { added: number }
}
declare global {
    interface Window { custom: string }
}
namespace N.M { export const k = 1; }
module Legacy { }
enum Color { Red = 1, Green = Red << 1, Blue = "blue".length }
const enum CE { A, B }
abstract class Base<T extends object = {}> implements Iterable<T> {
    #priv = 1;
    static readonly s?: number;
    protected abstract m(): void;
    declare d: string;
    accessor acc = 3;
    constructor(private readonly p: number, public q?: string) { super(); }
    get v(): number { return this.#priv; }
    set v(value) { this.#priv = value; }
    *[Symbol.iterator](): Iterator<T> { yield* []; }
    static { Base.s; }
    @dec() @dec2 method<const U>(this: Base<T>, ...rest: U[]): asserts this is Base<T> {}
    [key: string]: any;
}
type Mapped<T> = { readonly [K in keyof T as `get${Capitalize<string & K>}`]-?: () => T[K] };
type Cond<T> = T extends (infer U extends string)[] ? U : T extends Promise<infer V> ? V : never;
type Tpl = `a${number}b${string}c`;
type Tup = [a: string, b?: number, ...rest: boolean[]];
type Fn = new (...args: any[]) => unknown;
type Pred = (x: unknown) => x is string;
type Imp = typeof import("./mod", { with: { "resolution-mode": "import" } });
type U = A | B & C | keyof typeof ns | unique symbol | readonly string[];
let o = { a, b: 1, [c]: 2, ...d, m() {}, get g() { return 1; }, set g(v) {}, async *ag() { await 1; } };
let arr = [1, , 3, ...rest];
let re = /ab+c/gi, big = 123n, num = 0x1F_FF, oct = 0o17, s = 'single', t = `head ${1} middle ${2} tail`;
let tagged = tag`x${y}z`;
label: for (let i = 0; i < 10; i++) { if (i) continue label; else break; }
for (const k in o) {} for await (const v of gen()) {}
do { x++; --x; !x; ~x; -x; +x; typeof x; void 0; delete o.a; } while (false);
switch (x) { case 1: case 2: break; default: throw new Error(); }
try { f?.(); a?.b?.[c]; } catch ({ message }) {} finally {}
const [p1, { p2 = 1, ...p3 }] = q;
let cast = <string>v, as = v as const, sat = v satisfies object, nn = v!;
let arrow = async <T,>(a: T): Promise<T> => a, fnExpr = function* named() {};
x ??= y ||= z &&= w; x **= 2; x >>>= 1;
new.target; import.meta.url; import("dyn");
with (o) {}
debugger;
export default class {}
export = o;
