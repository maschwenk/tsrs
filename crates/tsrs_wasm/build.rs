// The module's shadow stack (address-taken locals; the first thing in linear memory, so an overflow traps instead of
// corrupting data). notes/wasm-build.md says how the size was chosen. Only for wasm targets: native test builds of
// this crate link as usual.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let family = std::env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    if family.split(',').any(|f| f == "wasm") {
        println!("cargo:rustc-cdylib-link-arg=-zstack-size=33554432");
    }
}
