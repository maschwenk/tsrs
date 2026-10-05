// TSRS_DERIVED_VARIANCE fuzz finding: an `any` argument in a template literal type gives `a${any}`, which is not
// assignable to `"ab"`, while the markers kept the template literal deferred.
interface T0<T extends string> {
  t: `a${T}`
}
interface D<T extends string> extends T0<T> { d: 1 }
declare const b: T0<any>
export const x0: T0<"b"> = b
declare const d: D<any>
export const x1: T0<"b"> = d
