# grammar (grammarchecks.rs)

## Signature changes

- `check_grammar_type_arguments`: `type_arguments: P<NodeList>` -> `Option<P<NodeList>>` (requested by checker-02/-04/-05:
  Go passes a nil list, e.g. `node.TypeArgumentList()`, and the callee handles nil via
  `checkGrammarForDisallowedTrailingComma`/`checkGrammarForAtLeastOneTypeArgument`).

## Shared-file edits

## Needs from others

## Doubts

- `check_grammar_regular_expression_literal`: Go's scanner error callback runs synchronously; the Rust scanner buffers
  errors, so they are drained (and run through the same callback body, `on_reg_exp_scanner_errors`) right after `scan()`
  and after `re_scan_slash_token(true)`. Order is preserved because the callback does not interact with the scanner.
