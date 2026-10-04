use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use tsrs_core::collections::OrderedMap;
use tsrs_core::context::Context;
use tsrs_diagnostics::Message;
use tsrs_lsproto as lsproto;

use crate::dynamic_queue::wait_until;
use crate::server::Server;

// progress.go:13. Go's `args []any` are stringified when the event is created (they cross threads).
pub(crate) struct progressEvent {
    pub(crate) message: &'static Message,
    pub(crate) args: Vec<String>,
    pub(crate) finish: bool,
}

// progress.go:22
// progressReporter abstracts the LSP transport operations needed by
// projectLoadingProgress so the progress logic can be tested without a
// full Server instance.
pub(crate) trait progressReporter: Send + Sync {
    // done returns a context that is canceled when the server is shutting down (Go: a channel that is closed).
    fn done(&self) -> Context;
    // localize converts a diagnostic message to a display string.
    fn localize(&self, msg: &'static Message, args: &[String]) -> String;
    // createWorkDoneProgress asks the client to create a progress token.
    fn create_work_done_progress(&self, token: &str);
    // sendProgress sends a $/progress notification.
    fn send_progress(&self, token: &str, value: lsproto::WorkDoneProgressBeginOrReportOrEnd);
}

// progress.go:34
// serverProgressReporter adapts *Server to the progressReporter interface.
struct serverProgressReporter {
    server: Weak<Server>,
}

impl progressReporter for serverProgressReporter {
    // progress.go:38
    fn done(&self) -> Context {
        match self.server.upgrade() {
            Some(server) => server.background_ctx(),
            None => {
                let (ctx, cancel) = Context::background().with_cancel();
                cancel.call();
                ctx
            }
        }
    }

    // progress.go:42
    fn localize(&self, msg: &'static Message, args: &[String]) -> String {
        // English only (docs/LSP.md): the server's locale does not change the text.
        tsrs_diagnostics::localize(Some(msg), tsrs_diagnostics::Key::default(), args)
    }

    // progress.go:46
    fn create_work_done_progress(&self, token: &str) {
        if let Some(server) = self.server.upgrade() {
            let _ = server.send_client_request_fire_and_forget(
                lsproto::WINDOW_WORK_DONE_PROGRESS_CREATE_INFO,
                lsproto::WorkDoneProgressCreateParams { token: lsproto::IntegerOrString { string: Some(token.to_string()), ..Default::default() } },
            );
        }
    }

