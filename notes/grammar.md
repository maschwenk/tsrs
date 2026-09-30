# grammar (grammarchecks.rs)

## Signature changes

- `check_grammar_type_arguments`: `type_arguments: P<NodeList>` -> `Option<P<NodeList>>` (requested by checker-02/-04/-05:
  Go passes a nil list, e.g. `node.TypeArgumentList()`, and the callee handles nil via
  `checkGrammarForDisallowedTrailingComma`/`checkGrammarForAtLeastOneTypeArgument`).

## Shared-file edits

None.

## Needs from others

None found; every callee resolved against its declared signature.

## Doubts

- `check_grammar_regular_expression_literal`: Go's scanner error callback runs synchronously; the Rust scanner buffers
  errors, so they are drained (and run through the same callback body, `on_reg_exp_scanner_errors`) right after `scan()`
  and after `re_scan_slash_token(true)`. Order is preserved because the callback does not interact with the scanner.
- `FunctionLikeBase.parameters` (nil-widened in the AST for the reparser) and `MappedTypeNode.members` are `unwrap()`ed
  where Go dereferences `.Nodes` directly (`checkGrammarFunctionLikeDeclaration`, `checkGrammarIndexSignatureParameters`,
  `checkGrammarMappedType`); the parser always sets them, so Go would panic in the same situations.
- `checkGrammarIndexSignatureParameters`: `everyType(t, c.isValidIndexKeyType)` is written as
  `every_type(t, |t| self.is_valid_index_key_type(t))` (the closure captures `self`, since `every_type` is a free
  function whose callback does not receive the checker).
- `for`-`await` / top-level `await` module-kind switches with Go `fallthrough` are expressed with two flags
  (`fallthrough_es`, `fallthrough_default`); re-checked that every path emits the same diagnostics in the same order.
