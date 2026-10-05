// TSRS_DERIVED_VARIANCE guard 3: an `any` argument resolves a conditional type on its parameter to both branches,
// where the markers that measured the variances kept it deferred. TypeScript relates `H<any>` to `H<unknown>` by
// variances (no error) but `E<any>` to `H<unknown>` structurally (an error); the derived comparison must keep the
// error. (The shape of playwright's `JSHandle<T>.asElement()`.)
interface Node0 { n: 1 }
interface H<T = any> {
  asElement(): T extends Node0 ? E<T> : null
  v: T
}
interface E<T = Node0> extends H<T> { e: 1 }
declare const a: H<any>
export const b: H<unknown> = a
declare const e: E<any>
export const b2: H<unknown> = e
