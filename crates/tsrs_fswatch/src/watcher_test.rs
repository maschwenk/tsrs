// watcher_test.go (the tests that run on macOS / Linux with the ported backends). Each per-backend test runs
// against every available watcher (FSEvents + kqueue on macOS, inotify on Linux).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::debounce::{debounce, defaultMaxWaitTime};
use crate::event::{Event, EventKind};
use crate::pathcompare::pathComparer;
use crate::pathkey::PathComparer;
use crate::testutil_test::*;
use crate::watcher::{all_watchers, default, dirWatch, is_in_directory_or_self, join_path_suffix, physical_dir_for, rebase_path, Error, ErrUnavailable, ErrWatchTerminated, WatchCallback};

const maxWaitTime: Duration = defaultMaxWaitTime;

fn write(path: &str, data: &str) -> TestResult {
    std::fs::write(path, data).map_err(|e| e.to_string())
}

fn mkdir(path: &str) -> TestResult {
    std::fs::create_dir(path).map_err(|e| e.to_string())
}

// watcher_test.go:578
#[test]
fn path_comparer() {
    let tests: &[(&str, &str, &str, bool, bool)] = &[
        ("/root", "/root", "", true, true),
        ("/root", "/root/file.ts", "/file.ts", true, true),
        ("/root", "/ROOT", "", false, true),
        ("/root", "/ROOT/File.ts", "/File.ts", false, true),
        ("/root", "/ROOT/Nested/File.ts", "/Nested/File.ts", false, true),
        ("/root", "/ROOT2/File.ts", "", false, false),
        ("/root", "/roo", "", false, false),
        ("/root/sub", "/ROOT", "", false, false),
        ("/root", "/other/File.ts", "", false, false),
        ("/root", "/ROOTish/File.ts", "", false, false),
        ("/root/sub", "/ROOT/SUB", "", false, true),
        ("/root/sub", "/ROOT/su", "", false, false),
        ("/root/[", "/ROOT/{/File.ts", "", false, false),
        ("/root/@", "/ROOT/`/File.ts", "", false, false),
        ("/", "/File.ts", "File.ts", true, true),
        ("/", "/", "", true, true),
        ("", "", "", false, false),
        ("", "/File.ts", "", false, false),
        ("/caf\u{00e9}", "/CAF\u{00c9}/File.ts", "/File.ts", false, true),
        ("/s", "/\u{017f}/File.ts", "/File.ts", false, true),
        ("/\u{017f}", "/S/File.ts", "/File.ts", false, true),
        ("/s/sub", "/\u{017f}/SUB/File.ts", "/File.ts", false, true),
        ("/\u{017f}/sub", "/S/SUB/File.ts", "/File.ts", false, true),
        ("/s", "/\u{017f}oo/File.ts", "", false, false),
        ("/k", "/\u{212a}/File.ts", "/File.ts", false, true),
        ("/\u{03c3}", "/\u{03c2}/File.ts", "/File.ts", false, true),
        ("/\u{00e9}", "/\u{00c8}/File.ts", "", false, false),
        ("/\u{00df}", "/SS/File.ts", "/File.ts", false, crate::canonicalize::nativePathFolding),
        ("/root/s", "/ROOT/\u{017f}/File.ts", "/File.ts", false, true),
        ("/root/\u{017f}", "/ROOT/S", "", false, true),
    ];
    let mut failures = Vec::new();
    for &(root, path, suffix, exact, ignore_case_want) in tests {
        for ignore_case in [false, true] {
            let comparer = pathComparer { ignore_case };
            let want = if ignore_case { ignore_case_want } else { exact };
            let got = comparer.suffix(root, path);
            if got.is_some() != want || got.as_deref().is_some_and(|s| s != suffix) {
                failures.push(format!("suffix({:?}, {:?}), ignoreCase={}: got {:?}, want ({:?}, {})", root, path, ignore_case, got, suffix, want));
            }
            if comparer.contains(root, path) != want {
                failures.push(format!("contains({:?}, {:?}), ignoreCase={}: want {}", root, path, ignore_case, want));
            }
            for to in ["/display", "/"] {
                let rebased = comparer.rebase(path, root, to);
                if rebased.is_some() != want || rebased.as_ref().is_some_and(|r| *r != join_path_suffix(to, suffix)) {
                    failures.push(format!("rebase({:?}, {:?}, {:?}), ignoreCase={}: got {:?}", path, root, to, ignore_case, rebased));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// watcher_test.go:664
#[test]
fn file_callback_case_sensitivity() {
    for ignore_case in [false, true] {
        let got: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
        let dw = dirWatch::new("/root".to_string(), "/root".to_string(), &debounce::new(), pathComparer { ignore_case }, None, true);
        let sink = got.clone();
        let cb: WatchCallback = Arc::new(move |events: Vec<Event>, _: Option<Error>| sink.lock().unwrap().extend(events));
        dw.add_callback("/root", "/root", false, cb, None, "/root/file.ts");
        dw.events.update("/root/FILE.ts");
        dw.events.update("/root/other.ts");
        dw.trigger_callbacks_for_test();
        let got = got.lock().unwrap();
        if ignore_case {
            assert!(got.len() == 1 && got[0].path == "/root/file.ts", "case-insensitive callback: got {:?}", got);
        } else {
            assert!(got.is_empty(), "case-sensitive callback: got {:?}", got);
        }
        dw.destroy_debounce_for_test();
    }
}

// watcher_test.go:686
#[test]
fn path_comparer_exact_keys() {
    let c = PathComparer::default();
    for path in ["/A/File.ts", "/straße/İ.ts", "/cafe\u{0301}.ts"] {
        assert_eq!(c.key(path), path);
    }
    assert!(c.rebase("/A/file.ts", "/a", "/target").is_none(), "zero comparer must use exact matching");
    assert_eq!(c.rebase("/a/file.ts", "/a", "/target").as_deref(), Some("/target/file.ts"));
}

// watcher_test.go:702
#[test]
fn rebase_path_cases() {
    let cases = [
        ("/from", "/from", "/to", "/to"),
        ("/from/child", "/from", "/to", "/to/child"),
        ("/from-sibling/child", "/from", "/to", "/from-sibling/child"),
        ("/child", "/", "/to", "/to/child"),
        ("/from/child", "/from", "/", "/child"),
    ];
    for (path, from, to, want) in cases {
        assert_eq!(rebase_path(path, from, to), want, "rebasePath({:?}, {:?}, {:?})", path, from, to);
    }
}

// watcher_test.go:764
#[test]
fn physical_dir_for_resolves_symlink_ancestor() {
    let mut t = TestT::new(1);
    let root = t.temp_dir();
    let target = format!("{}/target", root);
    std::fs::create_dir_all(format!("{}/nested", target)).unwrap();
    let link = format!("{}/link", root);
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let dir = format!("{}/nested", link);
    let want = physical_dir_for(&format!("{}/nested", target));
    assert_eq!(physical_dir_for(&dir), want);
}

// watcher_test.go:782
#[test]
fn is_in_directory_or_self_cases() {
    let cases = [
        ("/parent", "/parent", true),
        ("/parent", "/parent/child", true),
        ("/parent", "/parent/child/nested", true),
        ("/parent", "/parent-sibling", false),
        ("/", "/", true),
        ("/", "/child", true),
        ("", "/parent/child", false),
    ];
    for (dir, path, want) in cases {
        assert_eq!(is_in_directory_or_self(dir, path), want, "isInDirectoryOrSelf({:?}, {:?})", dir, path);
    }
}

// ----- files -------------------------------------------------------------

// watcher_test.go:853
#[test]
fn watch_file_create() {
    run_for_each_watcher("TestWatchFileCreate", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, _) = subscribe_for(t, &dir, w)?;
        let f = sub_path(&dir);
        write(&f, "hello")?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &f)])?;
        Ok(())
    });
}

