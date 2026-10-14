// Go's tests run the pool under `testing/synctest` (fake time); these use real time with short timeouts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tsrs_compiler::{CheckerHandle, Program};
use tsrs_core::context::{with_checker_lifetime, with_request_id, CheckerLifetime, Context};
use tsrs_core::tspath::Path;
use tsrs_lsproto as lsproto;
use tsrs_vfs::{bundled, vfstest, FS};

use super::*;
use crate::logging::new_test_logger;
use crate::session::{new_session, Session, SessionInit, SessionOptions};

fn ptr(c: &CheckerHandle) -> *const Checker {
    &raw const **c
}

fn ctx_with(request_id: &str, lifetime: CheckerLifetime) -> Context {
    with_checker_lifetime(&with_request_id(&Context::background(), request_id), lifetime)
}

// checkerpool_test.go:20
fn setup_checker_pool_session(opts: CheckerPoolOptions) -> (Arc<Session>, Arc<checkerPool>) {
    let files = [("/src/tsconfig.json", r#"{ "compilerOptions": { "noLib": true } }"#), ("/src/index.ts", "export const x: number = 1;")];
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files, false)));
    let session = new_session(&SessionInit {
        background_ctx: Context::background(),
        options: Arc::new(SessionOptions {
            current_directory: "/".to_string(),
            default_library_path: bundled::lib_path(),
            position_encoding: lsproto::PositionEncodingKind::UTF8,
            watch_enabled: false,
            logging_enabled: true,
            checker_pool_options: opts,
            ..Default::default()
        }),
        fs,
        client: None,
        logger: Some(new_test_logger()),
        npm_executor: None,
        parse_cache: None,
        content_mapped_parse_cache: None,
    });
    session.did_open_file(
        &Context::background(),
        lsproto::DocumentUri("file:///src/index.ts".to_string()),
        1,
        "export const x: number = 1;".to_string(),
        lsproto::LanguageKind::TypeScript,
    );

    let snapshot = session.snapshot();
    let project = snapshot.project_collection.configured_project(&Path::from("/src/tsconfig.json")).expect("expected configured project");
    let pool = project.checker_pool.clone().expect("expected checker pool");
    // Go's GC keeps the program alive through the pool's program pointer for as long as the test uses the pool; here
    // the project values own it (memregions.rs), and the session may replace this project's snapshot meanwhile.
    let _ = std::mem::ManuallyDrop::new(project);
    (session, pool)
}

fn program_of(session: &Session) -> &'static Program {
    session.get_language_service(&Context::background(), &lsproto::DocumentUri("file:///src/index.ts".to_string())).unwrap().get_program()
}

// checkerpool_test.go:55
fn new_test_checker_pool(program: &'static Program, opts: CheckerPoolOptions) -> Arc<checkerPool> {
    new_checker_pool(opts, program, Some(Box::new(|_: &str| {})))
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

// checkerpool_test.go:59
#[test]
fn checker_pool_diagnostics_routing() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });

    // Diagnostics requests should get checker at index 0.
    let c = pool.get_checker(&ctx_with("diag-req-1", CheckerLifetime::Diagnostics), None);
    assert_eq!(pool.checker_ptr(0), Some(ptr(&c)), "diagnostics should use checker index 0");
    drop(c);
}

// checkerpool_test.go:72
#[test]
fn checker_pool_query_routing() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });

    // Query requests should get a checker at index > 0.
    let c = pool.get_checker(&ctx_with("query-req-1", CheckerLifetime::Temporary), None);

    // Verify it's not the diagnostics checker slot.
    assert_ne!(pool.checker_ptr(0), Some(ptr(&c)), "query should not use checker index 0");
}

// checkerpool_test.go:87
#[test]
fn checker_pool_request_affinity() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });

    let (ctx, cancel) = Context::background().with_cancel();
    let ctx = with_checker_lifetime(&with_request_id(&ctx, "req-affinity"), CheckerLifetime::Temporary);

    // First call acquires.
    let c1 = pool.get_checker(&ctx, None);
    let p1 = ptr(&c1);
    drop(c1);

    // After release, same request should still get the same checker (cross-release affinity).
    let c2 = pool.get_checker(&ctx, None);
    assert_eq!(p1, ptr(&c2), "same request ID should return the same checker after release");
    drop(c2);
    cancel.call();
}

