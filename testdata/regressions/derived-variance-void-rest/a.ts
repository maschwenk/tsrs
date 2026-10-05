// TSRS_DERIVED_VARIANCE fuzz finding: the `void` arity rule of derived-variance-void-arity, reached through a tuple
// argument spread into a rest parameter (`m(...a: T)` with `T = [void]`).
interface B<T extends unknown[]> {
  m(...a: T): void
}
interface D<T extends unknown[]> extends B<T> { d: 1 }
declare const d: D<[void]>
declare const b: B<[unknown]>
export const a = [d, b]
export const check: number = a
