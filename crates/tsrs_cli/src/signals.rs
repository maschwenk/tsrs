// signal.NotifyContext(ctx, os.Interrupt, syscall.SIGTERM) as lsp.go and api.go use it: SIGINT and
// SIGTERM cancel the server's context instead of terminating the process. The command line keeps the
// signals' default action (docs/COMPATIBILITY.md).
//
// The signals are blocked in every thread and taken by one thread with sigwait, so the hooks run as
// ordinary code rather than in a signal handler. Not implemented on Windows, where the signals keep
// their default action.

use std::sync::Mutex;

type Hook = Box<dyn FnOnce() + Send>;

struct State {
    received: bool,
    hooks: Vec<Hook>,
}

static STATE: Mutex<State> = Mutex::new(State { received: false, hooks: Vec::new() });

/// Starts catching SIGINT and SIGTERM. Call it before the process starts any other thread: threads
/// inherit the signal mask, and a thread that does not block the signals would take them with their
/// default action. Child processes start with an empty mask (std::process::Command resets it).
#[cfg_attr(unix, expect(clippy::infinite_loop, reason = "the signal thread waits for process interrupts until process exit"))]
pub(crate) fn notify_interrupts() {
    #[cfg(unix)]
    {
        // SAFETY: sigset_t is a plain bit set that sigemptyset initializes; the calls only change
        // this thread's signal mask.
        let set = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut set);
            libc::sigaddset(&raw mut set, libc::SIGINT);
            libc::sigaddset(&raw mut set, libc::SIGTERM);
            libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
            set
        };
        std::thread::Builder::new()
            .name("signals".to_string())
            .spawn(move || loop {
                let mut signal = 0;
                // SAFETY: `set` and `signal` are valid for the call.
                if unsafe { libc::sigwait(&raw const set, &raw mut signal) } == 0 {
                    interrupted();
                }
            })
            .expect("failed to spawn the signal thread");
    }
}

/// Runs `hook` once SIGINT or SIGTERM arrives, or now if one already has.
pub(crate) fn on_interrupt(hook: impl FnOnce() + Send + 'static) {
    let mut state = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.received {
        drop(state);
        hook();
    } else {
        state.hooks.push(Box::new(hook));
    }
}

#[cfg(unix)]
fn interrupted() {
    let hooks = {
        let mut state = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.received = true;
        std::mem::take(&mut state.hooks)
    };
    for hook in hooks {
        hook();
    }
}