// checkerpool_test.go:108
#[test]
fn checker_pool_same_request_contention() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, ..Default::default() });
    let program = program_of(&session);
    for lifetime in [CheckerLifetime::Diagnostics, CheckerLifetime::Temporary, CheckerLifetime::API] {
        let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 2, ..Default::default() });
        let (ctx, cancel) = Context::background().with_cancel();
        let ctx = with_checker_lifetime(&with_request_id(&ctx, "same-request"), lifetime);

        let c1 = pool.get_checker(&ctx, None);
        let p1 = ptr(&c1) as usize;
        let acquired = Arc::new(AtomicBool::new(false));
        std::thread::scope(|s| {
            let (pool, ctx, acquired2) = (&pool, &ctx, std::sync::Arc::clone(&acquired));
            s.spawn(move || {
                let c2 = pool.get_checker(ctx, None);
                assert_eq!(p1, ptr(&c2) as usize);
                acquired2.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(100));
            assert!(!acquired.load(Ordering::SeqCst), "the request ID must not bypass exclusive acquisition");
            drop(c1);
        });
        assert!(acquired.load(Ordering::SeqCst), "waiting acquisition should finish after release");
        cancel.call();
    }
}

// checkerpool_test.go:150
#[test]
fn checker_pool_same_request_concurrent_queries() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 3, ..Default::default() });
    let (ctx, cancel) = Context::background().with_cancel();
    let ctx = with_request_id(&ctx, "same-request");

    let c1 = pool.get_checker(&ctx, None);
    let c2 = pool.get_checker(&ctx, None);
    assert_ne!(ptr(&c1), ptr(&c2), "overlapping acquisitions must use different checkers");
    assert_eq!(pool.query_sem_used(), 2, "each acquisition must hold its own slot");
    drop(c1);
    drop(c2);
    cancel.call();
}

// checkerpool_test.go:165
#[test]
fn checker_pool_idle_cleanup() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: Duration::from_millis(200) });

    // Create a checker via a diagnostics request.
    drop(pool.get_checker(&ctx_with("diag-cleanup", CheckerLifetime::Diagnostics), None));
    // Create a query checker as well.
    drop(pool.get_checker(&ctx_with("query-cleanup", CheckerLifetime::Temporary), None));

    // Both checkers should exist.
    let query_idx = pool.test_state(|st| {
        assert!(st.checkers[0], "diagnostics checker should exist");
        (1..st.checkers.len()).find(|&i| st.checkers[i]).expect("query checker should exist")
    });

    // Advance past idle timeout.
    std::thread::sleep(Duration::from_millis(800));

    // After cleanup, both checkers should be disposed.
    pool.test_state(|st| {
        assert!(!st.checkers[0], "diagnostics checker should be disposed after idle timeout");
        assert!(!st.checkers[query_idx], "query checker should be disposed after idle timeout");
    });
}

// checkerpool_test.go:217
#[test]
fn checker_pool_file_association_cleanup() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").unwrap();
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: Duration::from_millis(200) });

    // Create a query checker with file affinity.
    drop(pool.get_checker(&ctx_with("file-assoc-req", CheckerLifetime::Temporary), Some(source_file)));

    // File association should exist.
    assert!(pool.test_state(|st| st.file_associations.contains_key(&source_file)), "file should have a checker association");

    // Advance past idle timeout.
    std::thread::sleep(Duration::from_millis(800));

    // File association should be cleared.
    assert!(!pool.test_state(|st| st.file_associations.contains_key(&source_file)), "file association should be cleared after checker disposal");
}

// checkerpool_test.go:254
#[test]
fn checker_pool_min_checkers() {
    // Requesting maxCheckers=1 should be clamped to 2.
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 1, idle_timeout: secs(10) });
    assert_eq!(pool.opts().max_checkers, 2);
    assert_eq!(pool.test_state(|st| st.checkers.len()), 2);
}

// checkerpool_test.go:262
#[test]
fn checker_pool_default_idle_timeout() {
    // Zero idle timeout should default to 30s.
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, ..Default::default() });
    assert_eq!(pool.opts().idle_timeout, secs(30));
}

