// TSRS_DERIVED_VARIANCE fuzz finding: `{}` and `{ a?: string }` are assignable to each other (optional properties
// may be absent; an empty source passes the weak-type check), so they satisfy any variance, but `keyof` tells them
// apart. No `any`, no conditional type: `keyof T` (contravariant by the markers) is not monotone in assignability.
interface B<T> {
  k: keyof T
}
interface D<T> extends B<T> { d: 1 }
declare const b: B<{ a?: string }>
export const x0: B<{}> = b
declare const d: D<{ a?: string }>
export const x1: B<{}> = d
