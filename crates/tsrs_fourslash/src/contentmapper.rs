// Go's contentmapper.Spawner. Content mappers are out of scope (docs/LSP.md); the tests that configure one
// build a placeholder that the harness rejects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spawner;