// checkerpool_test.go:353
#[test]
fn checker_pool_canceled_checker_disposal() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").expect("source file");
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    // Acquire a query checker and cancel it.
    let mut c = pool.get_checker(&ctx_with("cancel-test", CheckerLifetime::Temporary), None);
    let p = ptr(&c);

    let (canceled_ctx, cancel) = Context::background().with_cancel();
    cancel.call();
    c.get_diagnostics_exported(&canceled_ctx, source_file);
    assert!(c.was_canceled());

    // Release should dispose the canceled checker.
    drop(c);

    // Next request should get a fresh checker.
    let c2 = pool.get_checker(&ctx_with("after-cancel", CheckerLifetime::Temporary), None);
    assert_ne!(ptr(&c2), p, "should get a new checker, not the canceled one");
}

// checkerpool_test.go:389
#[test]
fn checker_pool_request_association_cleanup_on_disposal() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(5) });

    // Create a query checker with a request association.
    let (req_ctx, req_cancel) = Context::background().with_cancel();
    let ctx = with_checker_lifetime(&with_request_id(&req_ctx, "assoc-cleanup-req"), CheckerLifetime::Temporary);
    let mut c = pool.get_checker(&ctx, None);

    // Cancel the checker to trigger disposal on release.
    let (canceled_ctx, cancel) = Context::background().with_cancel();
    cancel.call();
    let source_file = program.get_source_file("/src/index.ts").expect("source file");
    c.get_diagnostics_exported(&canceled_ctx, source_file);
    assert!(c.was_canceled());

    drop(c);

    // Request association should be cleared after checker disposal.
    assert!(
        !pool.test_state(|st| st.request_associations.contains_key("assoc-cleanup-req")),
        "request association should be cleared after checker disposal"
    );
    req_cancel.call();
}

// checkerpool_test.go:425
#[test]
fn checker_pool_request_association_cleanup_on_context_done() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    // Create a cancellable context to simulate request lifecycle.
    let (req_ctx, req_cancel) = Context::background().with_cancel();
    let ctx = with_checker_lifetime(&with_request_id(&req_ctx, "ctx-cleanup-req"), CheckerLifetime::Temporary);

    drop(pool.get_checker(&ctx, None));

    // Association should still exist after release.
    assert!(pool.test_state(|st| st.request_associations.contains_key("ctx-cleanup-req")), "request association should persist after release");

    // Cancel the request context — association should be cleaned up.
    req_cancel.call();
    std::thread::sleep(Duration::from_millis(100));

    assert!(
        !pool.test_state(|st| st.request_associations.contains_key("ctx-cleanup-req")),
        "request association should be cleaned up after context cancellation"
    );
}

// checkerpool_test.go:592
#[test]
fn checker_pool_no_request_id() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });

    // Calls without a request ID should still work (e.g., callhierarchy uses context.Background()).
    let ctx = Context::background();
    drop(pool.get_checker(&ctx, None));
    drop(pool.get_checker(&ctx, None));
    // Without request ID, no affinity guarantee — just verify it doesn't crash.
}

// checkerpool_test.go:610
#[test]
fn checker_pool_diagnostics_cross_release_affinity() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let (req_ctx, req_cancel) = Context::background().with_cancel();
    let ctx = with_checker_lifetime(&with_request_id(&req_ctx, "diag-affinity"), CheckerLifetime::Diagnostics);

    let c1 = pool.get_checker(&ctx, None);
    let p1 = ptr(&c1);
    assert_eq!(pool.checker_ptr(0), Some(p1), "should be the diagnostics checker");
    drop(c1);

    // Same request reacquiring diagnostics should get the same checker.
    let c2 = pool.get_checker(&ctx, None);
    assert_eq!(ptr(&c2), p1, "same diagnostics request should get the same checker after release");
    drop(c2);
    req_cancel.call();
}

// checkerpool_test.go:746
#[test]
fn checker_pool_discard_still_functional() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });
    pool.discard();

    // Pool should still work — GetChecker should create a fresh checker.
    let c = pool.get_checker(&ctx_with("post-obs", CheckerLifetime::Temporary), None);
    let p = ptr(&c);

    // Find the slot.
    let idx = (1..4).find(|&i| pool.checker_ptr(i) == Some(p)).unwrap_or(0);
    assert!(idx > 0, "checker should be in a query slot");

    // Release — checker should persist on discarded pool (no cleanup timer).
    drop(c);
    assert_eq!(pool.checker_ptr(idx), Some(p), "checker should persist after release on discarded pool");

    // Re-acquire — should get the same checker back.
    let c2 = pool.get_checker(&ctx_with("post-obs-2", CheckerLifetime::Temporary), None);
    assert_eq!(ptr(&c2), p, "should get the same checker on discarded pool");
}

