# Checker body-porting wave: instructions for every agent

Project `tsrs`: a faithful Rust port of the TypeScript 7 (Go) type checker (type-check only).
Your shell's default cwd is an unrelated monorepo: ignore its instructions and never edit it.

## Where you work

- Your private git worktree: `$TSRS_WORK/wt/<agent-name>` on branch `body/<agent-name>`
  (already created from tag `body-base`). Work ONLY there. Never touch the main checkout
  `$TSRS_WORK/tsrs` or other agents' worktrees.
- Build: `cd $TSRS_WORK/wt/<agent-name> && CARGO_TARGET_DIR=$PWD/target cargo check -p tsrs_checker --message-format short`
  (the first build compiles dependencies, ~1–2 min). In your worktree every other checker file still has `todo!()`
  stubs, so the crate compiles before you start and must compile when you finish: **0 errors, 0 warnings**.
- Commit to your branch early and often (`git add -A . && git commit -m "checker: <file> …"` is fine inside your own
  worktree). Do not merge, rebase or push.
- Go source of truth: `ts-ref/tsc/internal/checker/` (symlink inside your worktree).

## Read first

`docs/PORTING.md` (conventions, memory model, core-crate notes), `docs/AST.md` (AST API as implemented),
`docs/CHECKER.md` (checker data model as implemented — Type/TypeData, links, Relater handle style, function-valued
fields as methods, `GoMap`, `LiteralValue`, error reporting API). They are binding.

## The job

Your assignment names one or more generated stub files in `crates/tsrs_checker/src/` and the Go line range they came
from. Every function in them has a generated signature, a `// file.go:LINE` origin marker and a `todo!()` body.
Replace every `todo!()` with a faithful port of the Go function body.

- **Faithful** means: same algorithm, same order of operations and evaluation (side effects such as caching,
  diagnostics and type creation happen in the same order), same diagnostics with the same arguments, same helper
  decomposition. Read the Go function, write the Rust function. Do not port from memory of `checker.ts`; do not
  simplify, "fix" or optimize. Keep the Go comments that explain *why*; keep the origin markers.
- **Signatures are fixed.** Look up every callee in `docs/sigs/checker.txt` (one line per function: rust name |
  signature | go file:line | receiver) or by grepping the stub files; hand-written ones (function-valued Checker
  fields, `get_global_*`, type/links/mapper methods) are in `checker.rs`, `types.rs`, `links.rs`, `mapper.rs`.
  Use callees exactly as declared: if a parameter is `Option<P<Type>>` pass `Some(x)`; if a result is `Option`,
  handle or `unwrap()` it exactly where Go would dereference.
  If a generated signature is genuinely wrong for the Go semantics (e.g. Go passes/returns nil but the type is not
  `Option`, or a slice must be distinguishable nil-vs-empty), fix the signature in your file and record it in your
  notes file (below). Do not "fix" signatures in other agents' files — record the needed change instead and code
  against the signature you need (it will not compile against their stub: in that case adapt at the call site with a
  `// SIG:` comment explaining what should change, and keep the crate compiling).
- Local closures that Go nests inside a function become nested closures or small private helper fns next to it;
  closures never capture `self` (pass the checker as a parameter, per PORTING.md).
- Borrowing: checker state lives in plain fields behind `&mut self`. Arena handles (`P<…>`, `&'static […]`) are `Copy`:
  copy them out of `self` before calling other `self` methods; never hold a `RefCell` borrow or a reference into a
  `self.field` collection across a call that may re-enter the checker (clone the small Vec or index instead).
- AST access: use the getter methods (`node.as_binary_expression().left()`, `node.parent()`, `node.symbol()`), see AST.md.
  About 20 checker free functions share a name with a `tsrs_ast` function: write `crate::name(..)` or `ast::name(..)`.
  `P::get` shadows `T::get` (PORTING.md): symbol tables use `.lookup(name)`.
- Out-of-scope features: tracing (`c.tracer`), cancellation (`c.ctx`), language-service-only paths, emit resolver,
  declaration emit. Drop those branches (no `todo!()`), keeping everything else intact. Node builder / type-to-string
  functions (`type_to_string`, `symbol_to_string`, `signature_to_string`, …) are being ported by other agents: just
  call them by their declared signatures.
- If the data model lacks something you need (a struct field, a type, a constant, a small helper, an AST utility that
  was skipped), add it **minimally and additively** in the file where it belongs (`types.rs`, `checker.rs`,
  `*_types.rs`, or `crates/tsrs_ast/src/utilities_*.rs`) and record it in your notes. Do not restructure shared files:
  20+ branches will be merged and every shared-file edit is a potential conflict.
- No `todo!()`, `unimplemented!()` or placeholder bodies may remain in your files unless listed in your report with
  the reason.

## Notes file

Maintain `notes/<agent-name>.md` in your worktree (commit it). Sections: `## Signature changes` (function, old -> new,
why), `## Shared-file edits` (file, what, why), `## Needs from others` (things you called that look wrong/missing),
`## Doubts` (places where you were unsure the port is faithful — be honest, these get reviewed first).

## Done means

All functions in your file(s) ported; `cargo check -p tsrs_checker` in your worktree has 0 errors and 0 warnings;
everything committed on your branch; notes file up to date. There is nothing to run yet (the rest of the checker is
stubs), so correctness comes from careful reading: after finishing, re-read your port side by side with the Go source
once more and fix what you find. Work through the file in order; if you run low on context, commit what you have and
continue — do not stop early.

Final report (<= 15 lines): functions ported, anything left unported and why, number of signature changes and
shared-file edits (details are in the notes file), top doubts.
