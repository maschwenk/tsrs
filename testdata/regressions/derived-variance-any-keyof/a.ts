// TSRS_DERIVED_VARIANCE fuzz finding: an `any` argument evaluates `keyof T` to `string | number | symbol` and a
// homomorphic mapped type to an index signature, where the variance markers keep both deferred (T measures
// invariant, and `any` is related to everything in both directions). TypeScript relates `B<any>` to `B<{ a: 1 }>` by
// variances, but `D<any>` to `B<{ a: 1 }>` member by member, and reports an error.
interface B<T> {
  k: keyof T
  m: { [K in keyof T]: 1 }
}
interface D<T> extends B<T> { d: 1 }
declare const b: B<any>
export const x0: B<{ a: 1 }> = b
declare const d: D<any>
export const x1: B<{ a: 1 }> = d