// checkerpool_test.go:792
#[test]
fn checker_pool_diagnostics_checker_stable_identity() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let c1 = pool.get_checker(&ctx_with("diag-stable-1", CheckerLifetime::Diagnostics), None);
    let p1 = ptr(&c1);
    drop(c1);

    // Re-acquire before idle timeout — should be the same instance.
    let c2 = pool.get_checker(&ctx_with("diag-stable-2", CheckerLifetime::Diagnostics), None);
    assert_eq!(ptr(&c2), p1, "diagnostics checker should be the same instance before idle timeout");
}

// checkerpool_test.go:819
#[test]
fn checker_pool_diagnostics_checker_survives_discard() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let c = pool.get_checker(&ctx_with("diag-discard", CheckerLifetime::Diagnostics), None);
    let p = ptr(&c);
    drop(c);

    pool.discard();

    // Diagnostics checker should survive Discard.
    assert_eq!(pool.checker_ptr(0), Some(p), "diagnostics checker should survive Discard");

    // Should still be acquirable and be the same instance.
    let c2 = pool.get_checker(&ctx_with("diag-discard-2", CheckerLifetime::Diagnostics), None);
    assert_eq!(ptr(&c2), p, "diagnostics checker identity should be stable after Discard");
}

// checkerpool_test.go:853
#[test]
fn checker_pool_diagnostics_checker_independent_from_query() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let diag_c = pool.get_checker(&ctx_with("diag-indep", CheckerLifetime::Diagnostics), None);
    let query_c = pool.get_checker(&ctx_with("query-indep", CheckerLifetime::Temporary), None);

    // They should be different checker instances.
    assert_ne!(ptr(&diag_c), ptr(&query_c), "diagnostics and query checkers should be different");
}

// checkerpool_test.go:880
#[test]
fn checker_pool_api_checker_stable_identity() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: Duration::from_millis(100) });

    let ctx = with_checker_lifetime(&Context::background(), CheckerLifetime::API);
    let c1 = pool.get_checker(&ctx, None);
    let p1 = ptr(&c1);
    drop(c1);

    let c2 = pool.get_checker(&ctx, None);
    assert_eq!(ptr(&c2), p1, "API checker should be the same instance");
    drop(c2);

    // Should survive idle timeout.
    std::thread::sleep(Duration::from_millis(400));

    let c3 = pool.get_checker(&ctx, None);
    assert_eq!(ptr(&c3), p1, "API checker should survive idle timeout");
}

// checkerpool_test.go:908
#[test]
fn checker_pool_api_checker_survives_discard() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let ctx = with_checker_lifetime(&Context::background(), CheckerLifetime::API);
    let c = pool.get_checker(&ctx, None);
    let p = ptr(&c);
    drop(c);

    pool.discard();

    assert_eq!(pool.persistent_ptr(), Some(p), "API checker should survive Discard");

    let c2 = pool.get_checker(&ctx, None);
    assert_eq!(ptr(&c2), p, "API checker identity should be stable after Discard");
}

// checkerpool_test.go:935
#[test]
fn checker_pool_all_three_independent() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let ded_c = pool.get_checker(&ctx_with("ded-req", CheckerLifetime::Diagnostics), None);
    let tmp_c = pool.get_checker(&ctx_with("tmp-req", CheckerLifetime::Temporary), None);
    let per_c = pool.get_checker(&with_checker_lifetime(&Context::background(), CheckerLifetime::API), None);

    assert_ne!(ptr(&ded_c), ptr(&tmp_c), "diagnostics and temporary should be different");
    assert_ne!(ptr(&ded_c), ptr(&per_c), "diagnostics and API should be different");
    assert_ne!(ptr(&tmp_c), ptr(&per_c), "temporary and API should be different");
}

// checkerpool_test.go:965
#[test]
fn checker_pool_file_affinity() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").unwrap();
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    // First query with a file should create a checker and associate it.
    let c1 = pool.get_checker(&ctx_with("file-aff-1", CheckerLifetime::Temporary), Some(source_file));
    let p1 = ptr(&c1);
    drop(c1);

    // Second query with the same file (different request) should get the same checker via file affinity.
    let c2 = pool.get_checker(&ctx_with("file-aff-2", CheckerLifetime::Temporary), Some(source_file));
    assert_eq!(ptr(&c2), p1, "same file should return the same checker via file affinity");
}

