use std::sync::{Arc, Barrier};

use tsrs_core::tspath;

use super::super::fs;
use super::super::os_test::{mklink, temp_dir};
use super::*;

// Changes made after a probe expose accidental cache misses. Production users keep inputs stable for one check.
#[test]
fn metadata_is_shared_between_file_and_directory_probes() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let file = format!("{tmp}/file.ts");
    let directory = format!("{tmp}/directory");
    let missing = format!("{tmp}/missing.ts");
    std::fs::write(&file, "file").unwrap();
    std::fs::create_dir(&directory).unwrap();

    let cache = ResolverFsCache::new();
    assert!(cache.file_exists(&file));
    assert!(cache.directory_exists(&directory));
    assert!(!cache.file_exists(&missing));

    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    std::fs::remove_dir(&directory).unwrap();
    std::fs::write(&directory, "file").unwrap();
    std::fs::create_dir(&missing).unwrap();

    assert!(!cache.directory_exists(&file));
    assert!(!cache.file_exists(&directory));
    assert!(!cache.directory_exists(&missing));
    let fresh = ResolverFsCache::new();
    assert!(fresh.directory_exists(&file));
    assert!(fresh.file_exists(&directory));
    assert!(fresh.directory_exists(&missing));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn package_reads_cache_success_and_failure_but_source_reads_stay_live() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let package = format!("{tmp}/package.json");
    let missing_package = format!("{tmp}/missing/package.json");
    let source = format!("{tmp}/source.ts");
    std::fs::create_dir(format!("{tmp}/missing")).unwrap();
    std::fs::write(&package, "\u{feff}{\"version\":1}").unwrap();
    std::fs::write(&source, "before").unwrap();

    let cache = ResolverFsCache::new();
    assert_eq!(cache.read_file(&package).as_deref(), Some("{\"version\":1}"));
    assert_eq!(cache.read_file(&missing_package), None);
    assert_eq!(cache.read_file(&source).as_deref(), Some("before"));

    std::fs::write(&package, "{\"version\":2}").unwrap();
    std::fs::write(&missing_package, "{}").unwrap();
    std::fs::write(&source, "after").unwrap();
    assert_eq!(cache.read_file(&format!("{tmp}/./package.json/")).as_deref(), Some("{\"version\":1}"));
    assert_eq!(cache.read_file(&missing_package), None);
    assert_eq!(cache.read_file(&source).as_deref(), Some("after"));

    let fresh = ResolverFsCache::new();
    assert_eq!(fresh.read_file(&package).as_deref(), Some("{\"version\":2}"));
    assert_eq!(fresh.read_file(&missing_package).as_deref(), Some("{}"));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn full_stat_and_directory_listings_are_cached_independently() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let file = format!("{tmp}/file.ts");
    let new_file = format!("{tmp}/new.ts");
    std::fs::write(&file, "before").unwrap();
    let cache = ResolverFsCache::new();
    let stat = cache.stat(&file).unwrap();
    let entries = cache.get_accessible_entries(&tmp);
    assert_eq!(stat.name(), "file.ts");
    assert_eq!(stat.size(), 6);
    assert_eq!(entries.files, ["file.ts"]);

    std::fs::write(&file, "after").unwrap();
    std::fs::write(&new_file, "new").unwrap();
    assert_eq!(cache.stat(&format!("{tmp}//file.ts/")), Some(stat));
    assert_eq!(cache.get_accessible_entries(&format!("{tmp}/./")), entries);
    // A cached listing is not evidence that an unlisted path is missing.
    assert!(cache.file_exists(&new_file));
    let fresh = ResolverFsCache::new();
    assert_eq!(fresh.stat(&file).unwrap().size(), 5);
    assert_eq!(fresh.get_accessible_entries(&tmp).files, ["file.ts", "new.ts"]);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn concurrent_interning_publishes_one_path_record() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let path = format!("{tmp}/a/b/c/module.ts");
    std::fs::create_dir_all(format!("{tmp}/a/b/c")).unwrap();
    std::fs::write(&path, "").unwrap();
    let canonical = fs().realpath(&path);
    let cache = ResolverFsCache::new();
    let barrier = Barrier::new(12);
    let records = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..12)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    assert!(cache.file_exists(&path));
                    assert!(!cache.directory_exists(&path));
                    assert_eq!(cache.realpath(&path), canonical);
                    cache.intern(&path)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    for record in &records[1..] {
        assert!(Arc::ptr_eq(&records[0], record));
    }
    assert!(!Arc::ptr_eq(&records[0], &cache.intern(&format!("{tmp}/a/b/c/other.ts"))));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn colliding_hashes_still_compare_the_complete_path() {
    let cache = ResolverFsCache::new();
    let first = cache.intern_with_hash("/cache-collision/first.ts", 0);
    let second = cache.intern_with_hash("/cache-collision/second.ts", 0);
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first, &cache.intern_with_hash("/cache-collision/first.ts", 0)));
    assert!(Arc::ptr_eq(&second, &cache.intern_with_hash("/cache-collision/second.ts", 0)));
}

