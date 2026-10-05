// TSRS_DERIVED_VARIANCE guard 1: the variance digest covers type parameters, not `this`. `Num` inherits
// `constraint: Constraint<this>` from `Runtype<number>` with `this` = `Num`; the target `Runtype<any>` has `this` =
// `Runtype<any>`, and `Constraint` is invariant in its argument, so the structural comparison fails even though
// `number` -> `any` relates by variances. (After microsoft/TypeScript tests/cases/compiler/invariantGenericErrorElaboration.ts.)
interface Runtype<A> {
  constraint: Constraint<this>
  witness: A
}
interface Num extends Runtype<number> {
  tag: 'number'
}
declare const Num: Num
interface Constraint<A extends Runtype<any>> extends Runtype<A['witness']> {
  underlying: A
  check: (x: A['witness']) => void
}
export const wat: Runtype<any> = Num
