# gosig: Rust signature stubs from a Go package

`gosig` type-checks a Go package and writes Rust stubs (`todo!()` bodies) for its functions,
so that many agents can port bodies in parallel against signatures that already agree.
Its main job is deciding `P<T>` vs `Option<P<T>>` for every pointer parameter and result
with a whole-package nil-ability analysis.

## Running

```sh
cd tools/gosig
GOTOOLCHAIN=auto go run . -config checker.json           # writes crates/tsrs_checker/src/*.rs + docs/sigs/*.txt
GOTOOLCHAIN=auto go run . -config checker.json -dry      # analysis + statistics only
GOTOOLCHAIN=auto go run . -config checker.json -out ../../target/scratch/gosig/out   # write elsewhere
python3 check_stubs.py ../../target/scratch/gosig/out     # compile the stubs against dummy types
```

The module needs Go 1.27, so `GOTOOLCHAIN=auto` fetches it. The tool has its own `go.mod`.
It runs `go list -export -deps` in `moduleDir`, type-checks every package of that module
from source (std and third-party packages come from export data), and never imports Go
`internal` packages itself. A run takes about 1 s.

Flags for investigating decisions:

| flag | meaning |
| --- | --- |
| `-why FILE` | write the reason for every `Option` decision (`get_x.p1: nil passed at checker.go:123`) |
| `-trace goName,…` | follow the chain of returns that makes a result `Option`, down to its root |
| `-roots` | histogram of root causes of `Option` results (with example functions) |
| `-flowroots` / `-flowtrace goName` | where the nils that make parameters `Option` enter, and the hops they take |
| `-hot` | the most-called functions with `Option` positions (the places where a mistake costs most) |
| `-forwarding MODE`, `-transparent true/false` | override config knobs for comparison runs |

## Config (`checker.json`)

Paths are relative to the config file.

| key | meaning |
| --- | --- |
| `moduleDir`, `package` | Go module root and the import path to generate |
| `resultOnlyPackages` | extra in-module packages whose bodies are analyzed only so their *results* are known (`ast` accessors, `core`, `binder`) |
| `outDir`, `sigsFile`, `advisorySigsFile` | stub directory; grep file for generated signatures; grep file for functions that are ported by hand or later (e.g. `types.go`, `nodebuilder*.go`) |
| `prelude` | text at the top of every generated file |
| `files` | emitted Go files in output order: `{go, rust}` or `{go, chunks: [{rust, from, to}]}` (a declaration goes to the chunk holding its first line); `declsNote` labels the comment block that lists the file's non-function declarations. Several Go files may share a Rust file |
| `notPorted` | Go files (globs) that are ignored completely: no call sites, no reserved names |
| `skipFuncs` | functions never emitted (`NewChecker`, the pool helpers `Checker.getRelater`…); listed in the chunk's comment block |
| `checkerType`, `fieldsFile` | the state-machine type; its function-typed fields become methods in `fieldsFile` |
| `checkerHolders` / `notCheckerHolders` | helper structs holding `c *Checker` that become `impl<'c> Name<'c>` / that do not (the tool warns about unlisted ones) |
| `arenaTypes` | receivers whose methods take `&self` |
| `typeMap` | Go type string -> Rust type. Keys use package names (`*ast.Node`, `ast.SymbolTable`, `*diagnostics.Message`) and are unqualified for the target package (`*Checker`, `TypeData`). A generic named type maps by template: `"iter.Seq": "Vec<$0>"`, with an optional `"iter.Seq@param": "&[$0]"` for parameters |
| `dataInterfaces` | `*T` where `T` implements the interface maps by template: `ast.nodeData` -> `P<Node>` (except `SourceFile`), `TypeData` -> `&'static {T}` |
| `paramTypes` | per-position Rust types: `"Checker.newLiteralType.value": "LiteralValue"` |
| `constraintMap` | Go type-parameter constraints -> Rust bounds |
| `wordReplacements` | applied before snake_casing (`JSDoc` -> `Jsdoc`, `NaN` -> `Nan`, `IDs` -> `Ids`) |
| `zeroValueGenerics`, `coalesceGenerics`, `funcPassthrough` | generic helpers whose nil behavior the analysis must know: return the zero value on a miss (`core.Find`), return the first non-nil argument (`core.OrElse`), return their func argument (`core.Memoize`) |
| `callbackConditionalNil` | functions that return nil only when their callback does (`mapType`): each call site is judged by the callback it passes |
| `nilTransparent` | treat nil-in/nil-out parameters as non-nil (below) |
| `paramForwarding` | `forward` (default), `reverse`, `both`, `off` (below) |
| `overrides` | manual corrections applied before the fixpoint: `"Checker.goName.p0": "P"` or `"Option"`, `".r0"` for results, `"Checker.goName#local": "nonnil"` for a local whose zero value never escapes |

## Mapping

- Names: snake_case with acronym runs as one word (`getJSXElementType` -> `get_jsx_element_type`,
  `isESSymbol` -> `is_es_symbol`, `nodeID` -> `node_id`); keywords get a trailing `_`; Go type names
  are kept. Exported -> `pub`, else `pub(crate)`.
