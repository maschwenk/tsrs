checker: a nested declaration check charges its instantiations to the expression that asked for it

**Status:** written up only, not opened upstream. Found by tsrs on backstage/backstage (notes/open-history-dependence.md
section 5). tsrs's default mode restores the caller's count (below); `--checkerAssignment go` keeps Go's behaviour.

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

Possible fix (what tsrs does by default): save `instantiationCount` in `checkExpressionEx` and `checkSourceElement` and
restore it on exit, so a nested check is charged to itself only. `b.ts(2,11)` then goes away in both orders and at every
checker count. Applied unconditionally in tsrs, all 13,458 passing error baselines still pass, so no baseline depends on
the missing restore; it changes output only where a nested check used up the budget, which can happen single-threaded
(the `b.ts` first order above).
