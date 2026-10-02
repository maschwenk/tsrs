// Go internal/project (+ dirty, logging, background) (docs/LSP.md).

pub mod background;
pub mod dirty;
pub mod logging;

mod client;
mod extendedconfigcache;
mod filechange;
mod overlayfs;
mod ownercache;
mod parsecache;
mod programcounter;
mod refcountcache;
mod snapshotfs;
mod watch;

pub use client::*;
pub use extendedconfigcache::*;
pub use filechange::*;
pub use overlayfs::*;
pub use ownercache::*;
pub use parsecache::*;
pub use refcountcache::*;
pub use snapshotfs::*;
pub use watch::*;

mod ata;
mod autoimport;
mod checkerpool;
mod compilerhost;
mod configfileregistry;
mod configfileregistrybuilder;
mod memregions;
mod project;
mod projectcollection;
mod projectcollectionbuilder;
mod session;
mod snapshot;
mod snapshothost;

pub use ata::*;
pub use checkerpool::*;
pub use configfileregistry::*;
pub use project::*;
pub use projectcollection::*;
pub use projectcollectionbuilder::ProjectCollectionBuilder;
pub use session::*;
pub use snapshot::*;
pub use snapshothost::*;

#[cfg(test)]
mod projecttestutil;
#[cfg(test)]
mod session_test;
#[cfg(test)]
mod projectcollectionbuilder_test;
