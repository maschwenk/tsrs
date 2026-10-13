checker: how deep an elaboration goes depends on which path first created a type reference

**Status:** written up only, not opened upstream. Found by tsrs on rxjs (notes/open-history-dependence.md section 3).
tsrs does not change it: `--checkerAssignment go` prints what tsgo prints at every checker count, and the default mode
has no fix that keeps tsgo's single-checker output.

`createTypeReferenceEx` (checker.go:25577) interns references by their type arguments, and only the first request sets
the flags. A reference written in a type annotation is created with `ObjectFlagsFromTypeNode` (checker.go:23669), so the
flag says which path created the interned type first:

```go
func (c *Checker) createTypeReferenceEx(target *Type, typeArguments []*Type, objectFlags ObjectFlags) *Type {
	id := getTypeListKey(typeArguments)
	intf := target.AsInterfaceType()
	if t, ok := intf.instantiations[id]; ok {
		return t
	}
	t := c.newObjectType(ObjectFlagsReference|objectFlags|c.getPropagatingFlagsOfTypes(typeArguments, TypeFlagsNone), target.symbol)
```

`getRecursionIdentityFromTarget` (relater.go:849) gives a reference with that flag its own identity instead of its
symbol's. `isDeeplyNestedType` (relater.go:766) counts stack entries with the same identity, so whether
`Obs<number>` counts towards the three-level limit depends on whether something in the same checker wrote
`Obs<number>` in an annotation before the comparison instantiated it. The elaboration then stops one level earlier or
later, and the printed message changes. With several checkers it also depends on how the files are split.

Repro (`a.ts` only declares a variable; nothing imports it):

```ts
// lib.ts
export interface Obs<T> {
  buf(): Obs<T[]>;
  grp(o: Opts<T>): void;
}
export interface Opts<T> { e?: (v: T) => T }
export type ObsV<T> = Obs<T>;
export declare function of<T>(v: T): Obs<T>;
export declare function take(x: ObsV<string | number>): void;
// a.ts
import type { Obs } from "./lib";
export declare const early: Obs<number>;
// b.ts
import { of, take } from "./lib";
take(of(1));
```

```
$ tsgo -p . --singleThreaded    # files: lib.ts, a.ts, b.ts; also --checkers 2
b.ts(2,6): error TS2345: Argument of type 'Obs<number>' is not assignable to parameter of type 'ObsV<string | number>'.
  The types returned by 'buf().buf().grp' are incompatible between these types.
$ tsgo -p . --checkers 3        # also 4, and lib.ts b.ts without a.ts
b.ts(2,6): error TS2345: Argument of type 'Obs<number>' is not assignable to parameter of type 'ObsV<string | number>'.
  The types returned by 'buf().grp' are incompatible between these types.
```

The source stack at the third `Obs` holds `Obs<number>`, `Obs<number[]>` and `Obs<number[][]>` (with the method types
between them). When `b.ts`'s checker creates `Obs<number>` from `of(1)`, all three share `Obs`'s identity and the third
is deeply nested, so `Obs<number[]>`'s `grp` is the reported failure. When `a.ts` was checked first on that checker,
`Obs<number>` carries `ObjectFlagsFromTypeNode`, only two count, and the relater goes one level deeper.

On rxjs `54796b38a57e` (`packages/rxjs/tsconfig.json`) tsgo prints the same 22,030 error headers at 1, 2, 4 and 8
checkers, but the elaboration text of 5 of them differs between 1 and 4 checkers and of 24 between 1 and 8. For example
`zip-all.spec.ts(99,50)` reports "The types returned by '[bufferTime](...)[groupBy]'" at 4 checkers and
"'[bufferTime](...)[bufferTime](...)[groupBy]'" at 1 and 2; there the single-checker run has `Observable<number>` with
`ObjectFlagsFromTypeNode` on the source stack and the 4-checker run has it without.

No fix is proposed: any rule that does not depend on creation order changes which elaborations single-checker runs
print today.
