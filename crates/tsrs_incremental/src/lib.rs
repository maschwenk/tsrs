// Port of Go's execute/incremental package: incremental programs, tsbuildinfo read/write and affected-file
// tracking.

mod affectedfileshandler;
mod buildinfo;
mod buildinfo_decode;
mod buildinfotosnapshot;
pub mod emit;
mod emitfileshandler;
mod host;
mod incremental;
mod program;
mod programtosnapshot;
mod referencemap;
mod snapshot;
mod snapshottobuildinfo;

pub use buildinfo::*;
pub use host::{create_host, get_m_time, Host};
pub use incremental::{new_build_info_reader, read_build_info_program, register_build_info_read_phases, BuildInfoReader};
pub use program::{new_program, Program, SemanticDiagnosticsState, SignatureUpdateKind, TestingData};
pub use snapshot::{compute_hash, get_file_emit_kind, FileEmitKind, FileInfo};
