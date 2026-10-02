// Go internal/project/ata is out of scope (docs/LSP.md): typings are never installed. Only the value type the
// project code compares is kept (ata.go:21, :27); `Session` never creates a typings installer.

use std::sync::Arc;

use tsrs_core::collections::Set;
use tsrs_core::{CompilerOptions, TypeAcquisition, P};

#[derive(Clone, Debug, Default)]
pub struct TypingsInfo {
    pub type_acquisition: Option<TypeAcquisition>,
    pub compiler_options: Option<P<CompilerOptions>>,
    pub unresolved_imports: Option<Arc<Set<String>>>,
}

impl TypingsInfo {
    // ata.go:27
    pub fn equals(&self, other: &TypingsInfo) -> bool {
        TypeAcquisition::equals(self.type_acquisition.as_ref(), other.type_acquisition.as_ref())
            && self.compiler_options.unwrap().get_allow_js() == other.compiler_options.unwrap().get_allow_js()
            && match (&self.unresolved_imports, &other.unresolved_imports) {
                (None, None) => true,
                (Some(a), Some(b)) => a.equals(b),
                (Some(a), None) | (None, Some(a)) => a.len() == 0,
            }
    }
}

// Go `ata.NpmExecutor`: the session keeps the hook; nothing calls it while ATA is not ported.
pub trait NpmExecutor: Send + Sync {
    fn npm_install(&self, cwd: &str, npm_install_args: &[String]) -> Result<Vec<u8>, String>;
}
