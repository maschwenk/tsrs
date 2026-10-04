use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use tsrs_core::arena::Region;

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::DuplicateSourceFile;
use tsrs_core::{ensure_script_kind_from_file_name, ScriptKind, P};

use crate::overlayfs::{FileHandle, Hash128};
use crate::refcountcache::{new_ref_count_cache, RefCountCache, RefCountCacheOptions};

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct ParseCacheKey {
    // Go embeds ast.SourceFileParseOptions.
    pub source_file_parse_options: SourceFileParseOptions,
    pub script_kind: ScriptKind,
    pub hash: Hash128,
}

// parsecache.go:22
pub fn new_parse_cache_key(options: SourceFileParseOptions, hash: Hash128, mut script_kind: ScriptKind) -> ParseCacheKey {
    if script_kind == ScriptKind::Unknown {
        script_kind = ensure_script_kind_from_file_name(&options.file_name);
    }
    ParseCacheKey { source_file_parse_options: options, hash, script_kind }
}

// ContentMappedParseCacheKey identifies the complete output bundle for one mapped input. Hash folds the
// original content, mapper transform identity, and diagnostic locale together.
// (Content mappers are not ported: the type exists for the cache's signature; no entry is ever created.)
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct ContentMappedParseCacheKey {
    pub source_file_parse_options: SourceFileParseOptions,
    pub hash: Hash128,
}

// parsecache.go:55
// parseCacheKeyForFile reconstructs the ordinary parse-cache key for a source file held by a program.
pub(crate) fn parse_cache_key_for_file(file: P<SourceFile>) -> ParseCacheKey {
    new_parse_cache_key(file.parse_options().clone(), file.hash.get(), file.script_kind())
}

// parsecache.go:64
// parseCacheKeyForDuplicate reconstructs an ordinary parse-cache key for a deduplicated source file.
pub(crate) fn parse_cache_key_for_duplicate(file: &DuplicateSourceFile) -> ParseCacheKey {
    new_parse_cache_key(file.parse_options.clone(), file.hash, file.script_kind)
}

pub type ParseCache = RefCountCache<ParseCacheKey, P<SourceFile>, Arc<dyn FileHandle>>;

/// The parse-cache references one snapshot clone takes for the programs it builds (`compilerHost::get_source_file`
/// acquires, the program-update refs and derefs in `Project::create_program`). They are handed to the new snapshot,
/// which releases them in `Snapshot::dispose`. If the clone unwinds before a snapshot exists (a panic while building
/// a program, e.g. the module resolver's `Unexpected moduleResolution`), nothing else would release them: the
/// clone rolls them back with this record so the files' regions are freed.
#[derive(Default)]
pub(crate) struct ParseCacheJournal(Mutex<FxHashMap<ParseCacheKey, i64>>);

impl ParseCacheJournal {
    pub(crate) fn acquire(&self, cache: &ParseCache, key: ParseCacheKey, fh: Arc<dyn FileHandle>) -> P<SourceFile> {
        let file = cache.acquire(key.clone(), fh);
        *self.0.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_default() += 1;
        file
    }

    pub(crate) fn ref_(&self, cache: &ParseCache, key: ParseCacheKey) {
        cache.ref_(key.clone());
        *self.0.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_default() += 1;
    }

    pub(crate) fn deref(&self, cache: &ParseCache, key: &ParseCacheKey) {
        cache.deref(key);
        *self.0.lock().unwrap_or_else(|e| e.into_inner()).entry(key.clone()).or_default() -= 1;
    }

    /// Releases every reference this clone still holds (its programs never reached a snapshot).
    pub(crate) fn roll_back(&self, cache: &ParseCache) {
        let held = std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()));
        for (key, count) in held {
            for _ in 0..count.max(0) {
                cache.deref(&key);
            }
        }
    }
}

// parsecache.go:74
//
// Memory (docs/LSP.md "Memory plan for a long-lived server"): each parsed file version gets its own region; parse
// and bind (and later, through `arena::enter_owner`, the file's lazily filled data) allocate there. The cache entry
// owns the region until its final Deref, and every program that contains the file owns it too (`programOwner`), so
// it is freed once neither the cache nor any live program refers to the file, as Go's GC would.
pub fn new_parse_cache(options: RefCountCacheOptions) -> ParseCache {
    let regions: Arc<Mutex<FxHashMap<usize, Region>>> = Arc::default();
    let parse_regions = regions.clone();
    let mut cache = new_ref_count_cache(options, move |key: &ParseCacheKey, fh: Arc<dyn FileHandle>| {
        let text = fh.content();
        let region = Region::new(file_region_first_chunk(text.len()));
        let file = {
            let _scope = region.enter();
            let file = tsrs_parser::parse_source_file(key.source_file_parse_options.clone(), text, key.script_kind);
            file.hash.set(fh.hash());
            tsrs_binder::bind_source_file(file);
            file
        };
        region.trim();
        let text_index = file.text_index.get();
        region.on_free(Box::new(move || tsrs_ast::unregister_source_text(text_index)));
        parse_regions.lock().unwrap().insert(file.addr(), region);
        file
    });
    let evict_regions = regions.clone();
    cache.on_evict = Some(Box::new(move |file: &P<SourceFile>| {
        let region = evict_regions.lock().unwrap().remove(&file.addr());
        drop(region);
    }));
    cache.on_revive = Some(Box::new(move |file: &P<SourceFile>| {
        // A live program still holds the region (it is the one re-referencing the file).
        if let Some(region) = Region::containing(file.addr()) {
            regions.lock().unwrap().insert(file.addr(), region);
        }
    }));
    cache
}

// The text is copied into the region; text, AST and binder data take about 8 times the text (measured on the
// private monorepo: 7.8x on average). Regions grow by a quarter when this is exceeded, and the region is trimmed to
// what it used after binding.
fn file_region_first_chunk(text_len: usize) -> usize {
    text_len.saturating_mul(10).saturating_add(4 << 10).min(64 << 20)
}

// Content mappers are not ported (docs/LSP.md): the canonical/supplemental bundle type is reduced to the
// canonical file, and nothing ever acquires from this cache.
pub struct ContentMappedParseCache {
    // One reference owns the canonical file and all supplemental files as a bundle. Callers ref and
    // deref the canonical file only.
    pub ref_count_cache: RefCountCache<ContentMappedParseCacheKey, P<SourceFile>, ()>,
}

// parsecache.go:92
pub fn new_content_mapped_parse_cache(options: RefCountCacheOptions) -> ContentMappedParseCache {
    ContentMappedParseCache {
        ref_count_cache: new_ref_count_cache(options, |_: &ContentMappedParseCacheKey, _: ()| -> P<SourceFile> {
            panic!("content-mapped source files must be produced with AcquireOrError")
        }),
    }
}