// checkerpool_test.go:1078
#[test]
fn checker_pool_default_max_checkers() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions::default());
    assert_eq!(pool.opts().max_checkers, 4);
}

// checkerpool_test.go:1151
#[test]
fn checker_pool_discard_idempotent() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });
    drop(pool.get_checker(&ctx_with("idem", CheckerLifetime::Temporary), None));
    pool.discard();
    pool.discard();
    pool.test_state(|st| {
        assert!(st.discarded);
        assert!(!st.has_cleanup_timer, "discarded pool should have no cleanup timer");
    });
}

// checkerpool_test.go:1194
#[test]
fn checker_pool_get_global_diagnostics_empty() {
    let (_, pool) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    assert_eq!(pool.get_global_diagnostics(), [] as [tsrs_core::P<tsrs_ast::Diagnostic>; 0]);
}

// checkerpool_test.go:1235
#[test]
fn checker_pool_api_checker_disposed_on_cancel() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").expect("source file");
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    let ctx = with_checker_lifetime(&Context::background(), CheckerLifetime::API);
    let mut c = pool.get_checker(&ctx, None);
    let p = ptr(&c);

    // Cancel the API checker.
    let (canceled_ctx, cancel) = Context::background().with_cancel();
    cancel.call();
    c.get_diagnostics_exported(&canceled_ctx, source_file);
    assert!(c.was_canceled());

    // Releasing a canceled API checker must drop it so it isn't reused.
    drop(c);

    assert!(!pool.test_state(|st| st.persistent), "canceled API checker should be dropped on release");

    // Next API acquisition gets a fresh, usable checker rather than panicking.
    let c2 = pool.get_checker(&ctx, None);
    assert_ne!(ptr(&c2), p, "should get a fresh API checker after cancellation");
}

// checkerpool_test.go:1272
#[test]
fn checker_pool_non_cancelable_context_no_affinity() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(10) });
    let pool = new_test_checker_pool(program_of(&session), CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    // A context that can never be canceled must not record request affinity.
    drop(pool.get_checker(&ctx_with("never-canceled", CheckerLifetime::Temporary), None));
    drop(pool.get_checker(&ctx_with("never-canceled", CheckerLifetime::Diagnostics), None));
    assert!(pool.test_state(|st| st.request_associations.is_empty()));
}

fn cancel_checker(c: &mut CheckerHandle, source_file: tsrs_core::P<tsrs_ast::SourceFile>) {
    let (canceled_ctx, cancel) = Context::background().with_cancel();
    cancel.call();
    c.get_diagnostics_exported(&canceled_ctx, source_file);
    assert!(c.was_canceled());
}

// tsrs-only (memory regions): a disposed checker and its region are freed once no request that held it can use its
// data any more, not with the program.
#[test]
fn checker_pool_disposed_checker_freed_after_last_lease() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").expect("source file");
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(30) });

    // Request A holds the query checker and releases it, keeping its results.
    let ctx_a = with_checker_data_lease(&ctx_with("lease-a", CheckerLifetime::Temporary));
    drop(pool.get_checker(&ctx_a, None));
    // Request B gets the same checker, is canceled, and the release disposes the checker.
    let ctx_b = with_checker_data_lease(&ctx_with("lease-b", CheckerLifetime::Temporary));
    let mut c = pool.get_checker(&ctx_b, None);
    cancel_checker(&mut c, source_file);
    drop(c);
    assert!(!pool.test_state(|st| st.checkers[1]), "canceled checker should be disposed");
    assert_eq!(pool.region_counts(), (1, 1, 0), "disposed checker kept while requests that held it are running");
    drop(ctx_b);
    assert_eq!(pool.region_counts(), (1, 1, 0), "request A can still use the checker's data");
    drop(ctx_a);
    assert_eq!(pool.region_counts(), (0, 0, 0), "disposed checker freed after its last request");

    // Idle disposal of a checker no request holds frees it right away.
    let idle_pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 2, idle_timeout: Duration::from_millis(100) });
    let ctx = with_checker_data_lease(&ctx_with("lease-idle", CheckerLifetime::Temporary));
    drop(idle_pool.get_checker(&ctx, None));
    drop(ctx);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!idle_pool.test_state(|st| st.checkers[1]), "idle checker should be disposed");
    assert_eq!(idle_pool.region_counts(), (0, 0, 0), "idle-disposed checker freed");
}

