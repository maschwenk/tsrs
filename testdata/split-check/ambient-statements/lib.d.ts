// Two executable statements at the top level: only the first reports TS1036.
declare const a: number;
declare const b: string;
a;
declare function f(x: number): void;
declare const c: boolean;
b;
interface I { x: number }
declare const d: I;
