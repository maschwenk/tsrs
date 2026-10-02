use std::fmt;
use std::io::Write;
use std::sync::{Mutex, Weak};

use tsrs_lsproto as lsproto;
use tsrs_lsproto::{LogVerbosity, MessageType};

use crate::server::Server;

// logger.go:13. Go's nil-receiver checks (`if l == nil`) have no counterpart: the server always has a logger.
pub struct logger {
    server: Weak<Server>,
    mu: Mutex<LogVerbosity>,
}

// logger.go:19
pub(crate) fn new_logger(server: Weak<Server>) -> logger {
    logger { server, mu: Mutex::new(LogVerbosity::Info) }
}

// logger.go:28
// maxVerbosityForMessageType returns the least-verbose log level at which
// messages of the given LSP MessageType should still be sent.
fn max_verbosity_for_message_type(msg_type: MessageType) -> LogVerbosity {
    match msg_type {
        MessageType::Error => LogVerbosity::Error,
        MessageType::Warning => LogVerbosity::Warning,
        MessageType::Info => LogVerbosity::Info,
        MessageType::Debug => LogVerbosity::Debug,
        _ => LogVerbosity::Info,
    }
}

// logger.go:44
// isValidLogVerbosity reports whether v is one of the defined LogVerbosity values.
pub(crate) fn is_valid_log_verbosity(v: LogVerbosity) -> bool {
    v.0 >= LogVerbosity::Off.0 && v.0 <= LogVerbosity::Error.0
}

impl logger {
    // logger.go:48
    fn send_log_message(&self, msg_type: MessageType, message: String) {
        let Some(server) = self.server.upgrade() else {
            return;
        };

        if !server.init_started.load(std::sync::atomic::Ordering::SeqCst) {
            server.write_stderr_line(&message);
            return;
        }

        // Don't send messages that the client will filter out anyway.
        let verbosity = *self.mu.lock().unwrap();
        if verbosity == LogVerbosity::Off || verbosity.0 > max_verbosity_for_message_type(msg_type).0 {
            return;
        }

        let notification = lsproto::WINDOW_LOG_MESSAGE_INFO.new_notification_message(lsproto::LogMessageParams { type_: msg_type, message: message.clone() });

        let background_ctx = server.background_ctx();
        if server.outgoing_queue.put(&background_ctx, notification.message()).is_err() && background_ctx.err().is_some() {
            server.write_stderr_line(&message);
        }
    }

    // logger.go:78
    pub fn log(&self, msg: &str) {
        self.send_log_message(MessageType::Info, msg.to_string());
    }

    // logger.go:85
    pub fn logf(&self, args: fmt::Arguments<'_>) {
        self.send_log_message(MessageType::Info, fmt::format(args));
    }

    // logger.go:92
    pub fn verbose(&self) -> Option<&logger> {
        let verbosity = *self.mu.lock().unwrap();
        if verbosity == LogVerbosity::Off || verbosity.0 > LogVerbosity::Debug.0 {
            return None;
        }
        Some(self)
    }

    // logger.go:104
    pub fn is_verbose(&self) -> bool {
        let verbosity = *self.mu.lock().unwrap();
        verbosity.0 >= LogVerbosity::Trace.0 && verbosity.0 <= LogVerbosity::Debug.0
    }

    // logger.go:113
    pub fn set_verbose(&self, verbose: bool) {
        let mut verbosity = self.mu.lock().unwrap();
        if verbose {
            *verbosity = LogVerbosity::Debug;
        } else {
            *verbosity = LogVerbosity::Info;
        }
    }

    // logger.go:126
    pub fn is_tracing(&self) -> bool {
        *self.mu.lock().unwrap() == LogVerbosity::Trace
    }

    // logger.go:135
    pub fn set_verbosity(&self, verbosity: LogVerbosity) {
        *self.mu.lock().unwrap() = verbosity;
    }

    // logger.go:144
    pub fn error(&self, msg: &str) {
        self.send_log_message(MessageType::Error, msg.to_string());
    }

    // logger.go:151
    pub fn errorf(&self, args: fmt::Arguments<'_>) {
        self.send_log_message(MessageType::Error, fmt::format(args));
    }

    // logger.go:158
    pub fn warn(&self, msg: &str) {
        self.send_log_message(MessageType::Warning, msg.to_string());
    }

    // logger.go:165
    pub fn warnf(&self, args: fmt::Arguments<'_>) {
        self.send_log_message(MessageType::Warning, fmt::format(args));
    }

    // logger.go:172
    pub fn info(&self, msg: &str) {
        self.send_log_message(MessageType::Info, msg.to_string());
    }

    // logger.go:179
    pub fn infof(&self, args: fmt::Arguments<'_>) {
        self.send_log_message(MessageType::Info, fmt::format(args));
    }
}
