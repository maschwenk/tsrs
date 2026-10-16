# Ad hoc tasks

Repository maintenance and generation tasks live here, with one Rust crate per directory. Each task is a Cargo
workspace member. Run tasks from the repository root with `cargo run --locked -p <package> -- <arguments>`.

| Task | Package | Purpose |
| --- | --- | --- |
| [gen_stringutil](gen_stringutil) | `tsrs_gen_stringutil` | Generate compact Unicode casing tables from pinned source data. |
| [bench_js_case](bench_js_case) | `tsrs_bench_js_case` | Verify and measure JavaScript string casing. |

Use the dev profile for generation and correctness checks, and `--release` for performance measurements.
