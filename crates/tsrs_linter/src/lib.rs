//! Headless program loading; native rule dispatch lives in the type checker.
mod linter;
#[cfg(test)]
mod tests;

pub use linter::*;
pub use tsrs_checker::lint::*;
