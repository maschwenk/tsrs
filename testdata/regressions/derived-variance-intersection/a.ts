// TSRS_DERIVED_VARIANCE fuzz finding: assignability is not monotone under intersection. `{}` is assignable to
// `{ [k: string]: string }` (an object literal type has an implicit index signature) and T measures covariant, but
// `{} & { z?: 1 }` reduces to `{ z?: 1 }`, whose `z` does not satisfy the index signature.
interface B<T> {
  i: T & { z?: 1 }
}
interface D<T> extends B<T> { d: 1 }
declare const b: B<{}>
export const x0: B<{ [k: string]: string }> = b
declare const d: D<{}>
export const x1: B<{ [k: string]: string }> = d
