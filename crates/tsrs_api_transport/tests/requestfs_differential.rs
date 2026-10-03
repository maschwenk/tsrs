// Differential test of the request filesystem port against the pinned Go implementation: the same
// scenarios (tests/fixtures/requestfs_scenarios.json) were run through Go's NewForUpdate over a vfstest
// host (tests/fixtures/requestfs_gen.go.txt); every step's file-change summary and every query's
// FileExists/DirectoryExists/ReadFile/Realpath/GetAccessibleEntries/Stat must match.

use std::sync::Arc;

use serde_json::{json, Value};
use tsrs_api_transport::requestfs::{new_for_update, AnyFs, RequestFileSystemParams};
use tsrs_project::{FileChangeSummary, FsRef};
use tsrs_vfs::vfstest::{self, MapFile};
use tsrs_vfs::FS;

fn sorted_uris(set: &tsrs_core::collections::Set<tsrs_lsproto::DocumentUri>) -> Value {
    let mut v: Vec<String> = set.keys().iter().map(|u| u.0.clone()).collect();
    v.sort();
    if v.is_empty() {
        Value::Null
    } else {
        json!(v)
    }
}

fn query(fs: &dyn FS, q: &str) -> Value {
    let e = fs.get_accessible_entries(q);
    let mut links: Vec<String> = e.symlinks.map(|s| s.into_iter().collect()).unwrap_or_default();
    links.sort();
    let none_if_empty = |v: Vec<String>| if v.is_empty() { Value::Null } else { json!(v) };
    json!({
        "path": q,
        "fileExists": fs.file_exists(q),
        "directoryExists": fs.directory_exists(q),
        "readFile": fs.read_file(q),
        "realpath": fs.realpath(q),
        "entries": {"files": none_if_empty(e.files), "directories": none_if_empty(e.directories), "symlinks": none_if_empty(links)},
        "stat": fs.stat(q).map(|i| json!({"name": i.name, "size": i.size, "isDir": i.is_dir(), "mode": i.mode.bits()})),
    })
}

/// Go marshals nil and empty slices differently (null vs []); compare them as equal.
fn normalize(v: &Value) -> Value {
    match v {
        Value::Array(a) if a.is_empty() => Value::Null,
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), normalize(v))).collect()),
        Value::String(s) if s.is_empty() => Value::String(String::new()),
        other => other.clone(),
    }
}

#[test]
fn request_filesystem_matches_pinned_go() {
    let scenarios: Vec<Value> = serde_json::from_str(include_str!("fixtures/requestfs_scenarios.json")).unwrap();
    let expected: Value = serde_json::from_str(include_str!("fixtures/requestfs_go.json")).unwrap();
    let mut compared = 0;
    for sc in &scenarios {
        let name = sc["name"].as_str().unwrap();
        let case_sensitive = sc["caseSensitive"].as_bool().unwrap();
        let cwd = sc["cwd"].as_str().unwrap();
        let mut host: Vec<(String, MapFile)> = Vec::new();
        for (k, v) in sc["host"]["files"].as_object().unwrap() {
            host.push((k.clone(), MapFile::from(v.as_str().unwrap())));
        }
        if let Some(links) = sc["host"]["symlinks"].as_object() {
            for (k, v) in links {
                host.push((k.clone(), vfstest::symlink(v.as_str().unwrap())));
            }
        }
        let base: Arc<dyn FS> = Arc::new(vfstest::from_map(host, case_sensitive));
        let mut fs = AnyFs::Other(FsRef::Host(base));
        let queries: Vec<&str> = sc["queries"].as_array().unwrap().iter().map(|q| q.as_str().unwrap()).collect();
        let go_steps = expected[name].as_array().unwrap_or_else(|| panic!("no Go result for {name}"));
        for (i, step) in sc["steps"].as_array().unwrap().iter().enumerate() {
            let params: RequestFileSystemParams = serde_json::from_value(step.clone()).unwrap();
            let mut changes = FileChangeSummary::default();
            let mut out = json!({"error": "", "changed": null, "created": null, "deleted": null, "includesOutside": false});
            match new_for_update(Some(&params), &fs, cwd, &mut changes) {
                Ok(next) => {
                    fs = next;
                    out["changed"] = sorted_uris(&changes.changed);
                    out["created"] = sorted_uris(&changes.created);
                    out["deleted"] = sorted_uris(&changes.deleted);
                    out["includesOutside"] = json!(changes.includes_watch_change_outside_node_modules);
                }
                Err(e) => out["error"] = json!(e),
            }
            out["queries"] = json!(queries.iter().map(|q| query(fs.fs(), q)).collect::<Vec<_>>());
            let go = &go_steps[i];
            for key in ["error", "changed", "created", "deleted", "includesOutside"] {
                assert_eq!(normalize(&out[key]), normalize(&go[key]), "{name} step {i}: {key}");
            }
            for (q, (ours, theirs)) in queries.iter().zip(out["queries"].as_array().unwrap().iter().zip(go["queries"].as_array().unwrap())) {
                assert_eq!(normalize(ours), normalize(theirs), "{name} step {i}: query {q}");
                compared += 1;
            }
        }
    }
    assert!(compared >= 100, "only {compared} queries compared");
    eprintln!("request filesystem: {compared} query results match pinned Go");
}

