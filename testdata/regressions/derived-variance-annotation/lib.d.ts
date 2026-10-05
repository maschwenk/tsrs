// A wrong variance annotation in a declaration file: `out T` on a contravariant use. Under skipLibCheck TypeScript
// does not report TS2636, and relates two references to `B` by the annotation.
export interface B<out T> {
  f: (x: T) => void
}
export interface D<T> extends B<T> { d: 1 }