#[cfg(unix)]
#[test]
fn parent_and_canonical_links_do_not_keep_path_records_alive() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let target = format!("{tmp}/target.ts");
    let link = format!("{tmp}/link.ts");
    std::fs::write(&target, "").unwrap();
    mklink("target.ts", &link, false);
    let (records, kept_child) = {
        let cache = ResolverFsCache::new();
        let canonical = fs().realpath(&target);
        assert_eq!(cache.realpath(&target), canonical);
        assert_eq!(cache.realpath(&canonical), canonical);
        assert_eq!(cache.realpath(&link), canonical);
        let records = [tmp.as_str(), target.as_str(), link.as_str(), canonical.as_str()].map(|path| Arc::downgrade(&cache.intern(path)));
        (records, cache.intern(&link))
    };
    assert!(kept_child.parent.as_ref().unwrap().upgrade().is_none());
    drop(kept_child);
    assert!(records.iter().all(|record| record.upgrade().is_none()));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn realpath_keeps_physical_dotdot_separate_from_normalized_probes() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    std::fs::create_dir_all(format!("{tmp}/real/sub")).unwrap();
    std::fs::write(format!("{tmp}/real/physical.ts"), "physical").unwrap();
    std::fs::write(format!("{tmp}/lexical.ts"), "lexical").unwrap();
    mklink("real/sub", &format!("{tmp}/link"), true);
    let cache = ResolverFsCache::new();
    let physical = format!("{tmp}/link/../physical.ts");
    let lexical = format!("{tmp}/link/../lexical.ts");

    assert!(!cache.file_exists(&physical));
    assert_eq!(cache.read_file(&physical), None);
    assert_eq!(cache.realpath(&physical), fs().realpath(&format!("{tmp}/real/physical.ts")));
    assert!(!cache.file_exists(&physical));
    assert_eq!(cache.realpath(&lexical), lexical);
    assert!(cache.file_exists(&lexical));
    assert_eq!(cache.read_file(&lexical).as_deref(), Some("lexical"));
    assert_eq!(cache.realpath(&lexical), lexical);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn realpath_and_metadata_match_os_for_symlinks_and_failures() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    std::fs::create_dir(format!("{tmp}/directory")).unwrap();
    std::fs::write(format!("{tmp}/directory/file.ts"), "").unwrap();
    std::fs::write(format!("{tmp}/directory/back\\slash.ts"), "").unwrap();
    mklink("directory", &format!("{tmp}/relative"), true);
    mklink(&format!("{tmp}/directory"), &format!("{tmp}/absolute"), true);
    mklink("directory/file.ts", &format!("{tmp}/file.ts"), false);
    mklink("missing.ts", &format!("{tmp}/dangling.ts"), false);
    mklink("cycle-b", &format!("{tmp}/cycle-a"), false);
    mklink("cycle-a", &format!("{tmp}/cycle-b"), false);
    let cache = ResolverFsCache::new();
    for relative in [
        "relative",
        "relative/file.ts",
        "absolute/file.ts",
        "file.ts",
        "dangling.ts",
        "cycle-a",
        "cycle-b",
        "missing.ts",
        "file.ts/child",
        "file.ts/",
        "directory//./file.ts",
        "directory/back\\slash.ts",
    ] {
        let path = format!("{tmp}/{relative}");
        let expected = fs().realpath(&path);
        assert_eq!(cache.realpath(&path), expected, "realpath({path})");
        assert_eq!(cache.file_exists(&path), fs().file_exists(&path), "file_exists({path})");
        assert_eq!(cache.directory_exists(&path), fs().directory_exists(&path), "directory_exists({path})");
        assert_eq!(cache.realpath(&path), expected, "cached realpath({path})");
    }
    assert_eq!(cache.get_accessible_entries(&tmp), fs().get_accessible_entries(&tmp));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn realpath_cache_is_fresh_for_each_compilation() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let target = format!("{tmp}/target.ts");
    let link = format!("{tmp}/link.ts");
    std::fs::write(&target, "").unwrap();
    mklink("target.ts", &link, false);
    let cache = ResolverFsCache::new();
    let expected = fs().realpath(&target);
    assert_eq!(cache.realpath(&link), expected);
    std::fs::remove_file(&link).unwrap();
    assert_eq!(cache.realpath(&link), expected);
    assert_eq!(ResolverFsCache::new().realpath(&link), link);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn realpath_preserves_os_casing_and_long_symlink_targets() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let relative = vec!["directory"; 32].join("/");
    let directory = format!("{tmp}/{relative}");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(format!("{directory}/MixedCase.ts"), "").unwrap();
    mklink(&relative, &format!("{tmp}/link"), true);
    let cache = ResolverFsCache::new();
    for path in [
        format!("{directory}/MixedCase.ts"),
        format!("{directory}/mixedcase.ts"),
        format!("{tmp}/link/MixedCase.ts"),
        format!("{tmp}/link/"),
    ] {
        assert_eq!(cache.realpath(&path), fs().realpath(&path));
        assert_eq!(cache.file_exists(&path), fs().file_exists(&path));
        assert_eq!(cache.directory_exists(&path), fs().directory_exists(&path));
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn denied_directory_matches_os_errors() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tspath::normalize_slashes(&temp_dir());
    let directory = format!("{tmp}/denied");
    let file = format!("{directory}/file.ts");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(&file, "").unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o0)).unwrap();
    let cache = ResolverFsCache::new();
    let actual = (cache.realpath(&file), cache.file_exists(&file), cache.stat(&file), cache.get_accessible_entries(&directory));
    let expected = (fs().realpath(&file), fs().file_exists(&file), fs().stat(&file), fs().get_accessible_entries(&directory));
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(actual, expected);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn file_exists_accepts_special_nodes_omitted_from_directory_listings() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let device = format!("{tmp}/device");
    mklink("/dev/null", &device, false);
    let cache = ResolverFsCache::new();
    let entries = cache.get_accessible_entries(&tmp);
    assert!(entries.files.is_empty());
    assert!(entries.directories.is_empty());
    for path in [&device, "/dev/null"] {
        assert!(cache.file_exists(path));
        assert!(!cache.directory_exists(path));
        assert!(cache.stat(path).unwrap().mode().intersects(FileMode::CharDevice));
        assert_eq!(cache.realpath(path), fs().realpath(path));
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}

