use super::*;

fn record_read() {
    physical("api.read", "/physical/package.json", || Some("{}"), |result| if result.is_some() { "some" } else { "none" });
    cache("package_json", "miss");
}

#[test]
fn nested_scopes_and_unwinding_restore_attribution() {
    let collector = Arc::new(Collector::default());
    let _root = Scope::enter(Context::root(Arc::clone(&collector)));
    logical(
        "outer",
        "read_file",
        "/logical/package.json",
        || {
            record_read();
            let panic = std::panic::catch_unwind(|| {
                logical(
                    "inner",
                    "realpath",
                    "/inner",
                    || {
                        physical("api.read_link", "/link", || (), |_| "ok");
                        panic!("test unwind");
                    },
                    |_| "ok",
                );
            });
            assert!(panic.is_err());
            record_read();
        },
        |_| "ok",
    );
    physical("api.metadata", "/outside", || (), |_| "ok");

    let rows = collector.json();
    let rows = rows["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter()
            .any(|row| row["layer"] == "cache" && row["site"] == "outer" && row["path"] == "/logical/package.json" && row["count"] == 2)
    );
    assert!(rows.iter().any(|row| row["layer"] == "physical"
        && row["site"] == "outer"
        && row["logical_operation"] == "read_file"
        && row["path"] == "/physical/package.json"
        && row["count"] == 2));
    assert!(rows.iter().any(|row| row["site"] == "inner" && row["operation"] == "api.read_link"));
    assert!(rows.iter().any(|row| row["site"] == "nonresolver" && row["path"] == "/outside"));
}

#[test]
fn concurrent_threads_aggregate_without_leaking_scopes() {
    let collector = Arc::new(Collector::default());
    std::thread::scope(|scope| {
        for index in 0..8 {
            let collector = Arc::clone(&collector);
            scope.spawn(move || {
                let _root = Scope::enter(Context::root(collector));
                let site = if index % 2 == 0 { "even" } else { "odd" };
                for _ in 0..10 {
                    logical(site, "read_file", "/logical/package.json", record_read, |_| "ok");
                }
                physical("api.metadata", "/outside", || (), |_| "ok");
            });
        }
    });
    let rows = collector.json();
    let rows = rows["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 7);
    for row in rows {
        if row["site"] == "nonresolver" {
            assert_eq!(row["count"], 8);
        } else {
            assert_eq!(row["count"], 40);
            assert_eq!(row["logical_operation"], "read_file");
        }
    }
}

#[test]
fn os_operations_keep_the_logical_scope_through_common() {
    let temporary = std::env::temp_dir().join(format!("tsrs_fsstats_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temporary);
    std::fs::create_dir(&temporary).unwrap();
    std::fs::write(temporary.join("source.ts"), "export {};").unwrap();
    let directory = tsrs_core::tspath::normalize_slashes(&temporary.to_string_lossy());
    let file = format!("{directory}/source.ts");
    let missing = format!("{directory}/missing.ts");
    let collector = Arc::new(Collector::default());
    let _root = Scope::enter(Context::root(Arc::clone(&collector)));

    assert!(logical("integration", "read_file", &file, || crate::osvfs::fs().read_file(&file), |_| "some").is_some());
    assert!(logical("integration", "read_file", &missing, || crate::osvfs::fs().read_file(&missing), |_| "none").is_none());
    let entries = logical(
        "integration",
        "get_accessible_entries",
        &directory,
        || crate::osvfs::fs().get_accessible_entries(&directory),
        |_| "entries",
    );
    assert_eq!(entries.files, ["source.ts"]);

    let json = collector.json();
    let rows = json["rows"].as_array().unwrap();
    let physical: Vec<_> = rows.iter().filter(|row| row["layer"] == "physical").collect();
    assert!(physical.iter().all(|row| row["site"] == "integration"));
    assert!(physical.iter().any(|row| row["operation"] == "api.read" && row["result"] == "error.not_found"));
    assert!(physical.iter().any(|row| row["operation"] == "api.read_dir" && row["logical_operation"] == "get_accessible_entries"));
    assert!(physical.iter().any(|row| row["operation"] == "api.ReadDir.next" && row["result"] == "end"));
    assert!(physical.iter().any(|row| row["operation"] == "api.DirEntry.file_type" && row["result"] == "file" && row["path"] == file));
    std::fs::remove_dir_all(temporary).unwrap();
}
