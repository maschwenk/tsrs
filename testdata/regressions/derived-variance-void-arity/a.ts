// TSRS_DERIVED_VARIANCE fuzz finding: under the strict subtype relation (union subtype reduction) a trailing
// parameter whose type contains `void` is optional for the arity check, so `(x: void) => void` is not a strict
// subtype of `(x: unknown) => void`. The variance markers are never `void`: TypeScript reduces `[B<void>, B<unknown>]`
// to `B<unknown>[]` by variances, but keeps `D<void>` in `[D<void>, B<unknown>]`; the shortcut reduced it away.
interface B<T> {
  m(x: T): void
}
interface D<T> extends B<T> { d: T }
declare const d: D<void>
declare const b: B<unknown>
export const a = [d, b]
export const check: number = a