fn prime_directory_name_snapshot(cache: &ResolverFsCache, directory: &str) -> Arc<CachedPath> {
    assert!(cache.directory_exists(directory));
    let parent = cache.intern(directory);
    for index in 0..DIRECTORY_SCAN_AFTER_MISSES {
        assert!(!cache.file_exists(&format!("{directory}/missing-{index}.ts")));
        assert!(parent.data().names.get().is_none());
    }
    assert!(!cache.file_exists(&format!("{directory}/snapshot-trigger.ts")));
    assert!(parent.data().names.get().is_some());
    parent
}

#[test]
fn directory_names_require_distinct_misses_and_a_known_parent() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let listing_eligible = DirectoryNames::read(&tmp).is_some();
    let cache = ResolverFsCache::new();
    let parent = cache.intern(&tmp);
    let first = format!("{tmp}/missing-0.ts");
    for _ in 0..32 {
        assert!(!cache.file_exists(&first));
    }
    assert_eq!(parent.missing_children.load(Ordering::SeqCst), 1);
    for index in 1..=DIRECTORY_SCAN_AFTER_MISSES {
        assert!(!cache.file_exists(&format!("{tmp}/missing-{index}.ts")));
    }
    assert_eq!(parent.missing_children.load(Ordering::SeqCst), DIRECTORY_SCAN_AFTER_MISSES);
    assert!(parent.data().names.get().is_none());

    assert!(cache.directory_exists(&tmp));
    // A repeated negative does not create a snapshot, even after the parent becomes known.
    assert!(!cache.file_exists(&first));
    assert!(parent.data().names.get().is_none());
    assert!(!cache.file_exists(&format!("{tmp}/snapshot-trigger.ts")));
    assert_eq!(parent.data().names.get().unwrap().is_some(), listing_eligible);

    let added = format!("{tmp}/added-after-snapshot.ts");
    std::fs::write(&added, "").unwrap();
    assert!(fs().file_exists(&added));
    // Unsupported filesystems retain live metadata probes instead of inferring negatives.
    assert_eq!(cache.file_exists(&added), !listing_eligible);
    assert!(ResolverFsCache::new().file_exists(&added));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(unix)]
