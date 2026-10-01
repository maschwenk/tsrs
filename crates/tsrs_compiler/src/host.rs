use std::fmt::Display;
use std::sync::Arc;

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_core::tspath::Path;
use tsrs_core::{ensure_script_kind_from_file_name, P};
use tsrs_diagnostics::Message;
use tsrs_tsoptions::{ExtendedConfigCache, ParsedCommandLine};
use tsrs_vfs::FS;

pub type TraceFn = dyn Fn(&'static Message, &[&dyn Display]) + Send + Sync;

// Content mappers are not ported; GetContentMappedSourceFiles/ContentMapperProject are omitted.
pub trait CompilerHost: Send + Sync {
    fn fs(&self) -> &dyn FS;
    fn default_library_path(&self) -> &str;
    fn get_current_directory(&self) -> &str;
    fn trace(&self, msg: &'static Message, args: &[&dyn Display]);
    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>>;
    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>>;
}

pub struct compilerHost {
    current_directory: String,
    fs: Arc<dyn FS>,
    default_library_path: String,
    extended_config_cache: Option<Arc<dyn ExtendedConfigCache + Send>>,
    trace: Box<TraceFn>,
}

pub fn new_cached_fs_compiler_host(
    current_directory: &str,
    fs: Arc<dyn FS>,
    default_library_path: &str,
    extended_config_cache: Option<Arc<dyn ExtendedConfigCache + Send>>,
    trace: Option<Box<TraceFn>>,
) -> Arc<dyn CompilerHost> {
    new_compiler_host(current_directory, Arc::new(tsrs_vfs::cachedvfs::from(fs)), default_library_path, extended_config_cache, trace)
}

pub fn new_compiler_host(
    current_directory: &str,
    fs: Arc<dyn FS>,
    default_library_path: &str,
    extended_config_cache: Option<Arc<dyn ExtendedConfigCache + Send>>,
    trace: Option<Box<TraceFn>>,
) -> Arc<dyn CompilerHost> {
    let trace = trace.unwrap_or_else(|| Box::new(|_: &'static Message, _: &[&dyn Display]| {}));
    Arc::new(compilerHost {
        current_directory: current_directory.to_string(),
        fs,
        default_library_path: default_library_path.to_string(),
        extended_config_cache,
        trace,
    })
}

impl CompilerHost for compilerHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    fn default_library_path(&self) -> &str {
        &self.default_library_path
    }

    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }

    fn trace(&self, msg: &'static Message, args: &[&dyn Display]) {
        (self.trace)(msg, args)
    }

    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        let text = self.fs().read_file(&opts.file_name)?;
        let script_kind = ensure_script_kind_from_file_name(&opts.file_name);
        Some(tsrs_parser::parse_source_file_owned(opts, text, script_kind))
    }

    fn get_resolved_project_reference(&self, _file_name: &str, _path: Path) -> Option<P<ParsedCommandLine>> {
        // Only reached through project references, which are not ported yet.
        let _ = &self.extended_config_cache;
        None
    }
}
