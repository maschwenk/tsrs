// Member lists here are parsed and bound on first use when the CLI does not check declaration files
// (notes/mem-lazy-dts-members.md). Each block below exercises one way a lazy list could differ from an eager one.

// `infer` inside a type literal declares its type parameter in the enclosing conditional type: such a list must not
// be lazy, or `M` would be missing from the conditional type's locals when the conditional type is created (the type
// literal is a mapped type's template, which the checker resolves later).
export type AllModels<Args, K extends PropertyKey> = Args extends { [P in K]: { $all: infer M; other: string } } ? M : never;

// Two declarations of one interface in one file, with different type parameter names: the members table must list
// `a`, `a2`, `b`, `b2` in source order (the error below prints the missing properties in that order).
export interface Merged<T> { a: T; a2: T }
export interface Merged<U> { b: U; b2: U }

// A class whose static members merge with a namespace: `exports` must be filled in source order.
export declare class Widget { static first: number; static second: string; name: string }
export declare namespace Widget { const third: boolean; }

// An interface merged with a value and a namespace: the namespace fills `exports`, which the interface list does not.
export interface Config { verbose: boolean; level: number }
export declare const Config: { create(): Config };
export declare namespace Config { const defaults: Config; }

// A nested type literal inside a lazy list, and one inside a function type parameter.
export interface Outer { inner: { deep: { leaf: string } }; fn(arg: { x: number; y: number }): void }

// Lists that stay eager: `this`, an import type, a computed name.
export interface Fluent { self(): this; count: number }
export interface WithImport { mod: typeof import("./other"); n: number }
export interface Computed { [Symbol.iterator](): Iterator<number>; size: number }