#[test]
fn directory_names_do_not_infer_positive_metadata_or_hide_symlinks() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let removed = format!("{tmp}/removed.ts");
    let changed = format!("{tmp}/changed");
    let device = format!("{tmp}/device");
    let dangling = format!("{tmp}/dangling.ts");
    std::fs::write(&removed, "").unwrap();
    std::fs::create_dir(&changed).unwrap();
    mklink("/dev/null", &device, false);
    mklink("absent-target.ts", &dangling, false);
    let listing_eligible = DirectoryNames::read(&tmp).is_some();
    let cache = ResolverFsCache::new();
    let parent = prime_directory_name_snapshot(&cache, &tmp);
    assert_eq!(parent.data().names.get().unwrap().is_some(), listing_eligible);

    std::fs::remove_file(&removed).unwrap();
    std::fs::remove_dir(&changed).unwrap();
    std::fs::write(&changed, "").unwrap();
    assert!(!cache.file_exists(&removed));
    assert!(cache.file_exists(&changed));
    assert!(!cache.directory_exists(&changed));
    assert!(cache.file_exists(&device));
    assert!(!cache.file_exists(&dangling));
    for path in [&device, &dangling] {
        assert!(cache.intern(path).link_metadata().unwrap().intersects(FileMode::Symlink));
    }
    assert!(cache.stat(&device).unwrap().mode.intersects(FileMode::CharDevice));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn directory_name_hash_and_lookup_fold_ascii_across_chunks() {
    let lengths = [1, 7, 8, 9, 15, 16, 17, 31, 32, 33];
    let names: Vec<String> = lengths.into_iter().map(|length| (0..length).map(|index| char::from(b'a' + index % 26)).collect()).collect();
    let mut table = HashTable::new();
    for name in &names {
        table.insert_unique(name_hash(name), name.clone().into_boxed_str(), |name| name_hash(name));
    }
    let snapshot = DirectoryNames(table);
    for name in names {
        let upper = name.to_ascii_uppercase();
        let mixed: String = name.chars().enumerate().map(|(index, ch)| if index % 2 == 0 { ch.to_ascii_uppercase() } else { ch }).collect();
        assert_eq!(name_hash(&name), name_hash(&upper));
        assert_eq!(name_hash(&name), name_hash(&mixed));
        assert!(!snapshot.excludes(&name));
        assert!(!snapshot.excludes(&upper));
        assert!(!snapshot.excludes(&mixed));
    }
    assert!(snapshot.excludes("definitely-absent.ts"));
    assert!(!snapshot.excludes(""));
    assert!(!snapshot.excludes("."));
    assert!(!snapshot.excludes(".."));
    assert!(!snapshot.excludes("\u{e9}.ts"));
}

