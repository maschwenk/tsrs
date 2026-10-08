export class Box<T> {
  constructor(public value: T) {}
  map<U>(f: (v: T) => U): Box<U> { return new Box(f(this.value)); }
}
// café 😀 non-ASCII text in a comment
export const s = "é😀";
