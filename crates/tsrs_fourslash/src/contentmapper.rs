use std::fmt;
use std::sync::Arc;

// Go's contentmapper.Spawner, as FourslashOptions holds it: a tsrs_contentmapper spawner. The language server does
// not run content mappers yet (phase 2, notes/contentmappers.md), so the harness still rejects a test that sets one.
#[derive(Clone)]
pub struct Spawner(pub Arc<dyn tsrs_contentmapper::Spawner>);

impl fmt::Debug for Spawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Spawner")
    }
}
