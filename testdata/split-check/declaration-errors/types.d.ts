export interface Shape { [k: string]: Base }
export declare class Base { private brand: string; parse(x: unknown): unknown }
export declare class Str extends Base { s: string }
export declare class Num extends Base { n: number }
export declare class Obj<T extends Shape> extends Base { shape: T }
export declare const ok: Obj<{ a: Str; b: Num }>;
export declare const bad1: Obj<{ a: Str; b: number }>;
export interface Merged<T> { a: T }
export declare const ok2: Obj<{ c: Obj<{ d: Str }> }>;
export interface Merged<U> { b: U }
export declare const bad2: Obj<{ c: Obj<{ d: string }> }>;
export declare function over(x: string): string;
export declare const bad3: Obj<{ e: boolean }>;
export declare function over(x: number): number;
export declare const bad4: Obj<{ f: Obj<{ g: Str; h: null }> }>;
export type Alias<T extends string> = T;
export declare const bad5: Alias<number>;
export declare const dup: string;
export declare const bad6: Obj<{ i: Obj<{ j: Obj<{ k: 1 }> }> }>;
export declare const dup: number;
export declare const bad7: Missing;
export interface Merged<T> { c: T }
export declare const bad8: Obj<{ l: Str }, string>;
