// Port of execute/build/compilerHost.go.

use std::sync::Arc;

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::CompilerHost;
use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_diagnostics::Message;
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::FS;

use super::host::host;

pub(crate) struct compilerHost {
    pub(crate) host: Arc<host>,
    pub(crate) trace: Box<dyn Fn(&'static Message, &[&dyn std::fmt::Display]) + Send + Sync>,
}

impl CompilerHost for compilerHost {
    fn fs(&self) -> &dyn FS {
        CompilerHost::fs(&*self.host)
    }

    fn default_library_path(&self) -> &str {
        self.host.default_library_path()
    }

    fn get_current_directory(&self) -> &str {
        CompilerHost::get_current_directory(&*self.host)
    }

    fn trace(&self, msg: &'static Message, args: &[&dyn std::fmt::Display]) {
        (self.trace)(msg, args)
    }

    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        self.host.get_source_file(opts)
    }

    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>> {
        self.host.get_resolved_project_reference(file_name, path)
    }
}
