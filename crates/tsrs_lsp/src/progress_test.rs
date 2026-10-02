// progress_test.go. Go runs these in a synctest bubble (fake time, synctest.Wait = "every goroutine is
// blocked"); here time is real (sleeps a little past each delay) and `wait_idle` waits until the run thread is
// blocked with no queued event.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tsrs_core::context::{CancelFunc, Context};
use tsrs_diagnostics::Message;
use tsrs_lsproto as lsproto;

use crate::progress::{new_project_loading_progress_from_reporter, progressEvent, progressReporter, projectLoadingProgress};

// progress_test.go:15
#[derive(Clone, Debug, Default)]
struct progressCall {
    method: &'static str, // "create", "begin", "report", "end"
    token: String,
    title: String, // begin only
    msg: String,   // begin/report only
}

// progress_test.go:22
struct fakeProgressReporter {
    calls: Mutex<Vec<progressCall>>,
    ctx: Context,
}

impl progressReporter for fakeProgressReporter {
    // progress_test.go:28
    fn done(&self) -> Context {
        self.ctx.clone()
    }

    // progress_test.go:32
    fn localize(&self, msg: &'static Message, args: &[String]) -> String {
        tsrs_diagnostics::localize(Some(msg), tsrs_diagnostics::Key::default(), args)
    }

    // progress_test.go:36
    fn create_work_done_progress(&self, token: &str) {
        self.calls.lock().unwrap().push(progressCall { method: "create", token: token.to_string(), ..Default::default() });
    }

    // progress_test.go:42
    fn send_progress(&self, token: &str, value: lsproto::WorkDoneProgressBeginOrReportOrEnd) {
        let mut calls = self.calls.lock().unwrap();
        if let Some(begin) = value.begin {
            calls.push(progressCall { method: "begin", token: token.to_string(), title: begin.title, msg: begin.message.unwrap_or_default() });
        } else if let Some(report) = value.report {
            calls.push(progressCall { method: "report", token: token.to_string(), msg: report.message.unwrap_or_default(), ..Default::default() });
        } else if value.end.is_some() {
            calls.push(progressCall { method: "end", token: token.to_string(), ..Default::default() });
        }
    }
}

impl fakeProgressReporter {
    // progress_test.go:63
    fn get_calls(&self) -> Vec<progressCall> {
        self.calls.lock().unwrap().clone()
    }
}

fn setup(delay_ms: u64) -> (Arc<fakeProgressReporter>, Arc<projectLoadingProgress>, CancelFunc) {
    let (ctx, cancel) = Context::background().with_cancel();
    let reporter = Arc::new(fakeProgressReporter { calls: Mutex::new(Vec::new()), ctx });
    let p = new_project_loading_progress_from_reporter(reporter.clone(), Duration::from_millis(delay_ms));
    (reporter, p, cancel)
}

fn project(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

// Go's synctest.Sleep(d) right up to a timer's deadline: real time needs a margin for the timer thread to run.
fn sleep_past(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms + 100));
}

fn project_0() -> &'static Message {
    &tsrs_diagnostics::Project_0
}

// progress_test.go:72
#[test]
fn test_progress_start_finish_before_delay() {
    let (reporter, p, cancel) = setup(500);

    p.start(project_0(), project("myProject"));
    p.wait_idle();

    // Finish before the delay fires — no UI should appear.
    p.finish(project_0(), project("myProject"));
    p.wait_idle();

    // Advance time past the delay to ensure no progress is sent.
    std::thread::sleep(Duration::from_millis(600));

    let calls = reporter.get_calls();
    assert!(calls.is_empty(), "expected no progress calls for fast operation, got {:?}", calls);

    cancel.call();
}

