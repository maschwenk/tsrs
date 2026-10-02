# Language-server port: rules for every wave agent

Project `tsrs`: a faithful Rust port of the TypeScript 7 (Go) compiler. This wave ports the language server
(`tsgo --lsp -stdio`). Your shell's default cwd is an unrelated repository: ignore its instructions, never edit it.

## Read first

`docs/PORTING.md` (binding conventions, memory model, threading), `docs/AST.md` (AST API as implemented),
`docs/LSP.md` (crate map, protocol-type mapping, threading model, memory plan for this work). Skim `docs/CHECKER.md`
if you call into the checker.

## Where you work

- Your git worktree `$TSRS_WORK/wt/lsp-<wave>` on branch `lsp-<wave>` (created for you from branch `lsp`).
  Work only there. `ts-ref` inside it is a symlink to the Go reference checkout (read-only). `$TSRS_WORK` =
  `/Users/maxschwenk/Developer/tsrs-work`.
- Build with your own target dir: `CARGO_TARGET_DIR=$PWD/target cargo check -p <crate> --message-format short`.
  Your crates must compile with 0 errors and 0 warnings (workspace lints already silence the noisy kinds).
- Commit early and often inside your worktree (`git add -A . && git commit -m "<crate>: <what>"`). Do not push,
  merge or rebase onto anything; the lead lands your branch onto `lsp`.
- Scratch files: `target/scratch/<wave>/` (git-ignored), never in the source tree.

## How to port

- The Go source (`ts-ref/tsc/internal/...`) is the specification. One Rust file per Go file, same base name in
  snake_case, functions in Go's order, each preceded by an origin marker comment `// file.go:LINE` (the Go line of
  the `func`). Same names (snake_case), same decomposition, same order of side effects, Go quirks preserved. No
  redesign, no "improvements", no doc comments restating code. Keep Go comments that explain why.
- Go `context.Context` -> `tsrs_core::context::Context` (cheap `Clone`; request id, checker lifetime, cancellation).
- Goroutines / channels / `sync.WaitGroup` -> OS threads and `std::sync` primitives with the same semantics (see
  LSP.md "Threading model"); never a thread per request (each thread owns a leak arena).
- Protocol types are generated in `tsrs_lsproto` (LSP.md "Protocol types and JSON"): Go `lsproto.Foo` ->
  `tsrs_lsproto::Foo`, fields snake_case, unions are structs of `Option`s with Go's field names, enums are newtypes
  with associated consts (`lsproto.SymbolKindFile` -> `SymbolKind::File`).
- Out of scope (keep the branch structure, take the "not mapped / not enabled" path, no `todo!()`): content
  mappers / span maps, the `api` package, ATA, telemetry, pprof, localization (English only).
- Never leave `todo!()` / `unimplemented!()` for in-scope behavior without listing it in your notes and report.
- Only edit the crates and files your brief assigns. If you need something small in another crate (a missing AST
  utility, a `pub` on a field), add it minimally and additively and list it under "Shared-file edits" in your notes.

## Notes file

Keep `notes/lsp-<wave>.md` in your worktree (commit it): what was ported (Go file -> Rust file), `## Shared-file
edits`, `## Deviations` (anything that differs from Go and why), `## Needs from others`, `## Doubts`.

## Done means

Everything in your brief ported, compiling with 0 warnings, the gates in your brief green, everything committed,
notes up to date. Final report <= 15 lines: what is done, what is not and why, gate results, top doubts.
