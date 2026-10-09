// Go `internal/contentmapper`, the host half (notes/contentmappers.md): host.go (the errors, the transform result
// types, and the Project and Host interfaces), hostimpl.go (the process host: protocol types, project leases,
// response decoding and position normalization), transform.go (parsing a transform result into source files), and
// process.rs, the production Spawner (Go cmd/tsc/sys.go spawnProcess). The definitions half of the package (Mapper,
// Definition, Manifest, IsSupportedVirtualExtension, OptionPathSegment) is tsrs_tsoptions::contentmappers; it is
// re-exported here so that `contentmapper::Mapper` reads as in Go.

mod host;
mod hostimpl;
mod process;
mod transform;

pub use host::*;
pub use hostimpl::*;
pub use process::*;
pub use transform::*;
pub use tsrs_tsoptions::{is_supported_virtual_extension, Definition, Manifest, Mapper, OptionPathSegment};

#[cfg(test)]
mod host_test;
#[cfg(test)]
mod process_test;
#[cfg(test)]
mod transform_test;
