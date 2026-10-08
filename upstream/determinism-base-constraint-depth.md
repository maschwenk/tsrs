checker: a base constraint cut short by the depth guard is cached for every later caller

**Status:** written up only, not opened upstream. Found by tsrs on TanStack/router (notes/open-history-dependence.md).
tsrs does not fix it: the fixes measured there either cost up to 4% of instructions or leave part of the dependence.

`getResolvedBaseConstraint` (checker.go:27917) explores ten levels of nested constraints, then stops at a type whose
recursion identity is already on the stack, and caches whatever it computed for the type, the cut result included:

```go
	identity := getRecursionIdentity(t)
	if len(stack) < 10 || len(stack) < 50 && !slices.Contains(stack, identity) {
		constraint = c.computeBaseConstraint(c.getSimplifiedType(t, false), append(stack, identity))
	}
	...
	if constraint == nil {
		constraint = c.noConstraintType
	}
	if constrained.resolvedBaseConstraint == nil {
		constrained.resolvedBaseConstraint = constraint
	}
```

The stack belongs to whichever computation asked first. A type first reached ten levels down in another type's
constraint is cut where a direct request would not be, and `noConstraintType` is then what every later request gets, the
direct ones included. `getRecursionIdentityTarget` (relater.go:820) gives an indexed access the identity of its object
type, and a type parameter's identity is its symbol, so a chain `T["a"]["a"]...` carries `T`'s identity: at ten levels
it reaches `T` itself and caches `noConstraintType` for the type parameter.

So a diagnostic in one file depends on which files the same checker checked before it, and with several checkers on how
the files are split. Repro (no import between the files; `a.ts` only makes the checker resolve the chain from the top):

```ts
// a.ts
const f: typeof victim = <U extends Box>(x: U, y: { b: string }) => x.b;
// b.ts
interface Box { a: Box; b: string }
function victim<T extends Box>(x: T, y: T["a"]["a"]["a"]["a"]["a"]["a"]["a"]["a"]["a"]["a"]): string {
  return x.b;
}
```

`tsconfig.json`:
`{"compilerOptions":{"strict":true,"noEmit":true,"target":"es2020","types":[]},"files":["a.ts","b.ts"]}`

```
$ tsgo -p . --checkers 1      # also 2 and 4
b.ts(3,12): error TS2339: Property 'b' does not exist on type 'T'.
$ tsgo -p . --checkers 8      # also 3 and 16: no error
```

Relating `a.ts`'s arrow function to `typeof victim` infers from `victim`'s parameter types, and that resolves the base
constraint of `T["a"]` x10 from the top (an empty stack). Ten levels down that reaches `T`, whose symbol is the identity
of every level above it, and caches `noConstraintType` for `T`. Checking `b.ts` afterwards, `x.b` gets the apparent type
of `T` from that cache: `unknown`. Without `a.ts`, or with `b.ts` checked first, there is no error. tsgo is
`typescript@7.1.0-dev.20260930.4`; TypeScript 5.9.3 prints nothing for either order, because its `getRecursionIdentity`
gives an indexed access the identity of its object type (`T`), which differs from the identity of the type parameter
(`T`'s symbol), so the chain never cuts `T` itself. Its cache keeps cut results the same way, so any chain that repeats
an identity ten levels down behaves like this one.

Possible fixes, both changing results somewhere:

- Do not cache a result computed while the guard cut something below a non-empty stack, and reuse a cached result only
  where the guard could not cut it either. That makes the cache independent of order, but its answers are the "cold"
  ones: a chain like `T["a"]` x10 requested from the top is then always cut, so a lone function whose parameter `y` has
  type `U["a"]` x10 (with `U extends Box`) reports TS2339 on `y.b`, which tsgo does not (checking the parameter's type
  node resolves the chain inner level first, so no request is ever ten deep). It also recomputes a lot: 1.4x-6x the base
  constraint computations on type-heavy projects in tsrs.
- Resolve each type's base constraint from the top (an empty stack) when it is first requested, and stop types that keep
  instantiating new ones with a separate limit. That gives the answer checking in program order usually gives, at about
  the current cost, but replaces the identity guard, so the result for such expanding constraints changes.
