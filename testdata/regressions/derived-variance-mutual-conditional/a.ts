// TSRS_DERIVED_VARIANCE fuzz finding: in a conditional type's check type the markers make a type parameter
// bivariant (deferred conditionals relate when their check types relate either way), but two arguments that are
// assignable to each other can still resolve the conditional to different branches.
interface C<T> {
  c: keyof T extends never ? 1 : 2
}
interface E<T> extends C<T> { e: 1 }
declare const c: C<{}>
export const x0: C<{ a?: string }> = c
declare const e: E<{}>
export const x1: C<{ a?: string }> = e