// watcher_test.go:867
#[test]
fn watch_file_update() {
    run_for_each_watcher("TestWatchFileUpdate", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, _) = subscribe_for(t, &dir, w)?;
        let f = sub_path(&dir);
        write(&f, "v1")?;
        let _ = r.wait_for_event(r.deadline(), |_| true); // consume the create event
        write(&f, "v2-longer")?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &f)])?;
        Ok(())
    });
}

// watcher_test.go:887
#[test]
fn watch_file_rename() {
    run_for_each_watcher("TestWatchFileRename", |t, w| {
        let dir = new_tmp_dir(t);
        let f1 = sub_path(&dir);
        let f2 = sub_path(&dir);
        write(&f1, "x")?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        std::fs::rename(&f1, &f2).map_err(|e| e.to_string())?;
        expect_event_set(&r, vec![want(EventKind::Delete, &f1), want(EventKind::Update, &f2)])?;
        Ok(())
    });
}

// watcher_test.go:928
#[test]
fn watch_file_delete() {
    run_for_each_watcher("TestWatchFileDelete", |t, w| {
        let dir = new_tmp_dir(t);
        let f = sub_path(&dir);
        write(&f, "x")?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        std::fs::remove_file(&f).map_err(|e| e.to_string())?;
        expect_event_sequence(&r, vec![want(EventKind::Delete, &f)])?;
        Ok(())
    });
}