// tsrs-only (memory regions): a disposed checker whose data may be referenced beyond its requests is kept until the
// pool is freed.
#[test]
fn checker_pool_disposed_checker_parked_when_data_escaped() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").expect("source file");
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 4, idle_timeout: secs(30) });

    // Held without a lease (a caller outside a request): pinned.
    let mut c = pool.get_checker(&ctx_with("no-lease", CheckerLifetime::Temporary), None);
    cancel_checker(&mut c, source_file);
    drop(c);
    assert_eq!(pool.region_counts(), (1, 0, 1), "checker held without a lease is parked");

    // Something stored data of the checker's region in a longer-lived structure.
    let ctx = with_checker_data_lease(&ctx_with("escaped", CheckerLifetime::Temporary));
    let mut c = pool.get_checker(&ctx, None);
    tsrs_core::arena::note_escape(tsrs_core::P::new(1u64).addr());
    cancel_checker(&mut c, source_file);
    drop(c);
    drop(ctx);
    assert_eq!(pool.region_counts(), (2, 0, 2), "checker whose data escaped is parked");

    // The pool accumulates the diagnostics checker's global diagnostics (`noLib`: missing global types).
    let ctx = with_checker_data_lease(&ctx_with("globals", CheckerLifetime::Diagnostics));
    drop(pool.get_checker(&ctx, None));
    assert!(!pool.get_global_diagnostics().is_empty());
    let mut c = pool.get_checker(&ctx, None);
    cancel_checker(&mut c, source_file);
    drop(c);
    drop(ctx);
    assert_eq!(pool.region_counts(), (3, 0, 3), "diagnostics checker whose global diagnostics the pool keeps is parked");

    pool.free_checkers();
    assert_eq!(pool.region_counts(), (0, 0, 0));
}

// tsrs-only (memory regions): a checker taken with `context.Background()` inside a request handler (call hierarchy,
// as in Go) gets the request's lease from the thread instead of being pinned.
#[test]
fn checker_pool_thread_lease_for_background_context() {
    let (session, _) = setup_checker_pool_session(CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(10) });
    let program = program_of(&session);
    let source_file = program.get_source_file("/src/index.ts").expect("source file");
    let pool = new_test_checker_pool(program, CheckerPoolOptions { max_checkers: 2, idle_timeout: secs(30) });

    let ctx = with_checker_data_lease(&ctx_with("request", CheckerLifetime::Temporary));
    let scope = enter_checker_data_lease(&ctx);
    let mut c = pool.get_checker(&Context::background(), None);
    cancel_checker(&mut c, source_file);
    drop(c);
    assert_eq!(pool.region_counts(), (1, 1, 0), "kept for the request, not pinned");
    drop(scope);
    drop(ctx);
    assert_eq!(pool.region_counts(), (0, 0, 0), "freed once the request is done");
}

// tsrs-only (memory regions): global diagnostics that differ only in related information are merged into a copy made
// outside the checker's region (the pool merges after the checker's region scope ends), which still refers to the
// originals' related information; the pool must keep that region.
#[test]
fn checker_pool_merged_global_diagnostic_keeps_the_checker_region() {
    use tsrs_ast::{new_compiler_diagnostic, DiagnosticExt};
    use tsrs_diagnostics as diagnostics;
    let region = Region::new(1 << 16);
    let (first, second) = {
        let _scope = region.enter();
        let related = |name: &str| new_compiler_diagnostic(&diagnostics::X_0_is_declared_here, &[&name]);
        (
            new_compiler_diagnostic(&diagnostics::Cannot_find_global_type_0, &[&"Array"]).add_related_info(Some(related("a"))),
            new_compiler_diagnostic(&diagnostics::Cannot_find_global_type_0, &[&"Array"]).add_related_info(Some(related("b"))),
        )
    };
    let merged = accumulate_global_diagnostics(&[first, second]);
    assert_eq!(merged.len(), 1);
    assert!(Region::containing(merged[0].addr()).is_none_or(|r| !r.ptr_eq(&region)), "the merged copy is made outside the region");
    assert_eq!(merged[0].related_information().len(), 2);
    assert!(region.escaped(), "the region of the merged diagnostics' related information is kept");
}
