use std::sync::Arc;

use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};

use crate::overlayfs::Hash128;
use crate::ownercache::{new_owner_cache, OwnerCache};
use crate::snapshotfs::FileSource;

pub struct ExtendedConfigParseArgs {
    pub file_name: String,
    pub content: String,
    pub fs: Arc<dyn FileSource>,
    pub resolution_stack: Vec<Path>,
    // tsoptions keeps hosts as `&'static dyn` (see `assume_static`); both live for the duration of the parse.
    pub host: &'static dyn ParseConfigHost,
    pub cache: &'static dyn tsoptions::ExtendedConfigCache,
}

pub struct ExtendedConfigCacheEntry {
    // Go embeds *tsoptions.ExtendedConfigCacheEntry.
    pub extended_config_cache_entry: P<tsoptions::ExtendedConfigCacheEntry>,
    pub hash: Hash128,
}

pub type ExtendedConfigCache = OwnerCache<Path, Arc<ExtendedConfigCacheEntry>, ExtendedConfigParseArgs>;

// extendedconfigcache.go:25
pub fn new_extended_config_cache() -> ExtendedConfigCache {
    new_owner_cache(
        |path: &Path, args: &ExtendedConfigParseArgs| {
            let extended_config_cache_entry =
                tsoptions::parse_extended_config(&args.file_name, path.clone(), &args.resolution_stack, args.host, Some(args.cache));
            let hash = hash(&extended_config_cache_entry, args);
            Arc::new(ExtendedConfigCacheEntry { extended_config_cache_entry, hash })
        },
        Some(Box::new(|_path: &Path, entry: &Arc<ExtendedConfigCacheEntry>, args: &ExtendedConfigParseArgs| {
            entry.hash == 0 || entry.hash != hash(&entry.extended_config_cache_entry, args)
        })),
    )
}

// extendedconfigcache.go:40
fn hash(entry: &tsoptions::ExtendedConfigCacheEntry, args: &ExtendedConfigParseArgs) -> Hash128 {
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    hasher.update(args.content.as_bytes());
    for file_name in entry.extended_file_names() {
        let Some(fh) = args.fs.get_file(&file_name) else {
            return 0;
        };
        hasher.update(fh.content().as_bytes());
    }
    hasher.digest128()
}

// tsoptions takes `&'static dyn ParseConfigHost` / `ExtendedConfigCache` (the CLI leaks its hosts), but reads them
// only while the parse call runs (the resolver host it allocates in the arena is never read afterwards). The
// project system's hosts are per-snapshot builders, so they are passed with an assumed 'static lifetime that
// the call does not outlive.
pub(crate) fn assume_static<T: ?Sized>(r: &T) -> &'static T {
    // SAFETY: see above; callers pass references that stay valid until the tsoptions call returns.
    unsafe { &*(r as *const T) }
}