// ----- directories -------------------------------------------------------

// watcher_test.go:946
#[test]
fn subscribe_dir_create() {
    run_for_each_watcher("TestSubscribeDirCreate", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, _) = subscribe_for(t, &dir, w)?;
        let f = sub_path(&dir);
        mkdir(&f)?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &f)])?;
        Ok(())
    });
}

// watcher_test.go:965
#[test]
fn subscribe_non_ascii_path() {
    run_for_each_watcher("TestSubscribeNonASCIIPath", |t, w| {
        let parent = new_tmp_dir(t);
        // "café" + "résumé"; both precomposed NFC.
        let dir = format!("{}/caf\u{00e9}-dir", parent);
        mkdir(&dir)?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        let child = format!("{}/r\u{00e9}sum\u{00e9}.txt", dir);
        write(&child, "hi")?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &child)])?;
        Ok(())
    });
}

// watcher_test.go:1004
#[test]
fn subscribe_dir_delete() {
    run_for_each_watcher("TestSubscribeDirDelete", |t, w| {
        let dir = new_tmp_dir(t);
        let f = sub_path(&dir);
        mkdir(&f)?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        std::fs::remove_dir_all(&f).map_err(|e| e.to_string())?;
        expect_event_sequence(&r, vec![want(EventKind::Delete, &f)])?;
        Ok(())
    });
}

// watcher_test.go:1020
#[test]
fn subscribe_watched_dir_deleted() {
    run_for_each_watcher("TestSubscribeWatchedDirDeleted", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, _) = subscribe_for(t, &dir, w)?;
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        expect_event_sequence(&r, vec![want(EventKind::Delete, &dir)])?;

        // Give the backend a moment to surface ErrWatchTerminated alongside
        // the delete; some backends batch the error into a later debounce
        // tick than the event itself.
        let deadline = std::time::Instant::now() + r.deadline();
        while std::time::Instant::now() < deadline && r.err_count() == 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        let errs = r.take_errs();
        if !errs.iter().any(|e| e.is(ErrWatchTerminated)) {
            fatalf!("expected ErrWatchTerminated after watched dir delete, got errs={:?}", errs);
        }

        // Re-create; should not emit events for a now-stale watch.
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let extra = r.drain_quiet(Duration::from_millis(200));
        if !extra.is_empty() {
            fatalf!("expected no follow-up events, got {:?}", extra);
        }
        Ok(())
    });
}

// ----- sub-files ---------------------------------------------------------

// watcher_test.go:1071
#[test]
fn subscribe_subfile_create() {
    run_for_each_watcher("TestSubscribeSubfileCreate", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, _) = subscribe_for(t, &dir, w)?;
        let sub = sub_path(&dir);
        mkdir(&sub)?;
        expect_contains(&r, EventKind::Update, &sub)?;
        std::thread::sleep(Duration::from_millis(100));
        let f = sub_path(&sub);
        write(&f, "hi")?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &f)])?;
        Ok(())
    });
}

// watcher_test.go:1142
#[test]
fn subscribe_subfile_delete() {
    run_for_each_watcher("TestSubscribeSubfileDelete", |t, w| {
        let dir = new_tmp_dir(t);
        let sub = sub_path(&dir);
        mkdir(&sub)?;
        let f = sub_path(&sub);
        write(&f, "x")?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        std::fs::remove_file(&f).map_err(|e| e.to_string())?;
        let want_ = vec![want(EventKind::Delete, &f)];
        let got = r.wait_for_all(r.deadline(), &want_);
        assert_event_sequence(&filter_events_for_paths(&got, &[&f]), &want_)?;
        Ok(())
    });
}

// watcher_test.go:1187
#[test]
fn subscribe_subdir_delete_with_files() {
    run_for_each_watcher("TestSubscribeSubdirDeleteWithFiles", |t, w| {
        let dir = new_tmp_dir(t);
        let sub_dir = sub_path(&dir);
        mkdir(&sub_dir)?;
        let child = sub_path(&sub_dir);
        write(&child, "x")?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        std::fs::remove_dir_all(&sub_dir).map_err(|e| e.to_string())?;
        expect_event_set(&r, vec![want(EventKind::Delete, &sub_dir), want(EventKind::Delete, &child)])?;
        Ok(())
    });
}

