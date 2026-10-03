// Port of Go's execute/build package (`tsc -b`), without watch mode. Go keeps it in its own package; here it is a
// module of tsrs_cli because it depends on the `tsc` module of this binary crate (Go's execute/tsc).

mod buildtask;
mod compilerhost;
mod host;
mod orchestrator;
mod parsecache;
mod uptodatestatus;

pub use orchestrator::{new_orchestrator, Options, Orchestrator, OrchestratorResult};
