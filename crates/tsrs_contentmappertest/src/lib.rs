// Go `internal/testutil/contentmappertest`: realistic content mapper implementations used by tests. The spawner
// serves them in-process over an in-memory connection, selecting the implementation by the command the mapper
// package declares (`exec`), so that tests exercise the content mapper host and its JSON-RPC connection without a
// subprocess.

mod component;
mod diagnostic_code_collision;
mod duplicate;
mod duplicate_projection;
mod dynamic_verbatim;
mod editing;
mod failing;
mod hoisting;
mod lisp;
mod manifest;
mod protocol;
mod registry;
mod spawner;
mod supplemental;
mod supplemental_diagnostics;
mod supplemental_globals;
mod supplemental_module;
mod synthesizing;
mod transforming;
mod verbatim;

pub use dynamic_verbatim::ProjectLifecycle;
pub use manifest::*;
pub use registry::{
    COMPONENT_MAPPER, DIAGNOSTIC_CODE_COLLISION_MAPPER, DUPLICATE_MAPPER, DUPLICATE_PROJECTION_MAPPER, DYNAMIC_VERBATIM_MAPPER,
    FAILING_MAPPER, HOISTING_MAPPER, LISP_MAPPER, MODULE_VERBATIM_MAPPER, PREFIXED_SUPPLEMENTAL_MAPPER, SUPPLEMENTAL_DIAGNOSTICS_MAPPER,
    SUPPLEMENTAL_GLOBALS_MAPPER, SUPPLEMENTAL_MAPPER, SUPPLEMENTAL_MODULE_MAPPER, SYNTHESIZING_MAPPER, TRANSFORMING_MAPPER,
    UNMAPPED_FOLDING_MAPPER, VERBATIM_MAPPER,
};
pub use spawner::{new_spawner, new_spawner_with_project_lifecycle, serve};
pub use transforming::{Handler, DECLARED_OPTIONS};