// watcher_test.go:1438
#[test]
fn subscribe_multiple_same_dir() {
    run_for_each_watcher("TestSubscribeMultipleSameDir", |t, w| {
        let dir = new_tmp_dir(t);
        std::thread::sleep(Duration::from_millis(50));
        let r1 = new_recorder(t, Some(w));
        let s1 = w.watch_directory(&dir, r1.callback(), vec![]).map_err(|e| e.to_string())?;
        let r2 = new_recorder(t, Some(w));
        let s2 = w.watch_directory(&dir, r2.callback(), vec![]).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(100));
        let f = sub_path(&dir);
        write(&f, "hi")?;
        let res = assert_event_sequence(&r1.next(r1.deadline()), &[want(EventKind::Update, &f)]).and_then(|_| assert_event_sequence(&r2.next(r2.deadline()), &[want(EventKind::Update, &f)]));
        let _ = s1.close();
        let _ = s2.close();
        res
    });
}

// watcher_test.go:1810
#[test]
fn subscribe_missing_dir_error() {
    run_for_each_watcher("TestSubscribeMissingDirError", |t, w| {
        let bogus = format!("{}/definitely-not-here", new_tmp_dir(t));
        if w.watch_directory(&bogus, noop_callback(), vec![]).is_ok() {
            fatalf!("expected error subscribing to non-existent dir");
        }
        Ok(())
    });
}

// watcher_test.go:1821
#[test]
fn subscribe_not_a_dir_error() {
    run_for_each_watcher("TestSubscribeNotADirError", |t, w| {
        let dir = new_tmp_dir(t);
        let f = sub_path(&dir);
        write(&f, "x")?;
        if w.watch_directory(&f, noop_callback(), vec![]).is_ok() {
            fatalf!("expected error subscribing to a file");
        }
        Ok(())
    });
}

// watcher_test.go:1845
#[test]
fn subscribe_rejects_relative_path() {
    run_for_each_watcher("TestSubscribeRejectsRelativePath", |_, w| {
        if w.watch_directory("relative/path", noop_callback(), vec![]).is_ok() {
            fatalf!("WatchDirectory with relative path should return an error");
        }
        if w.watch_file("relative/path/file.txt", noop_callback()).is_ok() {
            fatalf!("WatchFile with relative path should return an error");
        }
        Ok(())
    });
}

// watcher_test.go:1861
#[test]
fn subscribe_unsubscribe_idempotent() {
    run_for_each_watcher("TestSubscribeUnsubscribeIdempotent", |t, w| {
        let dir = new_tmp_dir(t);
        let r = new_recorder(t, Some(w));
        let sub = w.watch_directory(&dir, r.callback(), vec![]).map_err(|e| e.to_string())?;
        sub.close().map_err(|e| e.to_string())?;
        if let Err(e) = sub.close() {
            fatalf!("second Close should be a no-op, got {}", e);
        }
        Ok(())
    });
}

// watcher_test.go:1888
#[test]
fn subscribe_close_then_re_subscribe() {
    run_for_each_watcher("TestSubscribeCloseThenReSubscribe", |t, w| {
        let dir = new_tmp_dir(t);
        let r1 = new_recorder(t, Some(w));
        let s1 = w.watch_directory(&dir, r1.callback(), vec![]).map_err(|e| e.to_string())?;
        s1.close().map_err(|e| e.to_string())?;

        let r2 = new_recorder(t, Some(w));
        let s2 = w.watch_directory(&dir, r2.callback(), vec![]).map_err(|e| format!("re-WatchDirectory after Close: {}", e))?;
        std::thread::sleep(settle_sleep(w));

        let f = sub_path(&dir);
        write(&f, "hi")?;
        let res = expect_event_sequence(&r2, vec![want(EventKind::Update, &f)]);
        let _ = s2.close();
        res?;

        let stale = r1.drain_quiet(Duration::from_millis(50));
        if !stale.is_empty() {
            fatalf!("closed watch saw events: {:?}", to_want_events(&stale));
        }
        Ok(())
    });
}

// watcher_test.go:2163
#[test]
fn subscribe_no_events_after_unsubscribe() {
    run_for_each_watcher("TestSubscribeNoEventsAfterUnsubscribe", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, sub) = subscribe_for(t, &dir, w)?;
        sub.close().map_err(|e| e.to_string())?;
        let f = sub_path(&dir);
        write(&f, "x")?;
        let got = r.drain_quiet(Duration::from_millis(500));
        if !got.is_empty() {
            fatalf!("expected no events after closeWatch, got {:?}", to_want_events(&got));
        }
        Ok(())
    });
}

