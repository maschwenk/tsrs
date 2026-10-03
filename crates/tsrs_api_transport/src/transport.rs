// Port of tsc/internal/ipc/transport.go (+ transport_unix.go) and the connection setup in
// tsc/internal/api/server.go Run: stdio or a Unix domain socket (`--pipe <path>`), MessagePack +
// SyncConn by default, JSON-RPC + AsyncConn with `--async`.
//
// Windows named pipes (transport_windows.go) are not implemented: `--pipe` returns an explicit
// Unsupported error on non-Unix targets. The pinned sync Node client uses `--pipe` on Windows, so
// the sync client is unsupported on Windows until named pipes are added.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::conn_async::AsyncConn;
use crate::conn_sync::{ConnOptions, SyncConn};
use crate::handler::{Caller, CancellationToken, Handler};
use crate::jsonrpc::{FrameReader, FrameWriter};
use crate::message::{TransportError, DEFAULT_MAX_FRAME_BYTES};
use crate::msgpack::{MessagePackReader, MessagePackWriter};
use crate::protocol::WireProtocol;

/// One accepted bidirectional stream.
pub struct Stream {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    /// Shuts the stream down so a blocked reader wakes (sockets). None for stdio.
    pub closer: Option<Box<dyn Fn() + Send + Sync>>,
}

impl Stream {
    /// StdioTransport. Stdout is the wire: nothing else in the process may write to it.
    pub fn stdio() -> Stream {
        Stream { reader: Box::new(std::io::stdin()), writer: Box::new(std::io::stdout()), closer: None }
    }
}

/// PipeTransport: listens on `path` (removing a stale socket file first) and accepts one connection.
pub struct PipeListener {
    #[cfg(unix)]
    listener: std::os::unix::net::UnixListener,
    path: PathBuf,
}

impl PipeListener {
    #[cfg(unix)]
    pub fn bind(path: &Path) -> std::io::Result<PipeListener> {
        let _ = std::fs::remove_file(path);
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        Ok(PipeListener { listener, path: path.to_path_buf() })
    }

    #[cfg(not(unix))]
    pub fn bind(path: &Path) -> std::io::Result<PipeListener> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("--pipe {}: named pipe transport is not supported on this platform", path.display()),
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(unix)]
    pub fn accept(&self) -> std::io::Result<Stream> {
        let (stream, _) = self.listener.accept()?;
        let reader = stream.try_clone()?;
        let closer = stream.try_clone()?;
        Ok(Stream {
            reader: Box::new(reader),
            writer: Box::new(stream),
            closer: Some(Box::new(move || {
                let _ = closer.shutdown(std::net::Shutdown::Both);
            })),
        })
    }

    #[cfg(not(unix))]
    pub fn accept(&self) -> std::io::Result<Stream> {
        unreachable!("PipeListener cannot be bound on this platform")
    }
}

impl Drop for PipeListener {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.path);
    }
}

/// ipc.GeneratePipePath (Unix): `<tmpdir>/<name>`.
pub fn generate_pipe_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// A caller slot filled once the connection exists (Go: SetConnection after Accept), so the
/// session and callback filesystem can be built before the connection that owns them.
#[derive(Default)]
pub struct LateCaller(OnceLock<Arc<dyn Caller>>);

impl LateCaller {
    pub fn new() -> Arc<LateCaller> {
        Arc::new(LateCaller::default())
    }

    pub fn set(&self, caller: Arc<dyn Caller>) {
        let _ = self.0.set(caller);
    }
}

impl Caller for LateCaller {
    fn call(&self, method: &str, params: Option<&[u8]>) -> Result<Vec<u8>, TransportError> {
        match self.0.get() {
            Some(c) => c.call(method, params),
            None => Err(TransportError::Closed(Some(format!("{method} called before connection set")))),
        }
    }

    fn notify(&self, method: &str, params: Option<&[u8]>) -> Result<(), TransportError> {
        match self.0.get() {
            Some(c) => c.notify(method, params),
            None => Err(TransportError::Closed(Some(format!("{method} called before connection set")))),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ServeOptions {
    pub protocol: WireProtocol,
    pub collect_timing: bool,
    pub max_frame_bytes: u64,
    /// See `ConnOptions::reentrancy_grace`.
    pub reentrancy_grace: std::time::Duration,
}

impl ServeOptions {
    /// `--api` (sync, MessagePack) or `--api --async` (JSON-RPC).
    pub fn new(is_async: bool) -> ServeOptions {
        ServeOptions {
            protocol: if is_async { WireProtocol::JsonRpc } else { WireProtocol::MessagePack },
            collect_timing: false,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            reentrancy_grace: crate::reentrancy::DEFAULT_ASYNC_GRACE,
        }
    }
}

/// A connection ready to run.
pub enum Connection {
    Sync(Arc<SyncConn>),
    Async(Arc<AsyncConn>),
}

impl Connection {
    /// Builds the protocol + connection for `stream` (server.go Run), and points `late` at it.
    pub fn new(stream: Stream, handler: Arc<dyn Handler>, options: ServeOptions, late: Option<&LateCaller>) -> Connection {
        let conn_options = ConnOptions { collect_timing: options.collect_timing, reentrancy_grace: options.reentrancy_grace };
        let conn = match options.protocol {
            WireProtocol::MessagePack => Connection::Sync(SyncConn::new(
                Box::new(MessagePackReader::with_limit(stream.reader, options.max_frame_bytes)),
                Box::new(MessagePackWriter::new(stream.writer)),
                handler,
                conn_options,
            )),
            WireProtocol::JsonRpc => Connection::Async(AsyncConn::new(
                Box::new(FrameReader::with_limit(stream.reader, options.max_frame_bytes)),
                Box::new(FrameWriter::new(stream.writer)),
                handler,
                conn_options,
                stream.closer,
            )),
        };
        if let Some(late) = late {
            late.set(conn.caller());
        }
        conn
    }

    pub fn caller(&self) -> Arc<dyn Caller> {
        match self {
            Connection::Sync(c) => c.caller(),
            Connection::Async(c) => c.caller(),
        }
    }

    pub fn cancellation(&self) -> CancellationToken {
        match self {
            Connection::Sync(c) => c.cancellation(),
            Connection::Async(c) => c.cancellation(),
        }
    }

    /// Blocks until the client disconnects (Ok) or the connection fails.
    pub fn run(&self) -> Result<(), TransportError> {
        match self {
            Connection::Sync(c) => c.run(),
            Connection::Async(c) => c.run(),
        }
    }
}

/// Convenience for the CLI: accept on stdio or `pipe_path`, then run until the client disconnects.
/// `make_handler` receives the caller for filesystem callbacks before the connection exists.
pub fn serve(
    pipe_path: Option<&Path>,
    options: ServeOptions,
    make_handler: impl FnOnce(Arc<dyn Caller>) -> Arc<dyn Handler>,
) -> Result<(), TransportError> {
    let late = LateCaller::new();
    let handler = make_handler(late.clone());
    let (_listener, stream) = match pipe_path {
        Some(path) => {
            let listener = PipeListener::bind(path)
                .map_err(|e| TransportError::Io(std::io::Error::new(e.kind(), format!("failed to create pipe transport: {e}"))))?;
            let stream = listener
                .accept()
                .map_err(|e| TransportError::Io(std::io::Error::new(e.kind(), format!("failed to accept connection: {e}"))))?;
            (Some(listener), stream)
        }
        None => (None, Stream::stdio()),
    };
    let conn = Connection::new(stream, handler, options, Some(&late));
    conn.run()
}
