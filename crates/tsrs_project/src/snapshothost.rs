use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_core::context::Context;
use tsrs_core::tspath::{self, Path};
use tsrs_core::{get_script_kind_from_file_name, ScriptKind, P};
use tsrs_ls::lsutil;
use tsrs_lsproto as lsproto;
use tsrs_vfs::FS;

use crate::extendedconfigcache::{new_extended_config_cache, ExtendedConfigCache};
use crate::filechange::FileChangeSummary;
use crate::logging::Logger;
use crate::overlayfs::{new_cached_file_handle, new_overlay, new_overlay_fs, FsRef, ToPath};
use crate::parsecache::{new_content_mapped_parse_cache, new_parse_cache, new_parse_cache_key, ContentMappedParseCache, ParseCache, ParseCacheKey};
use crate::programcounter::programCounter;
use crate::refcountcache::RefCountCacheOptions;
use crate::session::{SessionInit, SessionOptions, UpdateReason};
use crate::snapshot::{APISnapshotRequest, Snapshot, SnapshotChange};
use crate::snapshotfs::new_snapshot_fs;
use crate::watch::{allWatchKinds, get_recursive_glob_pattern, new_watched_files, PatternsAndIgnored};

// SnapshotHost owns the services shared by a collection of immutable snapshots.
pub struct SnapshotHost {
    pub(crate) options: Arc<SessionOptions>,
    pub(crate) to_path: ToPath,
    pub(crate) fs: Arc<dyn FS>,

    pub(crate) parse_cache: Arc<ParseCache>,
    pub(crate) content_mapped_parse_cache: Arc<ContentMappedParseCache>,
    pub(crate) extended_config_cache: Arc<ExtendedConfigCache>,
    pub(crate) program_counter: programCounter,
    // (Content mappers are not ported: Go's `contentMapperHost` is always nil here.)
    snapshot_id: AtomicU64,
}

pub struct SourceFileLease {
    cache: Arc<ParseCache>,
    key: ParseCacheKey,
    source_file: P<SourceFile>,
    released: Mutex<bool>,
}

impl SourceFileLease {
    // snapshothost.go:41
    pub fn source_file(&self) -> P<SourceFile> {
        self.source_file
    }

    // snapshothost.go:45
    pub fn release(&self) {
        let mut released = self.released.lock().unwrap();
        if !*released {
            *released = true;
            self.cache.deref(&self.key);
        }
    }
}

impl SnapshotHost {
    // snapshothost.go:51
    pub(crate) fn next_snapshot_id(&self) -> u64 {
        self.snapshot_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    // snapshothost.go:55
    pub fn acquire_source_file(&self, options: SourceFileParseOptions, text: &str, script_kind: ScriptKind) -> SourceFileLease {
        let file_handle = new_cached_file_handle(&options.file_name, text);
        let key = new_parse_cache_key(options, file_handle.hash(), script_kind);
        SourceFileLease { cache: Arc::clone(&self.parse_cache), key: key.clone(), source_file: self.parse_cache.acquire(&key, file_handle), released: Mutex::new(false) }
    }

    // snapshothost.go:65
    pub fn acquire_existing_source_file(&self, key: ParseCacheKey) -> Option<SourceFileLease> {
        let source_file = self.parse_cache.acquire_existing(&key)?;
        Some(SourceFileLease { cache: Arc::clone(&self.parse_cache), key, source_file, released: Mutex::new(false) })
    }
}

// snapshothost.go:77
pub fn new_snapshot_host(init: &SessionInit) -> Arc<SnapshotHost> {
    let current_directory = init.options.current_directory.clone();
    let use_case_sensitive_file_names = init.fs.use_case_sensitive_file_names();
    let to_path: ToPath = Arc::new(move |file_name: &str| tspath::to_path(file_name, &current_directory, use_case_sensitive_file_names));
    let parse_cache = init.parse_cache.clone().unwrap_or_else(|| Arc::new(new_parse_cache(RefCountCacheOptions::default())));
    let content_mapped_parse_cache =
        init.content_mapped_parse_cache.clone().unwrap_or_else(|| Arc::new(new_content_mapped_parse_cache(RefCountCacheOptions::default())));

    Arc::new(SnapshotHost {
        options: Arc::clone(&init.options),
        to_path,
        fs: Arc::clone(&init.fs),
        parse_cache,
        content_mapped_parse_cache,
        extended_config_cache: Arc::new(new_extended_config_cache()),
        program_counter: programCounter::default(),
        snapshot_id: AtomicU64::new(0),
    })
}

impl SnapshotHost {
    // snapshothost.go:105
    // NewRootSnapshot creates an independent root snapshot.
    pub fn new_root_snapshot(self: &Arc<Self>) -> Arc<Snapshot> {
        self.new_root_snapshot_with(0, false)
    }

    // snapshothost.go:110
    // RetainSnapshot adds a reference to a snapshot owned by this host.
    pub fn retain_snapshot(&self, snapshot: &Snapshot) {
        snapshot.ref_();
    }

