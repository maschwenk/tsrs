use std::sync::Arc;

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

// parsecache.go:74
pub fn new_parse_cache(options: RefCountCacheOptions) -> ParseCache {
    new_ref_count_cache(options, |key: &ParseCacheKey, fh: Arc<dyn FileHandle>| {
        let file = tsrs_parser::parse_source_file(key.source_file_parse_options.clone(), fh.content(), key.script_kind);
        file.hash.set(fh.hash());
        tsrs_binder::bind_source_file(file);
        file
    })
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