// progress_test.go:96
#[test]
fn test_progress_shows_after_delay() {
    let (reporter, p, cancel) = setup(200);

    p.start(project_0(), project("myProject"));
    p.wait_idle();

    // Let the delay fire.
    sleep_past(200);
    p.wait_idle();

    let calls = reporter.get_calls();
    assert_eq!(calls.len(), 2, "expected 2 calls (create + begin), got {:?}", calls);
    assert_eq!(calls[0].method, "create", "expected create, got {:?}", calls[0]);
    assert_eq!(calls[1].method, "begin", "expected begin, got {:?}", calls[1]);
    assert_eq!(calls[1].title, tsrs_diagnostics::Loading.text(), "expected title {:?}, got {:?}", tsrs_diagnostics::Loading.text(), calls[1].title);

    // Finish the operation.
    p.finish(project_0(), project("myProject"));
    p.wait_idle();

    let calls = reporter.get_calls();
    let last = calls.last().unwrap();
    assert_eq!(last.method, "end", "expected end, got {:?}", last);

    cancel.call();
}

// progress_test.go:139
#[test]
fn test_progress_reports_multiple_operations() {
    let (reporter, p, cancel) = setup(100);

    // Start two different operations.
    p.start(project_0(), project("projA"));
    p.start(project_0(), project("projB"));
    p.wait_idle();

    // Let the delay fire.
    sleep_past(100);
    p.wait_idle();

    let calls = reporter.get_calls();
    // Should have: create, begin (with first message).
    assert!(calls.len() >= 2, "expected at least 2 calls, got {:?}", calls);
    assert_eq!(calls[0].method, "create", "expected create, got {:?}", calls[0]);
    assert_eq!(calls[1].method, "begin", "expected begin, got {:?}", calls[1]);

    // Finish one — should send a report with the remaining operation.
    p.finish(project_0(), project("projA"));
    p.wait_idle();

    let calls = reporter.get_calls();
    assert!(calls.iter().any(|c| c.method == "report"), "expected a report after partial finish, got {:?}", calls);

    // Finish the second — should send end.
    p.finish(project_0(), project("projB"));
    p.wait_idle();

    let calls = reporter.get_calls();
    let last = calls.last().unwrap();
    assert_eq!(last.method, "end", "expected end, got {:?}", last);

    cancel.call();
}

// progress_test.go:198
#[test]
fn test_progress_ref_counting() {
    let (reporter, p, cancel) = setup(100);

    // Start the same operation twice (ref count = 2).
    p.start(project_0(), project("proj"));
    p.start(project_0(), project("proj"));
    p.wait_idle();

    sleep_past(100);
    p.wait_idle();

    // Finish once (ref count = 1) — should NOT end.
    p.finish(project_0(), project("proj"));
    p.wait_idle();

    let calls = reporter.get_calls();
    assert!(!calls.iter().any(|c| c.method == "end"), "unexpected end with ref count > 0: {:?}", calls);

    // Finish again (ref count = 0) — should end.
    p.finish(project_0(), project("proj"));
    p.wait_idle();

    let calls = reporter.get_calls();
    let last = calls.last().unwrap();
    assert_eq!(last.method, "end", "expected end when ref count reaches 0, got {:?}", last);

    cancel.call();
}

// progress_test.go:238
#[test]
fn test_progress_new_token_after_end() {
    let (reporter, p, cancel) = setup(100);

    // First cycle.
    p.start(project_0(), project("proj"));
    p.wait_idle();
    sleep_past(100);
    p.wait_idle();

    let calls = reporter.get_calls();
    let first_token = calls[0].token.clone();

    p.finish(project_0(), project("proj"));
    p.wait_idle();

    // Second cycle — should get a new token.
    p.start(project_0(), project("proj2"));
    p.wait_idle();
    sleep_past(100);
    p.wait_idle();

    let calls = reporter.get_calls();
    let second_token = calls.iter().find(|c| c.method == "create" && c.token != first_token).map(|c| c.token.clone()).unwrap_or_default();
    assert!(!second_token.is_empty(), "expected a new token for second cycle, got calls: {:?}", calls);
    assert_ne!(first_token, second_token, "expected different tokens, both were {:?}", first_token);

    p.finish(project_0(), project("proj2"));
    p.wait_idle();

    cancel.call();
}

