// The parts of Go's testutil/contentmappertest the tests reference (mapper names and the package manifest).
// The mappers themselves are not ported (content mappers are out of scope).

use crate::contentmapper::Spawner;

// manifest.go:5
pub const PACKAGE_NAME: &str = "mapper";

// manifest.go:8
// PackageJSON returns a package manifest selecting the requested mapper.
pub fn package_json(mapper: &str) -> String {
    let mut compiler_options = "";
    let mut dynamic_config = "";
    if mapper == TRANSFORMING_MAPPER {
        compiler_options = r#", "compilerOptions": ["target", "jsx"]"#;
    }
    if mapper == DYNAMIC_VERBATIM_MAPPER {
        dynamic_config = r#", "dynamicConfig": true"#;
    }
    format!(
        "{{\n\t\"name\": {},\n\t\"version\": \"1.0.0\",\n\t\"typescript\": {{ \"contentMapper\": {{ \"exec\": [{}]{}{} }} }}\n}}",
        crate::go::quote(PACKAGE_NAME),
        crate::go::quote(mapper),
        compiler_options,
        dynamic_config
    )
}

// registry.go:10
pub const TRANSFORMING_MAPPER: &str = "compiler-test-mapper";
pub const VERBATIM_MAPPER: &str = "verbatim-mapper";
pub const MODULE_VERBATIM_MAPPER: &str = "module-verbatim-mapper";
pub const DYNAMIC_VERBATIM_MAPPER: &str = "dynamic-verbatim-mapper";
pub const DIAGNOSTIC_CODE_COLLISION_MAPPER: &str = "diagnostic-code-collision-mapper";
pub const FAILING_MAPPER: &str = "failing-mapper";
pub const SYNTHESIZING_MAPPER: &str = "synthesizing-mapper";
pub const COMPONENT_MAPPER: &str = "component-mapper";
pub const DUPLICATE_MAPPER: &str = "duplicate-mapper";
pub const LISP_MAPPER: &str = "lisp-mapper";
pub const SUPPLEMENTAL_MAPPER: &str = "supplemental-mapper";
pub const SUPPLEMENTAL_DIAGNOSTICS_MAPPER: &str = "supplemental-diagnostics-mapper";
pub const SUPPLEMENTAL_GLOBALS_MAPPER: &str = "supplemental-globals-mapper";
pub const SUPPLEMENTAL_MODULE_MAPPER: &str = "supplemental-module-mapper";
pub const PREFIXED_SUPPLEMENTAL_MAPPER: &str = "prefixed-supplemental-mapper";
pub const UNMAPPED_FOLDING_MAPPER: &str = "unmapped-folding-mapper";
pub const HOISTING_MAPPER: &str = "hoisting-mapper";
pub const DUPLICATE_PROJECTION_MAPPER: &str = "duplicate-projection-mapper";

// spawner.go:18 (the IPC spawner of the in-process test mappers is not ported)
pub fn new_spawner() -> Option<Spawner> {
    Some(Spawner)
}
