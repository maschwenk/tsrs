// Embeds the release version shown by `tsrs --version`: the workspace version plus the TypeScript reference
// (`[workspace.metadata.typescript]` in the workspace Cargo.toml), in the same shape npm/build.mjs uses for the
// npm package version.

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).expect("read workspace Cargo.toml");

    let ts_version = metadata_value(&text, "version");
    let ts_commit = metadata_value(&text, "commit");
    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap();

    println!("cargo:rustc-env=TSRS_RELEASE_VERSION={pkg_version}-ts{ts_version}");
    println!("cargo:rustc-env=TSRS_TYPESCRIPT_COMMIT={}", &ts_commit[..ts_commit.len().min(12)]);
}

// `key = "value"` inside the `[workspace.metadata.typescript]` table.
fn metadata_value(text: &str, key: &str) -> String {
    let mut in_table = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[workspace.metadata.typescript]";
            continue;
        }
        if !in_table {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return v.trim().trim_matches('"').to_string();
            }
        }
    }
    panic!("missing `{key}` in [workspace.metadata.typescript] of the workspace Cargo.toml");
}
