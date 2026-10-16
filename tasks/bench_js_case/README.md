# Verify and measure JavaScript casing

```sh
cargo run --locked -p tsrs_bench_js_case -- verify
cargo build --release --locked -p tsrs_bench_js_case
/usr/bin/time -l target/release/tsrs_bench_js_case unicode-lower # macOS; bench/count.py on Linux
```

`verify` hashes lowercase and uppercase results for all code points (including lone surrogates) and 4,913
contextual combinations. Build it against each revision to compare the digest; the baseline is `8a50f0eab6fa3087`
on 64-bit platforms.

Each workload performs one million casing calls: `ascii-lower`, `ascii-upper`, `ascii-unchanged`, `unicode-lower`,
or `unicode-upper`. Build each revision in release mode and compare the executable's resource counters.
