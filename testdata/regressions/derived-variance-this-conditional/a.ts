// TSRS_DERIVED_VARIANCE fuzz finding: `this` in a conditional type's check type measures bivariant (guard 1 accepts
// it), yet the derived type resolves the conditional to the other branch. The type arguments are identical; only
// `this` differs between the base reference in D's chain and the target.
interface B<T> {
  kind: this extends { tag: 1 } ? 1 : 2
  v: T
}
interface D<T> extends B<T> { tag: 1 }
declare const d: D<string>
export const x: B<string> = d
