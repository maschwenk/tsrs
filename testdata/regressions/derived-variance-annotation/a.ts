// TSRS_DERIVED_VARIANCE fuzz finding: TypeScript trusts a variance annotation without measuring it (getVariances),
// so `B<"a">` -> `B<string>` holds by the (wrong) annotation, but `D<"a">` -> `B<string>` is compared member by
// member and fails. The derived shortcut extends the trust across the derivation and loses the error.
import type { B, D } from "./lib"
declare const b: B<"a">
export const x0: B<string> = b
declare const d: D<"a">
export const x1: B<string> = d
