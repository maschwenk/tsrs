// Port of the type-check-only subset of Go's `execute/tsc` package.

mod compile;
mod diagnostics;
mod emit;
mod extendedconfigcache;
mod statistics;

pub use compile::*;
pub use diagnostics::*;
pub use emit::*;
pub use extendedconfigcache::*;
pub use statistics::*;
// The stream `System::spawn` returns (Go `io.ReadWriteCloser`).
pub use tsrs_ipc::ReadWriteCloser;
