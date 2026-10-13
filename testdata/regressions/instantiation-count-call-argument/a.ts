type D = "0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9";
type E = "0"|"1"|"2"|"3"|"4"|"5";
type U = `${D}${D}${D}${E}`;
type Chain<T, N extends unknown[] = []> = N["length"] extends 100 ? { value: T } : T extends string ? Chain<T, [...N, 0]> : never;
type Chain2<T, N extends unknown[] = []> = N["length"] extends 100 ? { other: T } : T extends string ? Chain2<T, [...N, 0]> : never;
declare function k<T>(x: Chain<T>): Chain2<T>;
declare const v: any;
const r = k<U>(v);