    /// Open a batch of files through the same discovery path as editor documents, without a live session.
    pub fn open_files(&self, ctx: &Context, base_snapshot: &Snapshot, file_names: &[String]) -> Result<Arc<Snapshot>, lsproto::Error> {
        let mut overlays = (*base_snapshot.overlays()).clone();
        let mut documents = Vec::with_capacity(file_names.len());
        for name in file_names {
            let name = tspath::get_normalized_absolute_path(name, self.get_current_directory());
            let content = base_snapshot.read_file(&name).ok_or_else(|| lsproto::Error::new(format!("cannot read requested file: {name}")))?;
            let kind = get_script_kind_from_file_name(&name);
            documents.push(tsrs_ls::lsconv::file_name_to_document_uri(&name));
            overlays.insert((self.to_path)(&name), Arc::new(new_overlay(name, content, 0, kind)));
        }
        let change = SnapshotChange { opened_documents: documents, ..Default::default() };
        Ok(base_snapshot.clone_snapshot(ctx, change, Arc::new(overlays), None, None))
    }

    // snapshothost.go:116
    // CloneSnapshot derives a snapshot from baseSnapshot without adopting it as any
    // canonical session state or performing session side effects.
    pub fn clone_snapshot(
        &self,
        ctx: &Context,
        base_snapshot: &Snapshot,
        file_changes: FileChangeSummary,
        api_request: Option<Arc<APISnapshotRequest>>,
    ) -> Result<Arc<Snapshot>, (Arc<Snapshot>, lsproto::Error)> {
        let mut change = SnapshotChange { file_changes, ..Default::default() };
        if let Some(api_request) = &api_request {
            change.fs = match &api_request.layered_file_system {
                Some(layered) => Some(FsRef::Layered(Arc::clone(layered))),
                None => api_request.file_system.clone().map(FsRef::Host),
            };
            change.file_system_override = change.fs.is_some();
            change.replace_file_system = api_request.replace_file_system;
        }
        change.api_request = api_request;
        let snapshot = self.update(ctx, base_snapshot, change);
        match snapshot.api_error.clone() {
            Some(err) => Err((snapshot, err)),
            None => Ok(snapshot),
        }
    }

    // snapshothost.go:137
    // update derives a snapshot from baseSnapshot without adopting it as any
    // canonical session state or performing session side effects.
    fn update(&self, ctx: &Context, base_snapshot: &Snapshot, change: SnapshotChange) -> Arc<Snapshot> {
        base_snapshot.clone_snapshot(ctx, change, base_snapshot.overlays(), None, None)
    }

    // snapshothost.go:143
    // CloneSnapshotWithAutoImports derives a snapshot with auto-import preparation without
    // adopting the clone in the background.
    pub fn clone_snapshot_with_auto_imports(&self, ctx: &Context, base_snapshot: &Snapshot, uri: &lsproto::DocumentUri, logger: Option<&dyn Logger>) -> Arc<Snapshot> {
        let mut change = SnapshotChange {
            reason: UpdateReason::RequestedLanguageServiceWithAutoImports,
            fs: Some(FsRef::Layered(Arc::clone(&base_snapshot.fs.fs))),
            file_system_override: base_snapshot.file_system_override,
            resource_request: base_snapshot.resource_request_for_document(uri),
            ..Default::default()
        };
        change.resource_request.auto_imports = uri.clone();
        base_snapshot.clone_snapshot(ctx, change, base_snapshot.overlays(), logger, None)
    }

    // snapshothost.go:154
    pub(crate) fn new_root_snapshot_with(self: &Arc<Self>, id: u64, relative_pattern_support: bool) -> Arc<Snapshot> {
        let file_system = new_overlay_fs(FsRef::Host(Arc::clone(&self.fs)), Arc::default(), self.options.position_encoding, Arc::clone(&self.to_path));
        Arc::new(self.new_snapshot(
            id,
            Arc::new(new_snapshot_fs(Arc::clone(&self.to_path), Arc::new(file_system))),
            Arc::default(),
            self.options.compiler_options_for_inferred_projects,
            lsutil::new_default_user_preferences(),
            None,
            Some(new_watched_files(
                "auto-import".to_string(),
                allWatchKinds,
                relative_pattern_support,
                Arc::new(|node_modules_dirs: &FxHashMap<Path, String>| {
                    let mut patterns: Vec<String> = node_modules_dirs.values().map(|dir| get_recursive_glob_pattern(dir)).collect();
                    patterns.sort();
                    PatternsAndIgnored { patterns_inside_workspace: patterns, ..Default::default() }
                }),
            )),
        ))
    }

    // snapshothost.go:184
    pub fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    // snapshothost.go:188
    pub fn get_current_directory(&self) -> &str {
        &self.options.current_directory
    }

    // snapshothost.go:192
    pub fn default_library_path(&self) -> &str {
        &self.options.default_library_path
    }

    // snapshothost.go:196
    pub fn close(&self) {
        // (Content mappers are not ported: there is no content mapper host to close.)
    }
}
