//! Binary AST codec of the TypeScript native API: a port of the pinned `microsoft/TypeScript`
//! `tsc/internal/api/encoder` package (commit in the workspace `Cargo.toml`), byte-compatible with the pinned
//! JavaScript client (`packages/typescript/src/api/node`). See `README.md` for the contract and coverage.
//!
//! Portions are derived from the TypeScript project (Apache-2.0, see the repository `NOTICE`).

pub mod decoder;
pub mod encoder;
pub mod format;
pub mod handle;
mod generated;
pub mod msgpack;
pub mod positionmap;
pub mod stringtable;

pub use decoder::{decode_nodes, decode_source_file, DecodeError, DecodedNode};
pub use encoder::{
    build_node_index_table, encode_node, encode_source_file, set_source_file_id, set_source_file_lease, source_file_hash, EncodeError,
    NodeIndexTable,
};
pub use format::PROTOCOL_VERSION;
pub use handle::{node_handle, parse_node_handle, resolve_node_index, ParsedNodeHandle};
pub use positionmap::PositionMap;
