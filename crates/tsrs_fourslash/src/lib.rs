// Port of Go's fourslash test harness (internal/fourslash) and its tests (internal/fourslash/tests, converted by
// tools/gen-fourslash into tests/gen). See docs/LSP.md "Fourslash".

pub mod baselineutil;
pub mod contentmapper;
pub mod contentmappertest;
pub mod fourslash;
pub mod go;
pub mod harnessutil;
pub mod runner;
pub mod semantictokens;
pub mod statebaseline;
pub mod stringtestutil;
pub mod test_parser;
pub mod testing;
pub mod testrunner;
pub mod tests;
pub mod testutil;
pub mod tsbaseline;

#[cfg(test)]
mod test_parser_test;
