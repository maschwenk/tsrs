// Port of Go's execute/build package (`tsc -b`), without watch mode. Go keeps it in its own package; here it is a
// module of tsrs_execute because it depends on the `tsc` module of this crate (Go's execute/tsc).

mod buildtask;
mod compilerhost;
mod host;
mod orchestrator;
mod parsecache;
mod uptodatestatus;

pub use orchestrator::{free_api_orchestrator, new_orchestrator, Options, Orchestrator, OrchestratorResult};