    // progress.go:52
    fn send_progress(&self, token: &str, value: lsproto::WorkDoneProgressBeginOrReportOrEnd) {
        if let Some(server) = self.server.upgrade() {
            let _ = server.send_notification(
                lsproto::PROGRESS_INFO,
                lsproto::ProgressParams { token: lsproto::IntegerOrString { string: Some(token.to_string()), ..Default::default() }, value },
            );
        }
    }
}

const PROGRESS_CHANNEL_CAPACITY: usize = 64;

// Go's `ch chan progressEvent` (buffered, 64): a bounded queue; senders block while it is full.
pub(crate) struct progressChannel {
    state: Mutex<progressChannelState>,
    // Signaled when an event is queued or the reporter is done (the run thread waits on it).
    not_empty: Condvar,
    // Signaled when an event is taken out (blocked senders wait on it).
    not_full: Condvar,
}

pub(crate) struct progressChannelState {
    pub(crate) events: VecDeque<progressEvent>,
    // Test support (Go's synctest.Wait): the run thread is blocked waiting for an event, the timer or done.
    idle: bool,
    exited: bool,
}

// progress.go:70
// projectLoadingProgress manages LSP WorkDoneProgress indicators for
// long-running operations. A single persistent goroutine processes
// start/finish events, maintains a ref-counted map of active operations,
// and sends progress messages in order.
//
// To avoid flickering on fast operations, the indicator is not shown
// until progressDelay has elapsed since the first start event. If all
// operations complete before then, no progress UI is displayed.
//
// start/finish may block if the internal buffer (64 events) is full,
// but will bail out if the server's background context is cancelled.
pub(crate) struct projectLoadingProgress {
    reporter: Arc<dyn progressReporter>,
    pub(crate) ch: Arc<progressChannel>,
    delay: Duration, // time to wait before showing progress UI
}

// progress.go:76
pub(crate) fn new_project_loading_progress(server: Weak<Server>, delay: Duration) -> Arc<projectLoadingProgress> {
    new_project_loading_progress_from_reporter(Arc::new(serverProgressReporter { server }), delay)
}

// progress.go:80
pub(crate) fn new_project_loading_progress_from_reporter(reporter: Arc<dyn progressReporter>, delay: Duration) -> Arc<projectLoadingProgress> {
    let p = Arc::new(projectLoadingProgress {
        reporter,
        ch: Arc::new(progressChannel {
            state: Mutex::new(progressChannelState { events: VecDeque::with_capacity(PROGRESS_CHANNEL_CAPACITY), idle: false, exited: false }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
        }),
        delay,
    });
    let runner = Arc::clone(&p);
    std::thread::Builder::new().name("lsp-progress".to_string()).spawn(move || runner.run()).expect("failed to spawn the progress thread");
    p
}

impl projectLoadingProgress {
    fn send_event(&self, ev: progressEvent) {
        let done = self.reporter.done();
        let ch = Arc::clone(&self.ch);
        let guard = self.ch.state.lock().unwrap();
        let wake = move || {
            drop(ch.state.lock().unwrap());
            ch.not_full.notify_all();
        };
        match wait_until(&done, &self.ch.not_full, guard, |s| s.events.len() < PROGRESS_CHANNEL_CAPACITY, wake) {
            Ok(mut state) => {
                // Sent successfully.
                state.events.push_back(ev);
                drop(state);
                self.ch.not_empty.notify_all();
            }
            Err(_) => {
                // Server shutting down; drop the event.
            }
        }
    }

    // progress.go:90
    pub(crate) fn start(&self, message: &'static Message, args: Vec<String>) {
        self.send_event(progressEvent { message, args, finish: false });
    }

    // progress.go:99
    pub(crate) fn finish(&self, message: &'static Message, args: Vec<String>) {
        self.send_event(progressEvent { message, args, finish: true });
    }

    // progress.go:110
    // run is the persistent goroutine that processes all progress events.
    // It owns all mutable state: no external synchronization needed.
    fn run(&self) {
        let mut loading: OrderedMap<String, i32> = OrderedMap::default();
        let mut token = String::new(); // current token; empty if no progress active
        let mut token_id = 0;
        let mut begun = false; // whether "begin" has been sent for the current token

        // Go's *time.Timer: the instant the delay fires; None when no timer is pending (a fired timer delivers
        // once, so it is cleared when it fires; stopping it is clearing it).
        let mut delay: Option<Instant> = None;
        let mut delay_fired = false; // true after the delay timer fires

        let done = self.reporter.done();
        let ch = Arc::clone(&self.ch);
        let _stop = done.after_func(move || {
            drop(ch.state.lock().unwrap());
            ch.not_empty.notify_all();
        });

        enum selected {
            event(progressEvent),
            delay,
            done,
        }

        loop {
            let sel = {
                let mut state = self.ch.state.lock().unwrap();
                loop {
                    if done.is_canceled() {
                        break selected::done;
                    }
                    if let Some(ev) = state.events.pop_front() {
                        self.ch.not_full.notify_all();
                        break selected::event(ev);
                    }
                    if let Some(deadline) = delay {
                        let now = Instant::now();
                        if now >= deadline {
                            delay = None;
                            break selected::delay;
                        }
                        state.idle = true;
                        state = self.ch.not_empty.wait_timeout(state, deadline - now).unwrap().0;
                    } else {
                        state.idle = true;
                        state = self.ch.not_empty.wait(state).unwrap();
                    }
                    state.idle = false;
                }
            };

            match sel {
                selected::event(ev) => {
                    let text = self.reporter.localize(ev.message, &ev.args);
                    if !ev.finish {
                        let count = loading.get(&text).copied().unwrap_or(0);
                        loading.insert(text.clone(), count + 1);
                        if token.is_empty() {
                            token_id += 1;
                            token = format!("tsgo-loading-{}", token_id);
                            begun = false;
                            if self.delay.is_zero() {
                                delay_fired = true;
                                self.reporter.create_work_done_progress(&token);
                            } else {
                                delay_fired = false;
                                delay = Some(Instant::now() + self.delay);
                            }
                        }
                        if delay_fired {
                            begun = self.begin_or_report(&token, &text, begun);
                        }
                    } else {
                        let count = loading.get(&text).copied().unwrap_or(0);
                        if count <= 1 {
                            loading.shift_remove(&text);
                        } else {
                            loading.insert(text.clone(), count - 1);
                        }
                        if token.is_empty() {
                            continue;
                        }
                        if loading.is_empty() {
                            if begun {
                                self.reporter.send_progress(
                                    &token,
                                    lsproto::WorkDoneProgressBeginOrReportOrEnd { end: Some(lsproto::WorkDoneProgressEnd::default()), ..Default::default() },
                                );
                            }
                            delay = None;
                            token = String::new();
                        } else if delay_fired {
                            let first = loading.keys().next().cloned().unwrap_or_default();
                            self.reporter.send_progress(
                                &token,
                                lsproto::WorkDoneProgressBeginOrReportOrEnd {
                                    report: Some(lsproto::WorkDoneProgressReport { message: Some(first), ..Default::default() }),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }

                selected::delay => {
                    delay_fired = true;
                    if !token.is_empty() && !loading.is_empty() {
                        self.reporter.create_work_done_progress(&token);
                        let first = loading.keys().next().cloned().unwrap_or_default();
                        begun = self.begin_or_report(&token, &first, begun);
                    }
                }

                selected::done => {
                    let mut state = self.ch.state.lock().unwrap();
                    state.exited = true;
                    state.idle = true;
                    return;
                }
            }
        }
    }

    // progress.go:200
    // beginOrReport sends WorkDoneProgressBegin if not yet begun, otherwise
    // sends WorkDoneProgressReport. Returns true to indicate begun state.
    fn begin_or_report(&self, token: &str, text: &str, begun: bool) -> bool {
        if !begun {
            let title = self.reporter.localize(&tsrs_diagnostics::Loading, &[]);
            self.reporter.send_progress(
                token,
                lsproto::WorkDoneProgressBeginOrReportOrEnd {
                    begin: Some(lsproto::WorkDoneProgressBegin { title, message: Some(text.to_string()), ..Default::default() }),
                    ..Default::default()
                },
            );
        } else {
            self.reporter.send_progress(
                token,
                lsproto::WorkDoneProgressBeginOrReportOrEnd {
                    report: Some(lsproto::WorkDoneProgressReport { message: Some(text.to_string()), ..Default::default() }),
                    ..Default::default()
                },
            );
        }
        true
    }

    // Test support (Go's synctest.Wait): blocks until no event is queued and the run thread is waiting.
    #[cfg(test)]
    pub(crate) fn wait_idle(&self) {
        loop {
            {
                let state = self.ch.state.lock().unwrap();
                if (state.events.is_empty() && state.idle) || state.exited {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // Test support: queue an event without waiting (Go's `p.ch <- ev` while the buffer has room).
    #[cfg(test)]
    pub(crate) fn fill(&self, ev: progressEvent) -> bool {
        let mut state = self.ch.state.lock().unwrap();
        if state.events.len() >= PROGRESS_CHANNEL_CAPACITY {
            return false;
        }
        state.events.push_back(ev);
        true
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        PROGRESS_CHANNEL_CAPACITY
    }
}
