use std::sync::Arc;

use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};

use crate::overlayfs::Hash128;
use crate::ownercache::{new_owner_cache, OwnerCache};
use crate::snapshotfs::FileSource;

pub struct ExtendedConfigParseArgs<'a> {
    pub file_name: String,
    pub content: String,
    pub fs: Arc<dyn FileSource>,
    pub resolution_stack: Vec<Path>,
    pub host: &'a dyn ParseConfigHost,
    pub cache: &'a dyn tsoptions::ExtendedConfigCache,
}

pub struct ExtendedConfigCacheEntry {
    // Go embeds *tsoptions.ExtendedConfigCacheEntry.
    pub extended_config_cache_entry: P<tsoptions::ExtendedConfigCacheEntry>,
    pub hash: Hash128,
}

pub struct ExtendedConfigCache {
    entries: OwnerCache<Path, Arc<ExtendedConfigCacheEntry>>,
}

// extendedconfigcache.go:25
pub fn new_extended_config_cache() -> ExtendedConfigCache {
    ExtendedConfigCache { entries: new_owner_cache() }
}

impl ExtendedConfigCache {
    pub fn load_and_acquire(&self, path: &Path, owner: u64, args: &ExtendedConfigParseArgs<'_>) -> Arc<ExtendedConfigCacheEntry> {
        self.entries.load_and_acquire(
            path,
            owner,
            || {
                let extended_config_cache_entry =
                    tsoptions::parse_extended_config(&args.file_name, path.clone(), &args.resolution_stack, args.host, Some(args.cache));
                let hash = hash(&extended_config_cache_entry, args);
                Arc::new(ExtendedConfigCacheEntry { extended_config_cache_entry, hash })
            },
            |entry| entry.hash == 0 || entry.hash != hash(&entry.extended_config_cache_entry, args),
        )
    }
}
impl std::ops::Deref for ExtendedConfigCache {
    type Target = OwnerCache<Path, Arc<ExtendedConfigCacheEntry>>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

// extendedconfigcache.go:40
fn hash(entry: &tsoptions::ExtendedConfigCacheEntry, args: &ExtendedConfigParseArgs<'_>) -> Hash128 {
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