#[test]
fn non_ascii_directory_entry_disables_negative_inference() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    std::fs::write(format!("{tmp}/\u{e9}.ts"), "").unwrap();
    let cache = ResolverFsCache::new();
    let parent = prime_directory_name_snapshot(&cache, &tmp);
    assert!(matches!(parent.data().names.get(), Some(None)));
    let added = format!("{tmp}/added-after-scan.ts");
    std::fs::write(&added, "").unwrap();
    assert!(cache.file_exists(&added));
    std::fs::remove_dir_all(&tmp).unwrap();
}

// macOS rejects this invalid-byte fixture before directory enumeration can observe it.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_directory_entry_disables_negative_inference() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let tmp = tspath::normalize_slashes(&temp_dir());
    let non_utf8 = std::path::Path::new(&tmp).join(OsString::from_vec(vec![b'x', 0xff]));
    std::fs::write(non_utf8, "").unwrap();
    let cache = ResolverFsCache::new();
    let parent = prime_directory_name_snapshot(&cache, &tmp);
    assert!(matches!(parent.data().names.get(), Some(None)));
    let added = format!("{tmp}/added-after-scan.ts");
    std::fs::write(&added, "").unwrap();
    assert!(cache.file_exists(&added));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn procfs_unlisted_thread_directory_is_not_inferred_missing() {
    std::thread::spawn(|| {
        // procfs exposes /proc/<tid> for every thread, but lists only process leaders.
        let thread_path = std::fs::read_link("/proc/thread-self").unwrap();
        let thread_id = thread_path.file_name().unwrap().to_str().unwrap();
        assert_ne!(thread_id, std::process::id().to_string());
        let hidden = format!("/proc/{thread_id}");
        assert!(fs().directory_exists(&hidden));
        assert!(std::fs::read_dir("/proc").unwrap().all(|entry| entry.unwrap().file_name() != thread_id));

        let cache = ResolverFsCache::new();
        let parent = prime_directory_name_snapshot(&cache, "/proc");
        assert!(matches!(parent.data().names.get(), Some(None)));
        assert!(cache.directory_exists(&hidden));
        assert_eq!(cache.realpath(&hidden), fs().realpath(&hidden));
    }).join().unwrap();
}

#[cfg(unix)]
#[test]
fn directory_names_do_not_reinterpret_raw_realpath_backslashes() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let lexical_parent = format!("{tmp}/directory");
    std::fs::create_dir(&lexical_parent).unwrap();
    let raw = format!("{tmp}/directory\\file.ts");
    std::fs::write(&raw, "").unwrap();
    let listing_eligible = DirectoryNames::read(&lexical_parent).is_some();
    let cache = ResolverFsCache::new();
    let parent = prime_directory_name_snapshot(&cache, &lexical_parent);
    assert_eq!(parent.data().names.get().unwrap().is_some(), listing_eligible);
    let raw_entry = cache.intern(&raw);
    assert!(!raw_entry.normalized);
    assert!(raw_entry.link_metadata().is_some());
    assert_eq!(cache.realpath(&raw), fs().realpath(&raw));
    assert!(!cache.file_exists(&raw));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn followed_metadata_keeps_kernel_link_limit_separate_from_realpath() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    std::fs::write(format!("{tmp}/file-target"), "").unwrap();
    std::fs::create_dir(format!("{tmp}/directory-target")).unwrap();
    for (kind, is_directory) in [("file", false), ("directory", true)] {
        // The Go walker permits 255 links; a Darwin stat lookup rejects this finite chain.
        let mut target = format!("{kind}-target");
        for index in (0..64).rev() {
            let link = format!("{kind}-{index}");
            mklink(&target, &format!("{tmp}/{link}"), is_directory);
            target = link;
        }
        let path = format!("{tmp}/{kind}-0");
        assert!(!fs().file_exists(&path));
        assert!(!fs().directory_exists(&path));
        let resolved = fs().realpath(&format!("{tmp}/{kind}-target"));
        assert_eq!(fs().realpath(&path), resolved);

        for realpath_first in [false, true] {
            let cache = ResolverFsCache::new();
            if realpath_first {
                assert_eq!(cache.realpath(&path), resolved);
            }
            assert!(!cache.file_exists(&path));
            assert!(!cache.directory_exists(&path));
            assert_eq!(cache.realpath(&path), resolved);
        }
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}
