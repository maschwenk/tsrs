// Go's testutil/contentmappertest, which the fourslash tests reference: the tsrs_contentmappertest crate.

use crate::contentmapper::Spawner;

pub use tsrs_contentmappertest::{
    package_json, COMPONENT_MAPPER, DIAGNOSTIC_CODE_COLLISION_MAPPER, DUPLICATE_MAPPER, DUPLICATE_PROJECTION_MAPPER,
    DYNAMIC_VERBATIM_MAPPER, FAILING_MAPPER, HOISTING_MAPPER, LISP_MAPPER, MODULE_VERBATIM_MAPPER, PACKAGE_NAME,
    PREFIXED_SUPPLEMENTAL_MAPPER, SUPPLEMENTAL_DIAGNOSTICS_MAPPER, SUPPLEMENTAL_GLOBALS_MAPPER, SUPPLEMENTAL_MAPPER,
    SUPPLEMENTAL_MODULE_MAPPER, SYNTHESIZING_MAPPER, TRANSFORMING_MAPPER, UNMAPPED_FOLDING_MAPPER, VERBATIM_MAPPER,
};

// spawner.go:18 (Go's interface value is never nil here; Some is FourslashOptions' optional field.)
pub fn new_spawner() -> Option<Spawner> {
    Some(Spawner(tsrs_contentmappertest::new_spawner()))
}
