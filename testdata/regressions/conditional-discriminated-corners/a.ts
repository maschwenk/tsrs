// Conditional types over discriminated unions: the per-constituent "definitely false" test of getConditionalType for
// discriminants of every kind (notes/perf-checker-algorithms.md). Every result is printed through an error.

enum NumE { A = 1, B = 2 }
enum StrE { A = 'a', B = 'b' }
declare const sym1: unique symbol;
declare const sym2: unique symbol;

type U1 =
  | { kind: 1; a: string }
  | { kind: 2; b: string }
  | { kind: NumE.A; c: string }
  | { kind: 'a'; d: string }
  | { kind: StrE.B; e: string }
  | { kind: typeof sym1; f: string }
  | { kind: true; g: string }
  | { kind: null; h: string }
  | { kind: undefined; i: string }
  | { kind: never; j: string }
  | { kind?: 'opt'; k: string }
  | ({ kind: 'x' } & { l: string })
  | ({ kind: 'y' } & { kind: string; m: string })
  | (string & { kind: 'branded' });

// A numeric literal against a numeric enum member and back.
type R1 = Extract<U1, { kind: NumE.A }>;
type R2 = Extract<U1, { kind: 1 }>;
// A string literal against a string enum member and back.
type R3 = Extract<U1, { kind: StrE.A }>;
type R4 = Extract<U1, { kind: 'b' }>;
// Unique symbols, booleans, null, undefined, never.
type R5 = Extract<U1, { kind: typeof sym1 }>;
type R6 = Extract<U1, { kind: typeof sym2 }>;
type R7 = Extract<U1, { kind: boolean }>;
type R8 = Extract<U1, { kind: null }>;
type R9 = Extract<U1, { kind: undefined }>;
type R10 = Extract<U1, { kind: never }>;
// Optional on either side; a weak target (every property optional).
type R11 = Extract<U1, { kind: 'opt' }>;
type R12 = Extract<U1, { kind?: 'opt' }>;
// Intersections, including a branded primitive.
type R13 = Extract<U1, { kind: 'x' }>;
type R14 = Extract<U1, { kind: 'y'; m: string }>;
type R15 = Extract<U1, { kind: 'branded' }>;
// Exclude: the false branch is the constituent itself.
type R16 = Exclude<U1, { kind: 'a' | 'x' }>;
type R17 = Exclude<U1, { kind: string }>;
// A primitive or union-typed discriminant on the target.
type R18 = Extract<U1, { kind: number }>;
type R19 = Extract<U1, { kind: 1 | 2 | 'a' | 'b' | 'x' }>;
// A target with an index signature, and one whose discriminant is not its first property.
type R20 = Extract<U1, { kind: 'a'; [k: string]: unknown }>;
type R21 = Extract<U1, { d: string; kind: 'a' }>;

// Chains of conditionals and hand-written distributions.
type Classify<T> = T extends { kind: 1 } ? 'one' : T extends { kind: 'a' } ? 'letter' : T extends { kind: true } ? 'yes' : 'other';
type R22 = Classify<U1>;
type PickKind<T, K> = T extends { kind: K } ? T : never;
type R23 = PickKind<U1, 2>;
type Payload<T, K> = T extends { kind: K } ? { wrapped: T } : { other: T['kind' & keyof T] };
type R24 = Payload<U1, 'a'>;

// Class instances with private and protected members as targets.
class Priv { private p = 1; kind: 'cls' = 'cls'; }
class Prot { protected q = 1; kind: 'cls2' = 'cls2'; }
type U2 = Priv | Prot | { kind: 'cls'; p: number } | { kind: 'cls2'; q: number };
type R25 = Extract<U2, Priv>;
type R26 = Extract<U2, Prot>;
type R27 = Extract<U2, { kind: 'cls' }>;

// Generic aliases on both sides (alias variance probing).
type Tagged<K extends string> = { kind: K; tag: true };
type U3 = Tagged<'a'> | Tagged<'b'> | { kind: 'c'; tag: true };
type R28 = Extract<U3, Tagged<'a'>>;
type R29 = Extract<U3, Tagged<'c'>>;

// A mapped type over the keys, the shape that repeats the per-constituent test N x M times.
type ByKind = { [K in 'a' | 'x' | 'y' | 'opt' | 'b']: Extract<U1, { kind: K }> };

declare function show<T>(x: T): void;
type Nope = { nope: true };
show<Nope>(null! as R1);
show<Nope>(null! as R1['kind' & keyof R1]);
show<Nope>(null! as R2);
show<Nope>(null! as R2['kind' & keyof R2]);
show<Nope>(null! as R3);
show<Nope>(null! as R3['kind' & keyof R3]);
show<Nope>(null! as R4);
show<Nope>(null! as R4['kind' & keyof R4]);
show<Nope>(null! as R5);
show<Nope>(null! as R5['kind' & keyof R5]);
show<Nope>(null! as R6);
show<Nope>(null! as R6['kind' & keyof R6]);
show<Nope>(null! as R7);
show<Nope>(null! as R7['kind' & keyof R7]);
show<Nope>(null! as R8);
show<Nope>(null! as R8['kind' & keyof R8]);
show<Nope>(null! as R9);
show<Nope>(null! as R9['kind' & keyof R9]);
show<Nope>(null! as R10);
show<Nope>(null! as R10['kind' & keyof R10]);
show<Nope>(null! as R11);
show<Nope>(null! as R11['kind' & keyof R11]);
show<Nope>(null! as R12);
show<Nope>(null! as R12['kind' & keyof R12]);
show<Nope>(null! as R13);
show<Nope>(null! as R13['kind' & keyof R13]);
show<Nope>(null! as R14);
show<Nope>(null! as R14['kind' & keyof R14]);
show<Nope>(null! as R15);
show<Nope>(null! as R15['kind' & keyof R15]);
show<Nope>(null! as R16);
show<Nope>(null! as R16['kind' & keyof R16]);
show<Nope>(null! as R17);
show<Nope>(null! as R17['kind' & keyof R17]);
show<Nope>(null! as R18);
show<Nope>(null! as R18['kind' & keyof R18]);
show<Nope>(null! as R19);
show<Nope>(null! as R19['kind' & keyof R19]);
show<Nope>(null! as R20);
show<Nope>(null! as R20['kind' & keyof R20]);
show<Nope>(null! as R21);
show<Nope>(null! as R21['kind' & keyof R21]);
show<Nope>(null! as R22);
show<Nope>(null! as R23);
show<Nope>(null! as R23['kind' & keyof R23]);
show<Nope>(null! as R24);
show<Nope>(null! as R24['kind' & keyof R24]);
show<Nope>(null! as R25);
show<Nope>(null! as R25['kind' & keyof R25]);
show<Nope>(null! as R26);
show<Nope>(null! as R26['kind' & keyof R26]);
show<Nope>(null! as R27);
show<Nope>(null! as R27['kind' & keyof R27]);
show<Nope>(null! as R28);
show<Nope>(null! as R28['kind' & keyof R28]);
show<Nope>(null! as R29);
show<Nope>(null! as R29['kind' & keyof R29]);
show<Nope>(null! as ByKind['a']);
show<Nope>(null! as ByKind['a']['kind' & keyof ByKind['a']]);
show<Nope>(null! as ByKind['x']);
show<Nope>(null! as ByKind['x']['kind' & keyof ByKind['x']]);
show<Nope>(null! as ByKind['opt']);
show<Nope>(null! as ByKind['opt']['kind' & keyof ByKind['opt']]);
