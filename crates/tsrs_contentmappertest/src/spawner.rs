use std::io::Write;
use std::sync::Arc;
use std::thread;

use tsrs_contentmapper::Spawner;
use tsrs_ipc::{self as ipc, Conn, Handler, ReadWriteCloser};

use crate::dynamic_verbatim::ProjectLifecycle;
use crate::protocol::staticProjectHandler;
use crate::registry::{handler_for_mapper, DYNAMIC_VERBATIM_MAPPER};
use crate::transforming::Handler as transformingHandler;

// The thread that serves an in-process mapper (Go's goroutine).
const SERVER_THREAD: &str = "contentmappertest-server";

// Serve drives the transforming mapper over the connection until it closes or ctx is cancelled.
// spawner.go:13 (No context: until the connection ends.)
pub fn serve(rwc: ReadWriteCloser) -> Result<(), ipc::Error> {
    ipc::new_async_conn(rwc, Arc::new(staticProjectHandler { handler: Arc::new(transformingHandler::default()) })).run()
}

// NewSpawner returns an in-process spawner for the test mapper implementations.
// spawner.go:18
pub fn new_spawner() -> Arc<dyn Spawner> {
    Arc::new(spawner { lifecycle: None })
}

// NewSpawnerWithProjectLifecycle returns an in-process spawner that records project protocol calls.
// spawner.go:23
pub fn new_spawner_with_project_lifecycle(lifecycle: Arc<ProjectLifecycle>) -> Arc<dyn Spawner> {
    Arc::new(spawner { lifecycle: Some(lifecycle) })
}

// spawner.go:27
struct spawner {
    lifecycle: Option<Arc<ProjectLifecycle>>,
}

impl Spawner for spawner {
    // spawner.go:31 (Go's net.Pipe is tsrs_ipc's socket pair.)
    fn spawn(&self, command: &[String], _dir: &str, _stderr: Option<Box<dyn Write + Send>>) -> Result<ReadWriteCloser, String> {
        let handler = handler_for_mapper(command, self.lifecycle.clone())?;
        let handler: Arc<dyn Handler> =
            if command[0] != DYNAMIC_VERBATIM_MAPPER { Arc::new(staticProjectHandler { handler }) } else { handler };
        let (client, server) = ipc::pipe().map_err(|err| err.to_string())?;
        thread::Builder::new()
            .name(SERVER_THREAD.to_string())
            .spawn(move || {
                let _ = ipc::new_async_conn(server, handler).run();
            })
            .map_err(|err| err.to_string())?;
        Ok(client)
    }
}
