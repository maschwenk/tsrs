// Go package internal/jsonrpc.

mod baseproto;
#[expect(clippy::module_inception, reason = "The module path preserves the upstream Go package and file layout.")]
mod jsonrpc;

pub use self::baseproto::*;
pub use self::jsonrpc::*;

#[cfg(test)]
mod baseproto_test;
#[cfg(test)]
mod jsonrpc_test;
