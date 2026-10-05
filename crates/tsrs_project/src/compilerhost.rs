use std::fmt::Display;
use std::sync::{Arc, RwLock};

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::CompilerHost;
use tsrs_core::context::Context;
use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_diagnostics::Message;
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::FS;

use crate::configfileregistry::ConfigFileRegistry;
use crate::configfileregistrybuilder::configFileRegistryBuilder;
use crate::logging::LogTree;
use crate::parsecache::{new_parse_cache_key, ParseCache, ParseCacheJournal};
use crate::project::{Project, ID};
use crate::projectcollectionbuilder::ProjectCollectionBuilder;
use crate::session::SessionOptions;
use crate::snapshotfs::{new_source_fs, sourceFS, FileHandleSource, SnapshotFS};

// The parts of the ProjectCollectionBuilder the host uses while the program is built (Go keeps the whole
// builder pointer and clears it in `freeze`).
pub(crate) struct hostBuilder {
    pub(crate) parse_cache: Arc<ParseCache>,
    pub(crate) parse_cache_journal: Arc<ParseCacheJournal>,
    pub(crate) config_file_registry_builder: Arc<configFileRegistryBuilder>,
    pub(crate) ctx: Context,
}

// compilerhost.go:21 (Go's configFilePath field is never read)
pub(crate) struct compilerHost {
    current_directory: String,
    pub(crate) session_options: Arc<SessionOptions>,

    pub(crate) source_fs: Arc<sourceFS>,
    config_file_registry: RwLock<Option<Arc<ConfigFileRegistry>>>,

    // Go keeps `project *Project`, the project being updated, and reads only its ID.
    project: RwLock<Option<ID>>,
    builder: RwLock<Option<hostBuilder>>,
    logger: RwLock<LogTree>,
}

// compilerhost.go:36
pub(crate) fn new_compiler_host(current_directory: &str, project: &Project, builder: &ProjectCollectionBuilder, logger: LogTree) -> Arc<compilerHost> {
    Arc::new(compilerHost {
        current_directory: current_directory.to_string(),
        session_options: Arc::clone(&builder.session_options),

        source_fs: Arc::new(new_source_fs(true, Arc::<crate::snapshotfs::snapshotFSBuilder>::clone(&builder.fs), Arc::clone(&builder.to_path))),
        config_file_registry: RwLock::new(None),

        project: RwLock::new(Some(project.id())),
        builder: RwLock::new(Some(hostBuilder {
            parse_cache: Arc::clone(&builder.parse_cache),
            parse_cache_journal: Arc::clone(&builder.parse_cache_journal),
            config_file_registry_builder: Arc::clone(&builder.config_file_registry_builder),
            ctx: builder.ctx.clone(),
        })),
        logger: RwLock::new(logger),
    })
}

impl compilerHost {
    // compilerhost.go:57
    // freeze clears references to mutable state to make the compilerHost safe for use
    // after the snapshot has been finalized. See the usage in snapshot.go for more details.
    pub(crate) fn freeze(&self, snapshot_fs: Arc<SnapshotFS>, config_file_registry: Arc<ConfigFileRegistry>) {
        let mut builder = self.builder.write().unwrap();
        if builder.is_none() {
            panic!("freeze can only be called once");
        }
        self.source_fs.set_source(snapshot_fs);
        self.source_fs.disable_tracking();
        *self.config_file_registry.write().unwrap() = Some(config_file_registry);
        *builder = None;
        *self.project.write().unwrap() = None;
        *self.logger.write().unwrap() = LogTree::nil();
    }

    // compilerhost.go:69
    fn ensure_alive(&self) {
        if self.builder.read().unwrap().is_none() || self.project.read().unwrap().is_none() {
            panic!("method must not be called after snapshot initialization");
        }
    }

    /// The builder's parse cache and the journal of the references its programs take.
    pub(crate) fn builder_parse_cache_journal(&self) -> (Arc<ParseCache>, Arc<ParseCacheJournal>) {
        let builder = self.builder.read().unwrap();
        let builder = builder.as_ref().expect("method must not be called after snapshot initialization");
        (Arc::clone(&builder.parse_cache), Arc::clone(&builder.parse_cache_journal))
    }

    pub(crate) fn builder_ctx(&self) -> Context {
        self.builder.read().unwrap().as_ref().map(|b| b.ctx.clone()).unwrap_or_default()
    }
}

impl CompilerHost for compilerHost {
    // compilerhost.go:81
    fn fs(&self) -> &dyn FS {
        &*self.source_fs
    }

    // compilerhost.go:76
    fn default_library_path(&self) -> &str {
        &self.session_options.default_library_path
    }

    // compilerhost.go:86
    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }

    // compilerhost.go:91
    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>> {
        // Config files outlive the program that asks for them (the registry keeps them): never in the program's
        // memory region.
        let _arena = tsrs_core::arena::enter_thread_arena();
        let builder = self.builder.read().unwrap();
        match builder.as_ref() {
            None => self.config_file_registry.read().unwrap().as_ref().unwrap().get_config(&path),
            Some(builder) => {
                // acquireConfigForProject will bypass sourceFS, so track the file here.
                self.source_fs.track(file_name);
                let project = self.project.read().unwrap().clone().unwrap();
                let logger = self.logger.read().unwrap().clone();
                builder.config_file_registry_builder.acquire_config_for_project(file_name, &path, &project, &logger)
            }
        }
    }

    // compilerhost.go:103
    // GetSourceFile implements compiler.CompilerHost. Files are cached in parseCache
    // and acquired immediately for the in-progress program.
    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        self.ensure_alive();
        let fh = self.source_fs.get_file_by_path(&opts.file_name, &opts.path)?;
        let key = new_parse_cache_key(opts, fh.hash(), fh.kind());
        let (cache, journal) = self.builder_parse_cache_journal();
        Some(journal.acquire(&cache, key, fh))
    }

    // compilerhost.go:172
    fn trace(&self, msg: &'static Message, args: &[&dyn Display]) {
        self.logger.read().unwrap().log(&msg.localize(args));
    }
}
