# checker-01 notes (checker_01.rs = checker.go 1–2172)

## Signature changes

- `get_suggested_symbol_for_nonexistent_symbol(location: P<Node>, ..)` -> `location: Option<P<Node>>`.
  Go's `onFailedToResolveSymbol` passes `errorLocation`, which is nil when `getGlobalSymbol` resolves with a
  diagnostic (`resolveName(nil, ...)`). No other callers.

## Shared-file edits

None.

## Needs from others

None found. Callees used as declared.

## Doubts

- `initialize_closures` is empty: the Go closures are the checker.rs methods. `initialize_iteration_resolvers`
  builds the two resolvers exactly like `new_checker` does inline; `new_checker` (foundation) never calls either
  function, so both are effectively dead but kept for parity.
- `get_global_{type,type_alias,value_symbol,type_symbol,types}_resolver` return memoized `Box<dyn FnMut(&mut Checker)>`
  closures (Go `core.Memoize`); nothing calls them, because the foundation implemented the globals as cached methods.
- `is_block_scoped_name_declared_before_use`: `decl_container` (Go `GetEnclosingBlockScopeContainer`, nil-able) is
  unwrapped at the three places it is passed to `P<Node>` parameters. Go would tolerate nil there (it only compares
  against it); this only matters for nodes without an enclosing block-scope container, which should not happen.
- `is_used_in_function_or_instance_property`: Go's `ast.FindAncestorOrQuit` + closure is inlined as a loop plus the
  private helper `is_used_in_function_or_instance_property_callback`, because the callback needs `&mut Checker`.
- `get_spelling_suggestion_for_name`: both callbacks (`getCandidateName` with `tryResolveAlias`, and `compareSymbols`)
  need the checker, so the checker is wrapped in a local `RefCell` for the duration of the call; the worker never
  invokes them re-entrantly.
- Candidate order for spelling suggestions: Go iterates maps (random); here the symbol table is insertion-ordered
  and `primitive_type_alias_suggestions()` is an FxHashMap. Ties are broken by `compareSymbols` like Go.
- `merge_pattern_ambient_modules` keeps `(Pattern, P<Symbol>)` pairs locally and allocates fresh
  `P<PatternAmbientModule>` at the end (Go mutates the fresh `grouped` structs in place; they are not visible before
  the final assignment, so no observable difference).
- `!SymbolFlags::Value` / `!SymbolFlags::Type & SymbolFlags::Value`: bitflags complement drops undefined bits, Go's
  `^` keeps them. Only matters if some code tests an undefined bit.
