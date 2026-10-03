// OWNED BY THE CHECKER LANE (mfs-cx/node-api-checker). Core only provides this placeholder so the
// crate compiles; the checker lane replaces the body (and may add `src/checker/**`).
//
// Contract (docs/NODE_API.md, "checker hook"):
// - `handle` receives every request whose `methods::METHODS` owner is `Owner::Checker`.
// - Return `None` only for methods the checker lane does not recognize; core turns that into an
//   explicit `ApiError::unsupported`. Never return a success value for unimplemented behavior.
// - Per-snapshot checker state (symbol/type/signature registries) lives in `CheckerSnapshotState`,
//   which core stores inside each registered snapshot and drops when the snapshot is released.

use crate::handler::{ApiResult, Response};
use crate::session::Session;
use tsrs_core::json::Value;

/// Per-snapshot registries owned by the checker lane. Core creates it with `Default` when a snapshot
/// is registered and drops it with the snapshot.
#[derive(Default)]
pub struct CheckerSnapshotState {}

pub(crate) fn handle(session: &Session, method: &str, params: &Value) -> Option<ApiResult<Response>> {
    let _ = (session, method, params);
    None
}
