// The tsc driver (Go's internal/execute): `tsc` and `tsc -b` over a `tsc::System`. The `tsrs` binary
// (crates/tsrs_cli) runs it over the OS file system; crates/tsrs_wasm runs it over a host file system.

pub mod build;
pub mod execute;
pub mod sys;
pub mod tsc;
#[cfg(test)]
mod tsctests;

/// Alloc-profile builds: the heap census the binary runs after a compilation (`tsrs_cli`'s `census::run`), given
/// the program and the addresses of the objects it reports as roots.
#[cfg(feature = "alloc-profile")]
pub type CensusHook = fn(&'static tsrs_compiler::Program, &[usize]);

#[cfg(feature = "alloc-profile")]
static CENSUS_HOOK: std::sync::OnceLock<CensusHook> = std::sync::OnceLock::new();

/// Registers the census `execute` runs after a compilation (alloc-profile builds; the binary does this at startup).
#[cfg(feature = "alloc-profile")]
pub fn set_census_hook(hook: CensusHook) {
    let _ = CENSUS_HOOK.set(hook);
}
