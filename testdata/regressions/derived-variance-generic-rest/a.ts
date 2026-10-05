// TSRS_DERIVED_VARIANCE fuzz finding: a rest parameter typed by the type parameter itself is compared element by
// element (`getTypeAtPosition`). `B<never[]>` -> `B<any>` holds by variances (`any` -> `never[]`), but member by
// member `(...a: never[]) => void` -> `(...a: any) => void` compares `any` with `never` and fails.
interface B<T extends unknown[]> {
  f: (...a: T) => void
}
interface D<T extends unknown[]> extends B<T> { d: 1 }
declare const b: B<never[]>
export const x0: B<any> = b
declare const d: D<never[]>
export const x1: B<any> = d