- Name collisions within one `impl` (or among free functions) are resolved deterministically:
  unexported first, then by config file order and line. An exported function that only forwards
  to the colliding function with an identical signature is **merged** (not emitted; the winner
  becomes `pub`); any other loser gets `_exported` (or `_2`, …). Both lists are printed.
- Receivers: `Checker` and checker holders `&mut self`; arena types `&self`; value receivers
  `self`; other types `&mut self` if the method (transitively) writes through the receiver.
- Types: `int` -> `i32`; `string` -> `&str` / `String` (result); `[]T` -> `&[T]` / `Vec<T>`
  (`&mut [T]` when written through); maps -> `&FxHashMap` / `FxHashMap` (`&mut` when written);
  `*T` struct -> `P<T>`, `*Checker` -> `&mut Checker`, pointer to non-struct -> `&mut T`;
  `...any` -> `&[&dyn Display]`; multiple results -> tuples; funcs -> `impl FnMut(&mut Checker, …) -> R`
  for methods of `Checker`/holders and functions taking a `*Checker` (unless the callback already
  takes `c *Checker` first), `impl FnMut(…)` elsewhere, `&mut dyn FnMut` nested,
  `Box<dyn FnMut>` in results; nil-able callbacks become `Option<&mut dyn FnMut(…)>`.
  Anything else (interfaces, `any`, union constraints) is a best guess followed by `/*?*/`.

## Nil-ability analysis

Nil-able positions are pointers, interfaces, funcs, table maps (`ast.SymbolTable`) and map
parameters. The whole package is analyzed (bodies in `notPorted` files are ignored) together
with the result-only packages.

**Parameters** are `Option` iff:

1. (a) some call site passes literal `nil` (also through `core.IfElse(c, x, nil)`), or
2. (b) the body compares the parameter with `nil` before any textual reassignment of it
   (comparisons after `p = p.Parent` say nothing about the incoming value), or
3. (c, `forward` mode) a *literal nil* reaches the argument through variables or other
   parameters without a visible guard. This is literal-nil provenance only: call results, fields
   and map misses do not count, and neither do parameters that are `Option` only because of (b).
   Guards that are recognized: `if x != nil {…}`, `x != nil && …`, `if x == nil { return }`,
   `if x == nil { x = v }`, if/else and switches that assign on every path, `debug.Assert`,
   earlier cases of a tagless `switch`, boolean guard variables (`ok := x != nil && …; if ok`),
   guards around closures, and definite assignment of zero-initialized locals.
4. Callback positions get the same treatment through the literals or function values passed at
   call sites (for example a literal that returns `nil` makes the callback result `Option`).

`reverse` mode is the literal spec rule: "a parameter forwarded to an `Option` parameter is
`Option`". It spreads from defensive callees and ends at `get_type_of_symbol(symbol: Option<…>)`,
with 1161 `Option` parameters against 361 in `forward` mode (288 with `off`).

**Nil-transparent parameters** (`nilTransparent`): when a function with a single pointer result
starts with `if p == nil [|| …] { return nil }` (or `return p`), or only guards `p` with
`if p != nil && … {…}` and ends in `return p`, and those are its only nil comparisons of `p`,
then `p` is `P<T>` and the nil exit is ignored (`instantiate_type(t: P<Type>, …) -> P<Type>`).
Callers that hold an `Option` map over the call. A literal `nil` argument still makes `p` `Option`.

**Results** use a three-point lattice per result (non-nil / unknown / nil):

- (a) `return nil`, a zero-valued named result or `var x *T` returned where it may still be
  zero, a map miss, `core.Find`-style zero values -> nil;
- (b) returning a call whose result is nil, a nil-able parameter, a local whose reaching values
  include nil (using the guards above), or a field that is compared with / assigned `nil`
  anywhere -> nil. Interface method calls resolve to all analyzed implementations. Slice
  elements and dynamic calls of func values count as non-nil. Calls into unanalyzed packages
  count as unknown;
- (c) a call site compares the result with `nil` (directly, or through a variable assigned once
  from the call with no live zero value). This only decides results whose body analysis is
  *unknown*: a single defensive check does not turn `get_type_of_symbol` into `Option` when all
  of its return paths are known to be non-nil. Bodiless positions (the `Checker` function
  fields) are decided by (c) alone.

## Known weak spots

- Guards expressed through other variables or invariants are invisible (`typeVariableCount == 1`
  implies `nakedTypeVariable != nil`; `assumeUninitialized` implies `prop != nil`). Fix these
  with `overrides`. The current overrides are in `checker.json`.
- Flow sensitivity is approximate. The nearest dominating guard or assignment in a statement
  list is honored, and otherwise all values a variable is ever assigned count. Loops and `goto`
  are handled conservatively.
- Fields count as nil-able once any code compares them with `nil`, so `return node.Parent` is
  `Option`, and so is a type field that is lazily initialized in one place.
- A result is `Option` whenever Go can return nil on *some* path, including a nil passed through
  from a nil-able argument. Callers that know better must `unwrap()`. `mapType` is special-cased
  (see `callbackConditionalNil`); similar helpers can be added there.
- Results of calls into packages that are not analyzed (`binder` is, `program` interfaces are
  not) are unknown and decided by call-site checks.
