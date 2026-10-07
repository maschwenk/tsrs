// The same inference walks made again and again (the inference memo, crates/tsrs_checker/src/infermemo.rs):
// return type inference from a contextual union to Extract<Union, { type: K }>, at many call sites.
interface E0 { type: 'k0'; data: { v0: number; s: string; extra?: boolean }; id: string }
interface E1 { type: 'k1'; data: { v1: number; s: string }; id: string }
interface E2 { type: 'k2'; data: { v2: number; s: string }; id: string }
interface E3 { type: 'k3'; data: { v3: number; s: string; extra?: boolean }; id: string }
interface E4 { type: 'k4'; data: { v4: number; s: string }; id: string }
interface E5 { type: 'k5'; data: { v5: number; s: string }; id: string }
interface E6 { type: 'k6'; data: { v6: number; s: string; extra?: boolean }; id: string }
interface E7 { type: 'k7'; data: { v7: number; s: string }; id: string }
interface E8 { type: 'k8'; data: { v8: number; s: string }; id: string }
interface E9 { type: 'k9'; data: { v9: number; s: string; extra?: boolean }; id: string }
interface E10 { type: 'k10'; data: { v10: number; s: string }; id: string }
interface E11 { type: 'k11'; data: { v11: number; s: string }; id: string }
interface E12 { type: 'k12'; data: { v12: number; s: string; extra?: boolean }; id: string }
interface E13 { type: 'k13'; data: { v13: number; s: string }; id: string }
interface E14 { type: 'k14'; data: { v14: number; s: string }; id: string }
interface E15 { type: 'k15'; data: { v15: number; s: string; extra?: boolean }; id: string }
interface E16 { type: 'k16'; data: { v16: number; s: string }; id: string }
interface E17 { type: 'k17'; data: { v17: number; s: string }; id: string }
interface E18 { type: 'k18'; data: { v18: number; s: string; extra?: boolean }; id: string }
interface E19 { type: 'k19'; data: { v19: number; s: string }; id: string }
interface E20 { type: 'k20'; data: { v20: number; s: string }; id: string }
interface E21 { type: 'k21'; data: { v21: number; s: string; extra?: boolean }; id: string }
interface E22 { type: 'k22'; data: { v22: number; s: string }; id: string }
interface E23 { type: 'k23'; data: { v23: number; s: string }; id: string }
type Ev = E0 | E1 | E2 | E3 | E4 | E5 | E6 | E7 | E8 | E9 | E10 | E11 | E12 | E13 | E14 | E15 | E16 | E17 | E18 | E19 | E20 | E21 | E22 | E23;
type EvType = Ev['type'];
type Payload<K extends EvType> = Extract<Ev, { type: K }>;
declare function ev<K extends EvType>(type: K, data: Payload<K>['data'], overrides?: Partial<Omit<Payload<K>, 'type' | 'data'>>): Payload<K>;
declare function consume(events: readonly Ev[]): number;
declare function pick<T, K extends keyof T>(obj: T, key: K): T[K];
export const ok = consume([
  ev('k0', { v0: 0, s: 'x' }),
  ev('k3', { v3: 0, s: 'x' }),
  ev('k6', { v6: 0, s: 'x' }),
  ev('k9', { v9: 0, s: 'x' }),
  ev('k12', { v12: 0, s: 'x' }),
  ev('k15', { v15: 0, s: 'x' }),
  ev('k18', { v18: 0, s: 'x' }),
  ev('k21', { v21: 0, s: 'x' }),
  ev('k0', { v0: 1, s: 'x' }),
  ev('k3', { v3: 1, s: 'x' }),
  ev('k6', { v6: 1, s: 'x' }),
  ev('k9', { v9: 1, s: 'x' }),
  ev('k12', { v12: 1, s: 'x' }),
  ev('k15', { v15: 1, s: 'x' }),
  ev('k18', { v18: 1, s: 'x' }),
  ev('k21', { v21: 1, s: 'x' }),
  ev('k0', { v0: 2, s: 'x' }),
  ev('k3', { v3: 2, s: 'x' }),
  ev('k6', { v6: 2, s: 'x' }),
  ev('k9', { v9: 2, s: 'x' }),
  ev('k12', { v12: 2, s: 'x' }),
  ev('k15', { v15: 2, s: 'x' }),
  ev('k18', { v18: 2, s: 'x' }),
  ev('k21', { v21: 2, s: 'x' }),
  ev('k0', { v0: 3, s: 'x' }),
  ev('k3', { v3: 3, s: 'x' }),
  ev('k6', { v6: 3, s: 'x' }),
  ev('k9', { v9: 3, s: 'x' }),
  ev('k12', { v12: 3, s: 'x' }),
  ev('k15', { v15: 3, s: 'x' }),
  ev('k18', { v18: 3, s: 'x' }),
  ev('k21', { v21: 3, s: 'x' }),
  ev('k0', { v0: 4, s: 'x' }),
  ev('k3', { v3: 4, s: 'x' }),
  ev('k6', { v6: 4, s: 'x' }),
  ev('k9', { v9: 4, s: 'x' }),
  ev('k12', { v12: 4, s: 'x' }),
  ev('k15', { v15: 4, s: 'x' }),
  ev('k18', { v18: 4, s: 'x' }),
  ev('k21', { v21: 4, s: 'x' }),
  ev('k0', { v0: 5, s: 'x' }),
  ev('k3', { v3: 5, s: 'x' }),
  ev('k6', { v6: 5, s: 'x' }),
  ev('k9', { v9: 5, s: 'x' }),
  ev('k12', { v12: 5, s: 'x' }),
  ev('k15', { v15: 5, s: 'x' }),
  ev('k18', { v18: 5, s: 'x' }),
  ev('k21', { v21: 5, s: 'x' }),
]);
export const bad = consume([
  ev('k0', { v0: 1, s: 'x' }),
  ev('k1', { v0: 1, s: 'x' }),
  ev('k2', { v2: 'no', s: 'x' }),
  ev('k3', { v3: 3, s: 'x' }, { id: 4 }),
  ev('k4', { v4: 4, s: 'x' }, { idd: 'x' }),
  ev('zz', { v0: 1, s: 'x' }),
]);
export const single: E5 = ev('k6', { v6: 6, s: 'x' });
export const both: E7 | E8 = ev('k7', { v8: 1, s: 'x' });
export const p0 = pick(ev('k9', { v9: 0, s: 'y' }), 'data');
export const p1 = pick(ev('k9', { v9: 1, s: 'y' }), 'data');
export const p2 = pick(ev('k9', { v9: 2, s: 'y' }), 'data');
export const p3 = pick(ev('k9', { v9: 3, s: 'y' }), 'data');
export const wrong = pick(ev('k9', { v9: 1, s: 'y' }), 'nope');
