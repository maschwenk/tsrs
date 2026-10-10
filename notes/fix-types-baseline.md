# fix-types-baseline: `.types` / `.symbols` baselines

The `.types` / `.symbols` baselines are a port of `testutil/tsbaseline/type_symbol_baseline.go`
(`crates/tsrs_testrunner/src/type_symbol_baseline.rs`, called from `verify_types_and_symbols` in compile.rs). The type
walk runs before the symbol walk on the same checker (Go order), each under its own panic guard. First run: 12,776 /
12,779 types and 12,779 / 12,779 symbols; the error-baseline suite was already matching on the non-error behavior, so
no checker divergence surfaced. The one checker-side fix was the printer escape below; with it, types 12,778 / 12,779.

## Internal-name prefix: U+007F stands for Go's 0xFE

Go's internal symbol-name prefix is the invalid byte 0xFE; tsrs uses U+007F (Rust strings must be valid UTF-8). The
checker treats any name that starts with it as internal (Go `name[0] == '\xFE'`). Only the printer's string escaper
needed the mapping: Go's `escapeStringWorker` decodes 0xFE as a stray `RuneError` and escapes it as `\uFFFD`, while
U+007F would be left raw (`(typeof E)["\uFFFDmissing"]` printed as `"<DEL>missing"` in enumWithBigint,
privateNameEnum). `tsrs_printer::escape_string_worker` therefore treats a *leading* U+007F followed by more text as
that stray byte. A U+007F elsewhere (or alone, `"\177"` in octalLiteralAndEscapeSequence) is user text and stays raw,
as in Go. Representation limit: a user string that starts with U+007F followed by more text would be escaped too.
