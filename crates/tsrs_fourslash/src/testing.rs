// Go's `*testing.T`, as far as the fourslash tests and harness use it. `t.Fatal` / `t.Skip` stop the test
// by unwinding with a `FatalPanic` / `SkipPanic` payload (Go: runtime.Goexit); the runner and `T::run`
// catch them.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;

pub struct FatalPanic;
pub struct SkipPanic;

#[derive(Default)]
struct TState {
    failed: bool,
    skipped: bool,
    finished: bool,
    logs: Vec<String>,
}

pub struct T {
    name: String,
    // Go file of the test (Go's runtime.Caller in NewFourslash), relative to the fourslash tests directory.
    file: String,
    state: Mutex<TState>,
}

impl T {
    pub fn new(name: &str, file: &str) -> T {
        T { name: name.to_string(), file: file.to_string(), state: Mutex::new(TState::default()) }
    }

    pub fn parallel(&self) {}

    pub fn helper(&self) {}

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    pub fn log(&self, msg: &str) {
        self.state.lock().unwrap().logs.push(msg.to_string());
    }

    pub fn error(&self, msg: &str) {
        let mut s = self.state.lock().unwrap();
        s.failed = true;
        s.logs.push(msg.to_string());
    }

    pub fn fail(&self) {
        self.state.lock().unwrap().failed = true;
    }

    pub fn fatal(&self, msg: &str) -> ! {
        self.error(msg);
        panic::panic_any(FatalPanic)
    }

    pub fn skip(&self, msg: &str) -> ! {
        {
            let mut s = self.state.lock().unwrap();
            s.skipped = true;
            if !msg.is_empty() {
                s.logs.push(msg.to_string());
            }
        }
        panic::panic_any(SkipPanic)
    }

    pub fn failed(&self) -> bool {
        self.state.lock().unwrap().failed
    }

    pub fn skipped(&self) -> bool {
        self.state.lock().unwrap().skipped
    }

    pub fn logs(&self) -> Vec<String> {
        self.state.lock().unwrap().logs.clone()
    }

    // Records a panic that was not a t.Fatal / t.Skip (Go reports those as test failures with the panic value).
    pub fn record_panic(&self, msg: &str) {
        self.error(&format!("panic: {msg}"));
    }

    // Runs f as a subtest named `<t.Name()>/<name>`; returns whether it passed. A failing subtest fails t.
    pub fn run(&self, name: &str, f: impl FnOnce(&T)) -> bool {
        let sub = T::new(&format!("{}/{}", self.name, rewrite(name)), &self.file);
        let r = panic::catch_unwind(AssertUnwindSafe(|| f(&sub)));
        if let Err(p) = r {
            if !p.is::<FatalPanic>() && !p.is::<SkipPanic>() {
                sub.record_panic(&crate::runner::panic_message(&p));
            }
        }
        let s = sub.state.into_inner().unwrap();
        let mut me = self.state.lock().unwrap();
        for l in s.logs {
            me.logs.push(format!("{}: {}", sub.name, l));
        }
        if s.failed {
            me.failed = true;
        }
        !s.failed
    }
}

// testing.rewrite: subtest names have spaces replaced by underscores.
fn rewrite(s: &str) -> String {
    s.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect()
}
