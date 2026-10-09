// A union of 1,101 classes for removeSubtypes (#218): every R<K> extends B and is removed after one comparison with
// B, every S<K> survives after comparing with all the others. TS2590's estimate is taken after 100,000 comparisons, so
// it depends on which constituents come first. Union constituents are kept in compareTypes order (names, then type
// arguments), not in the order a checker created the types, so the estimate starts with the S<K> types and reports
// TS2590 whichever of a.ts and b.ts a checker saw first. In program order b.ts creates the S<K> types first: with
// constituents in creation order (as TypeScript 5.9 keeps them) the estimate would start with the R<K> types and stay
// far below the limit, so the single-threaded run prints nothing; the determinism gate's assignments that check a.ts
// first on b.ts's checker cover the other order.
declare class B { kind: string }
declare class S<K> { s: K }
declare class R<K> extends B { r: K }
type D = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";
type Ss = { [K in `${D}${D}`]: S<K> }[`${D}${D}`];
type Rs = { [K in `${D}${D}${D}`]: R<K> }[`${D}${D}${D}`];