/// The same layering over the real OS filesystem (temp directory with a real symlink).
#[test]
fn request_filesystem_layers_over_real_os_files() {
    let dir = std::env::temp_dir().join(format!("tsrs-requestfs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/lib")).unwrap();
    std::fs::write(dir.join("src/a.ts"), "disk a").unwrap();
    std::fs::write(dir.join("src/b.ts"), "disk b").unwrap();
    std::fs::write(dir.join("src/lib/l.ts"), "disk l").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("src/lib"), dir.join("src/linked")).unwrap();
    let root = tsrs_core::tspath::normalize_slashes(dir.to_str().unwrap());
    let p = |s: &str| format!("{root}/{s}");

    let host: Arc<dyn FS> = Arc::new(tsrs_vfs::osvfs::fs());
    let base = AnyFs::Other(FsRef::Host(host));
    let params: RequestFileSystemParams = serde_json::from_value(json!({
        "kind": "layer",
        "files": {"src/a.ts": "request a", "src/new.ts": "new"},
        "removedPaths": ["src/b.ts"],
    }))
    .unwrap();
    let mut changes = FileChangeSummary::default();
    let layered = new_for_update(Some(&params), &base, &root, &mut changes).unwrap();
    let fs = layered.fs();
    assert_eq!(fs.read_file(&p("src/a.ts")).as_deref(), Some("request a"));
    assert_eq!(fs.read_file(&p("src/b.ts")), None, "removed path is a negative lookup");
    assert!(!fs.file_exists(&p("src/b.ts")));
    assert_eq!(fs.read_file(&p("src/lib/l.ts")).as_deref(), Some("disk l"), "falls back to disk");
    let mut entries = fs.get_accessible_entries(&p("src")).files;
    entries.sort();
    assert_eq!(entries, ["a.ts", "new.ts"]);
    #[cfg(unix)]
    {
        assert_eq!(fs.read_file(&p("src/linked/l.ts")).as_deref(), Some("disk l"));
        assert_eq!(fs.realpath(&p("src/linked/l.ts")), p("src/lib/l.ts"));
    }
    let mut changed: Vec<String> = changes.changed.keys().iter().map(|u| u.0.clone()).collect();
    changed.sort();
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].ends_with("/src/a.ts"));
    assert_eq!(changes.deleted.len(), 1);
    assert_eq!(changes.created.len(), 1);
    // Layers write through to the host; a full filesystem rejects mutation.
    fs.write_file(&p("src/out.js"), "emitted").unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("src/out.js")).unwrap(), "emitted");
    let full: RequestFileSystemParams = serde_json::from_value(json!({"kind": "full", "files": {"src/a.ts": "x"}})).unwrap();
    let full = new_for_update(Some(&full), &layered, &root, &mut FileChangeSummary::default()).unwrap();
    assert_eq!(full.fs().read_file(&p("src/lib/l.ts")), None, "full replaces the host");
    assert_eq!(full.fs().write_file(&p("src/out2.js"), "x"), Err("invalid argument".to_string()));
    assert!(!dir.join("src/out2.js").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
