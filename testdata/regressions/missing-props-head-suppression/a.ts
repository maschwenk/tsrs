declare class Reference<T extends object> {
  private entity;
  constructor(entity: T);
}
type EntityRef<T extends object> = true extends IsUnknown<PrimaryProperty<T>> ? Reference<T> : ({ [K in PrimaryProperty<T> & keyof T]: T[K] } & Reference<T>);
type IsUnknown<T> = T extends unknown ? (unknown extends T ? true : never) : never;
type PrimaryProperty<T> = T extends { id?: any } ? 'id' : unknown;
type Ref<T> = T extends any ? EntityRef<T & object> : never;
declare function getReference<E extends object>(name: abstract new () => E, id: string, wrapped: true): Ref<E>;
declare function getReference<E extends object>(name: abstract new () => E, id: string, wrapped?: boolean): E | Reference<E>;
declare class Loc { a: string; b: string; c: string; d: string; e: string; f: string; id: string }
declare function take(l: Loc): void;
take(getReference("x" as any as string, Loc));
