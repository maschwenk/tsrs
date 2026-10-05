checker: printed object-literal unions depend on which other literals the checker widened first

**Status:** written up only, not opened upstream. Found by tsrs (notes/perf-order-independence.md); tsrs ships the
fix by default and keeps Go's behaviour under `--checkerAssignment go`.

`getUndefinedProperty` (checker.go:18837) caches the optional `undefined` property it adds to widened object literals
by property **name**, for the whole checker:

```go
func (c *Checker) getUndefinedProperty(prop *ast.Symbol) *ast.Symbol {
	if cached := c.undefinedProperties[prop.Name]; cached != nil {
		return cached
	}
	result := c.createSymbolWithType(prop, c.undefinedOrMissingType)
	...
	c.undefinedProperties[prop.Name] = result
```

`createSymbolWithType` copies `prop`'s declarations, so the cached symbol keeps the declarations of the first property
of that name that this checker ever widened. `getNamedMembers` then sorts the widened literal's properties by first
declaration (`compareSymbols`: file index, then position). So where `x?: undefined` prints depends on which object
literal with an `x` the checker saw first, which can be in an unrelated file. As a result, a diagnostic or a declaration
file can change when an unrelated file is added to the program, or when files are split differently over checkers
(`--checkers N`).

Repro (no import between the files):

```ts
// a.ts
export const a = Math.random() > 0.5 ? { user: 1 } : { status: "a" };
// b.ts
export const b = Math.random() > 0.5 ? { status: "no" } : { status: "ok", user: 2 };
b.missing;
```

```
$ tsgo -p .               # files: a.ts, b.ts
b.ts(2,3): error TS2339: Property 'missing' does not exist on type '{ user?: undefined; status: string; } | { status: string; user: number; }'.
$ tsgo b.ts --strict --noEmit
b.ts(2,3): error TS2339: Property 'missing' does not exist on type '{ status: string; user?: undefined; } | { status: string; user: number; }'.
```

With several checkers the same thing happens across checkers. On a 38k-file program with ~40k errors, 3 messages in 3
files print differently between `--checkers 1`, 2, 4, 8 and 12. Within one file it also leaks across statements:
`conformance/objectLiteralNormalization` prints `c3: { a: number; b: number; } | { b?: undefined; a?: undefined; }`
because `b` was first cached on line 1 and `a` on line 10. Its `.symbols` baseline lists the stale line-1 declaration
for `a2.b` and `d1.pos.b`.

Fix: key the cache by the property it stands for, so the added property always sorts by the declaration it represents:

```diff
-	if cached := c.undefinedProperties[prop.Name]; cached != nil {
+	if cached := c.undefinedProperties[prop]; cached != nil {
 		return cached
 	}
 	...
-	c.undefinedProperties[prop.Name] = result
+	c.undefinedProperties[prop] = result
```

(`undefinedProperties` becomes `map[*ast.Symbol]*ast.Symbol`.) With it the repro prints `{ status: string; user?:
undefined; }` in both runs, and the 38k-file program prints the same text for any checker count and assignment. In the
test suite only `objectLiteralNormalization` changes: its `.types`, `.symbols` and `.js` (declaration) baselines move to
source order, for example `{ a?: undefined; b?: undefined; }`.
