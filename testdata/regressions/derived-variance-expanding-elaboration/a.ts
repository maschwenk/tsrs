// TSRS_DERIVED_VARIANCE fuzz finding (open, cosmetic): with an expanding recursive member (`B<T[]>`), deciding
// `D<string>` -> `B<string>` by variances skips comparisons whose relation-cache entries the later error elaboration
// meets; with the switch on, the last error is elaborated one recursion level deeper than tsgo does. The same
// errors are reported; only the message differs. (From tools/fuzz/derived_variance.py seed 117294, minimized.)
declare abstract class B<T> {
  next?: B<T[]>;
  f(x: T extends object ? keyof T : T): void;
}
declare abstract class D<T> extends B<T> { d1: T; d2: T; d3: T; }
declare const s1: D<string>;
export const t1: B<string> = s1;
declare const s2: B<string[]>;
export const t2: B<string[]> = s2;
declare const s3: D<null>;
export const t3: B<string | null> = s3;
