// Port of execute/build/host.go.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use rustc_hash::FxHashMap;
use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::CompilerHost;
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_diagnostics::Message;
use tsrs_incremental::{BuildInfo, BuildInfoReader};
use tsrs_tsoptions::{self as tsoptions, CompilerOptionsValue, ParseConfigHost, ParsedCommandLine};
use tsrs_vfs::FS;

use super::orchestrator::Orchestrator;
use super::parsecache::parseCache;
use crate::tsc::ExtendedConfigCache;

pub(crate) type mTimeCache = Arc<Mutex<FxHashMap<Path, Option<SystemTime>>>>;

pub(crate) struct host {
    pub(crate) orchestrator: OnceLock<&'static Orchestrator>,
    pub(crate) host: Arc<dyn CompilerHost>,
    // The caching filesystem under `host` (Go `o.host.host.FS().(*cachedvfs.FS)`), cleared by `resetCaches`.
    pub(crate) cached_fs: Arc<tsrs_vfs::cachedvfs::FS<Arc<dyn tsrs_vfs::FS>>>,

    // Caches that last only for build cycle and then cleared out
    pub(crate) extended_config_cache: Mutex<Arc<ExtendedConfigCache>>,
    pub(crate) source_files: parseCache<SourceFileParseOptions, Option<P<SourceFile>>>,
    pub(crate) config_times: Mutex<FxHashMap<Path, Duration>>,

    // caches that stay as long as they are needed
    pub(crate) resolved_references: parseCache<Path, Option<P<ParsedCommandLine>>>,
    pub(crate) m_times: Mutex<mTimeCache>,
}

impl host {
    fn o(&self) -> &'static Orchestrator {
        *self.orchestrator.get().unwrap()
    }

    // host.go:118
    pub(crate) fn load_or_store_m_time(&self, file: &str, old_cache: Option<&mTimeCache>, store: bool) -> Option<SystemTime> {
        let path = self.o().to_path(file);
        let m_times = self.m_times.lock().unwrap().clone();
        if let Some(existing) = m_times.lock().unwrap().get(&path) {
            return *existing;
        }
        let mut found = None;
        if let Some(old_cache) = old_cache {
            found = old_cache.lock().unwrap().get(&path).copied();
        }
        let mut m_time = match found {
            Some(m_time) => m_time,
            None => tsrs_incremental::get_m_time(&*self.host, file),
        };
        if store {
            m_time = *m_times.lock().unwrap().entry(path).or_insert(m_time);
        }
        m_time
    }

    // host.go:137
    pub(crate) fn store_m_time(&self, file: &str, m_time: SystemTime) {
        let path = self.o().to_path(file);
        self.m_times.lock().unwrap().lock().unwrap().insert(path, Some(m_time));
    }

    // host.go:142
    #[expect(dead_code, reason = "its only Go caller, BuildTask.updateWatch (build --watch), is not ported")]
    pub(crate) fn store_m_time_from_old_cache(&self, file: &str, old_cache: &mTimeCache) {
        let path = self.o().to_path(file);
        let m_time = old_cache.lock().unwrap().get(&path).copied();
        if let Some(m_time) = m_time {
            self.m_times.lock().unwrap().lock().unwrap().insert(path, m_time);
        }
    }

    // host.go:106
    pub(crate) fn get_m_time(&self, file: &str) -> Option<SystemTime> {
        self.load_or_store_m_time(file, None, true)
    }

    // host.go:110
    pub(crate) fn set_m_time(&self, file: &str, m_time: SystemTime) -> Result<(), String> {
        CompilerHost::fs(self).chtimes(file, m_time, m_time)
    }
}

impl ParseConfigHost for host {
    fn fs(&self) -> &dyn FS {
        self.host.fs()
    }

    fn get_current_directory(&self) -> &str {
        self.host.get_current_directory()
    }
}

impl CompilerHost for host {
    fn fs(&self) -> &dyn FS {
        self.host.fs()
    }

    fn default_library_path(&self) -> &str {
        self.host.default_library_path()
    }

    fn get_current_directory(&self) -> &str {
        self.host.get_current_directory()
    }

    fn trace(&self, _msg: &'static Message, _args: &[&dyn std::fmt::Display]) {
        panic!("build.Orchestrator.host does not support tracing; use a different host for tracing");
    }

    // host.go:55
    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        if tspath::is_declaration_file_name(&opts.file_name) || tspath::file_extension_is(&opts.file_name, tspath::EXTENSION_JSON) {
            // Cache dts and json files as they will be reused
            return self.source_files.load_or_store(&opts, |opts| self.host.get_source_file(opts.clone()), false /* allowZero */);
        }
        self.host.get_source_file(opts)
    }

    // host.go:71
    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>> {
        self.resolved_references.load_or_store(
            &path,
            |path| {
                let o = self.o();
                let config_start = o.opts.sys.now();
                // Wrap command line options in "compilerOptions" key to match tsconfig.json structure
                let command_line_raw = match &o.opts.command.raw {
                    CompilerOptionsValue::Object(_) => {
                        let mut wrapped = tsrs_core::collections::OrderedMap::default();
                        wrapped.insert("compilerOptions".to_string(), o.opts.command.raw.clone());
                        Some(CompilerOptionsValue::Object(wrapped))
                    }
                    _ => None,
                };
                let this: &'static host = o.host();
                let extended_config_cache = self.extended_config_cache.lock().unwrap().clone();
                let (command_line, _) = tsoptions::get_parsed_command_line_of_config_file_path(
                    file_name,
                    path.clone(),
                    Some(&o.opts.command.compiler_options),
                    command_line_raw.as_ref(),
                    this,
                    Some(&*extended_config_cache),
                );
                let config_time = o.opts.sys.now() - config_start;
                self.config_times.lock().unwrap().insert(path.clone(), config_time);
                command_line.map(P::new)
            },
            true, /* allowZero */
        )
    }
}

impl BuildInfoReader for host {
    // host.go:99
    fn read_build_info(&self, config: &ParsedCommandLine) -> Option<BuildInfo> {
        let o = self.o();
        let config_path = o.to_path(config.config_name());
        let task = o.get_task(&config_path);
        let (build_info, _) = task.load_or_store_build_info(o, &o.to_path(config.config_name()), &config.get_build_info_file_name());
        build_info.map(|b| (*b).clone())
    }
}

// `incremental.Host` over the orchestrator's host (Go passes `orchestrator.host` itself).
pub(crate) struct incrementalHost(pub(crate) &'static host);

impl tsrs_incremental::Host for incrementalHost {
    fn fs(&self) -> &dyn FS {
        self.0.host.fs()
    }

    fn get_m_time(&self, file_name: &str) -> Option<SystemTime> {
        self.0.get_m_time(file_name)
    }

    fn set_m_time(&self, file_name: &str, m_time: Option<SystemTime>) -> Result<(), String> {
        self.0.set_m_time(file_name, m_time.unwrap_or(SystemTime::UNIX_EPOCH))
    }
}
