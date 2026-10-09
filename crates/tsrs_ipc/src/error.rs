// Go `error` values as packages ipc and jsonrpc create and test them. Go wraps errors (`fmt.Errorf("...%w", err)`),
// joins them (`errors.Join`) and tests them against sentinel values (`errors.Is(err, io.EOF)`); the port keeps the
// text (`err.Error()`) and the sentinels an error wraps.

use std::fmt;

// The sentinel errors that this package and its callers test with `errors.Is`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorTag {
    // io.EOF: the stream ended where a message would start (Run returns nil for it).
    EOF,
    // ErrConnClosed (conn.go:11): the connection's read loop has exited.
    ConnClosed,
    // context.DeadlineExceeded: `call_with_timeout` stopped waiting (Go's Call returns `ctx.Err()`).
    DeadlineExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    message: String,
    // The sentinels this error wraps, outermost first.
    tags: Vec<ErrorTag>,
}

impl Error {
    // Go `errors.New(message)`, or `fmt.Errorf` without `%w`.
    pub fn new(message: impl Into<String>) -> Error {
        Error { message: message.into(), tags: Vec::new() }
    }

    // A sentinel error value (Go `io.EOF`, `ipc.ErrConnClosed`, `context.DeadlineExceeded`).
    pub fn tagged(tag: ErrorTag, message: impl Into<String>) -> Error {
        Error { message: message.into(), tags: vec![tag] }
    }

    // Go `fmt.Errorf(prefix + "%w", err)`.
    pub fn wrap(prefix: &str, err: Error) -> Error {
        Error { message: format!("{prefix}{}", err.message), tags: err.tags }
    }

    // Go `errors.Join(first, second)` of two non-nil errors: the texts on separate lines, both chains for `errors.Is`.
    pub fn join(first: &Error, second: &Error) -> Error {
        let mut tags = first.tags.clone();
        tags.extend_from_slice(&second.tags);
        Error { message: format!("{}\n{}", first.message, second.message), tags }
    }

    // The tail of a `fmt.Errorf("...%w ...", err)` format after the wrapped error.
    pub(crate) fn with_suffix(mut self, suffix: &str) -> Error {
        self.message.push_str(suffix);
        self
    }

    // Go `errors.Is(err, sentinel)`.
    pub fn is(&self, tag: ErrorTag) -> bool {
        self.tags.contains(&tag)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

// io.EOF.
pub(crate) fn eof() -> Error {
    Error::tagged(ErrorTag::EOF, "EOF")
}
