checker: a nested declaration check charges its instantiations to the expression that asked for it

**Status:** written up only, not opened upstream. Found by tsrs on backstage/backstage (notes/open-history-dependence.md
section 5). tsrs's default mode clears the count on exit (below); `--checkerAssignment go` keeps Go's behaviour.

`instantiateTypeWithAlias` (checker.go:22507) yields the error type and reports TS2589 once 5,000,000 instantiations
have been made "by the same statement or expression". `checkExpressionEx` (checker.go:7729) and `checkSourceElement`
(checker.go:2282) set `instantiationCount` to 0 on entry, and neither puts the caller's count back on exit:

```go
	saveCurrentNode := c.currentNode
	c.currentNode = node
	c.instantiationCount = 0
	uninstantiatedType := c.checkExpressionWorker(node, checkMode)
	...
	c.currentNode = saveCurrentNode
```

When an expression needs the type of a variable whose initializer has not been checked yet, the initializer is checked
inside it. Its instantiations count for the caller from then on, and if the initializer used up the budget, the next
instantiation in the caller reports TS2589 at the caller. Whether the initializer is checked there or was cached before
depends on what the checker has already checked. So a diagnostic in one file depends on the order the files are checked
in and, with several checkers, on how they are split.

Repro (global scripts, so `files` sets the check order):

```ts
// a.ts
type D = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";
type U = `${D}${D}${D}${D}`;
type Chain<T, N extends unknown[] = []> = N["length"] extends 100 ? { value: T } : T extends string ? Chain<T, [...N, 0]> : never;
declare function make<T>(f: (x: T) => void): Chain<T>;
const big = make((x: U) => {});
// b.ts
declare function id<T>(x: T): T;
const y = id(big);
```

`tsconfig.json`: `{"compilerOptions":{"strict":true,"noEmit":true,"target":"es2022","types":[]},"files":["a.ts","b.ts"]}`

```
$ tsgo -p . --checkers 1
a.ts(5,13): error TS2589: Type instantiation is excessively deep and possibly infinite.
$ # "files": ["b.ts", "a.ts"]
$ tsgo -p . --checkers 1
a.ts(5,13): error TS2589: Type instantiation is excessively deep and possibly infinite.
b.ts(2,11): error TS2589: Type instantiation is excessively deep and possibly infinite.
```

`make(...)` takes about 5,027,000 instantiations. With `a.ts` first, `big`'s type is cached when `b.ts` is checked and
`id(big)` costs a few instantiations. With `b.ts` first, checking `id(big)` checks `big`'s initializer, which reports at
`a.ts(5,13)` and leaves the count above the limit, so inferring `T` for `id` reports again at `b.ts(2,11)`. With
`a.ts` first, several checkers give the same dependence: tsgo prints one error at 1-4 checkers and both at 8 and 16.
tsgo is the pinned b85298b6 build; tsrs under `--checkerAssignment go` prints the same in both orders and at each of
those counts.

The depth limit (`len(c.instantiationStack) == 100`) has a similar dependence with a different cause: what a checker has
cached decides how deep a chain goes (notes/open-history-dependence.md section 6). No restore is missing there.

Possible fix (what tsrs does by default): clear `instantiationCount` on exit from `checkExpressionEx` and
`checkSourceElement`, as well as on entry. A nested check is charged to itself only, and `b.ts(2,11)` goes away in
both orders and at every checker count. This can remove a TS2589 even when the nested check used less than the
whole budget: its instantiations no longer add to the caller's work after it returns. The depth guard is unchanged.

Restoring the caller's saved count instead is not equivalent. Go discards the caller's earlier work on entry to
each nested check. Restoring it lets a call accumulate work across an argument check, and can add a TS2589 absent
from Go in every order. This one-file reduction is `testdata/regressions/instantiation-count-call-argument`:

```ts
type D = "0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9";
type E = "0"|"1"|"2"|"3"|"4"|"5";
type U = `${D}${D}${D}${E}`;
type Chain<T, N extends unknown[] = []> = N["length"] extends 100 ? { value: T } : T extends string ? Chain<T, [...N, 0]> : never;
type Chain2<T, N extends unknown[] = []> = N["length"] extends 100 ? { other: T } : T extends string ? Chain2<T, [...N, 0]> : never;
declare function k<T>(x: Chain<T>): Chain2<T>;
declare const v: any;
const r = k<U>(v);
```

With the same compiler options as above, pinned tsgo exits 0, as does Go-compatible tsrs. The save/restore version
reports `a.ts(8,11): error TS2589: Type instantiation is excessively deep and possibly infinite.` in default mode,
including single-threaded: it carries parameter-type work across checking `v` and adds the return-type work.
Clearing on exit discards both the child's work and the caller's earlier work, so the reduction exits 0 again.