// watcher_test.go:2344
#[test]
fn default_backend_matches_platform() {
    let d = default();
    let want_name = if cfg!(target_os = "linux") {
        "inotify"
    } else if cfg!(target_os = "macos") {
        "fsevents"
    } else {
        return;
    };
    assert!(d.available());
    assert_eq!(d.name(), want_name);
}

// watcher_test.go:2374
#[test]
fn unavailable_backend_returns_error() {
    let Some(unavailable) = all_watchers().into_iter().find(|w| !w.available()) else {
        return;
    };
    let mut t = TestT::new(1);
    let dir = new_tmp_dir(&mut t);
    let err = unavailable.watch_directory(&dir, noop_callback(), vec![]).err().expect("expected an error");
    assert!(err.is(ErrUnavailable), "expected ErrUnavailable from {}, got {}", unavailable.name(), err);
}

// watcher_test.go:2420
#[test]
fn non_recursive_file_create() {
    run_for_each_watcher("TestNonRecursiveFileCreate", |t, w| {
        let dir = new_tmp_dir(t);
        let (r, _) = subscribe_for_opts(t, &dir, w, vec![])?;
        let f = sub_path(&dir);
        write(&f, "hello")?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &f)])?;
        Ok(())
    });
}

// watcher_test.go:2483
#[test]
fn non_recursive_grandchild_ignored() {
    run_for_each_watcher("TestNonRecursiveGrandchildIgnored", |t, w| {
        let dir = new_tmp_dir(t);
        let sub = format!("{}/child", dir);
        mkdir(&sub)?;
        let (r, _) = subscribe_for_opts(t, &dir, w, vec![])?;
        let grandchild = sub_path(&sub);
        write(&grandchild, "deep")?;
        let marker = sub_path(&dir);
        write(&marker, "flush")?;
        let mut got = expect_contains(&r, EventKind::Update, &marker)?;
        got.extend(r.drain_quiet(2 * maxWaitTime));
        assert_no_events_for_path(&got, &grandchild, "expected no events for grandchild")
    });
}

// watcher_test.go:2608
#[test]
fn file_watch_create() {
    run_for_each_watcher("TestFileWatchCreate", |t, w| {
        let dir = new_tmp_dir(t);
        let f = format!("{}/target.txt", dir);
        let (r, _) = subscribe_file_for(t, &f, w)?;
        write(&f, "hello")?;
        expect_event_sequence(&r, vec![want(EventKind::Update, &f)])?;
        Ok(())
    });
}

// watcher_test.go:2641
#[test]
fn file_watch_delete() {
    run_for_each_watcher("TestFileWatchDelete", |t, w| {
        let dir = new_tmp_dir(t);
        let f = format!("{}/target.txt", dir);
        write(&f, "x")?;
        let (r, _) = subscribe_file_for(t, &f, w)?;
        std::fs::remove_file(&f).map_err(|e| e.to_string())?;
        expect_event_sequence(&r, vec![want(EventKind::Delete, &f)])?;
        Ok(())
    });
}

// watcher_test.go:2659
#[test]
fn file_watch_ignores_siblings() {
    run_for_each_watcher("TestFileWatchIgnoresSiblings", |t, w| {
        let dir = new_tmp_dir(t);
        let target = format!("{}/target.txt", dir);
        let sibling = format!("{}/sibling.txt", dir);
        let (r, _) = subscribe_file_for(t, &target, w)?;
        let (witness, _) = subscribe_for_opts(t, &dir, w, vec![])?;
        write(&sibling, "noise")?;
        expect_contains(&witness, EventKind::Update, &sibling)?;
        expect_no_buffered_events(&r, "expected no events for sibling")
    });
}

// watcher_test.go:2809
#[test]
fn atomic_save() {
    run_for_each_watcher("TestAtomicSave", |t, w| {
        let dir = new_tmp_dir(t);
        let target = format!("{}/config.json", dir);
        write(&target, r#"{"v":1}"#)?;
        let (r, _) = subscribe_for(t, &dir, w)?;
        let tmp = format!("{}.tmp", target);
        write(&tmp, r#"{"v":2}"#)?;
        std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
        let got = r.wait_for_event(r.deadline(), |e| e.path == target);
        if filter_events_for_paths(&got, &[&target]).is_empty() {
            fatalf!("expected events for {} after atomic save, got none", target);
        }
        Ok(())
    });
}
