// Go internal/astnav lives in crate tsrs_astnav (Go's checker imports it); the tests stay here.
pub use tsrs_astnav::*;

#[cfg(test)]
mod tokens_test;
#[cfg(test)]
pub(crate) use tokens_test::{parse_for_test, repo_root};