// progress_test.go:283
#[test]
fn test_progress_start_before_delay_then_more_after_delay() {
    let (reporter, p, cancel) = setup(200);

    // Start before delay.
    p.start(project_0(), project("projA"));
    p.wait_idle();

    // Let delay fire.
    sleep_past(200);
    p.wait_idle();

    let calls = reporter.get_calls();
    assert!(calls.len() >= 2, "expected create + begin after delay, got {:?}", calls);

    // Start another operation after delay — should send a report immediately.
    p.start(project_0(), project("projB"));
    p.wait_idle();

    let calls = reporter.get_calls();
    let last = calls.last().unwrap();
    assert_eq!(last.method, "report", "expected report for new start after delay, got {:?}", last);

    // Clean up.
    p.finish(project_0(), project("projA"));
    p.finish(project_0(), project("projB"));
    p.wait_idle();

    cancel.call();
}

// progress_test.go:322
#[test]
fn test_progress_finish_with_no_active_token() {
    let (reporter, p, cancel) = setup(100);

    // Finish without any prior start — should be a no-op.
    p.finish(project_0(), project("proj"));
    p.wait_idle();

    let calls = reporter.get_calls();
    assert!(calls.is_empty(), "expected no calls for orphan finish, got {:?}", calls);

    cancel.call();
}

// progress_test.go:343
#[test]
fn test_progress_shutdown_during_start_and_finish() {
    let (_reporter, p, cancel) = setup(100);

    // Cancel context so the run goroutine exits.
    cancel.call();
    p.wait_idle();

    // Fill the channel buffer so start/finish block on send.
    for _ in 0..p.capacity() {
        assert!(p.fill(progressEvent { message: project_0(), args: project("fill"), finish: false }));
    }

    // These should return immediately via the done() path
    // since the channel is full and the context is cancelled.
    p.start(project_0(), project("proj"));
    p.finish(project_0(), project("proj"));
}

// progress_test.go:366
#[test]
fn test_progress_shutdown_with_active_timer() {
    let (_reporter, p, cancel) = setup(500);

    // Start an operation so the delay timer is created.
    p.start(project_0(), project("proj"));
    p.wait_idle();

    // Shutdown while the delay timer is still pending.
    cancel.call();
    p.wait_idle();
}

// progress_test.go:384
#[test]
fn test_progress_zero_delay() {
    let (reporter, p, cancel) = setup(0);

    // With zero delay, progress should begin immediately.
    p.start(project_0(), project("proj"));
    p.wait_idle();

    let calls = reporter.get_calls();
    assert_eq!(calls.len(), 2, "expected 2 calls (create + begin), got {:?}", calls);
    assert_eq!(calls[0].method, "create", "expected create, got {:?}", calls[0]);
    assert_eq!(calls[1].method, "begin", "expected begin, got {:?}", calls[1]);
    assert_eq!(calls[1].msg, "Project 'proj'", "expected message {:?}, got {:?}", "Project 'proj'", calls[1].msg);

    // Start+finish should still produce begin and end.
    p.finish(project_0(), project("proj"));
    p.wait_idle();

    let calls = reporter.get_calls();
    let last = calls.last().unwrap();
    assert_eq!(last.method, "end", "expected end, got {:?}", last);

    cancel.call();
}

// progress_test.go:422
#[test]
fn test_progress_finish_before_delay_no_begun() {
    let (reporter, p, cancel) = setup(500);

    // Start, then finish before delay — begun is false, so no end is sent.
    p.start(project_0(), project("proj"));
    p.wait_idle();
    p.finish(project_0(), project("proj"));
    p.wait_idle();

    let calls = reporter.get_calls();
    assert!(!calls.iter().any(|c| c.method == "end"), "unexpected end when begun=false: {:?}", calls);

    cancel.call();
}
