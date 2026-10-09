use std::sync::Arc;

use crate::component::componentHandler;
use crate::diagnostic_code_collision::diagnosticCodeCollisionHandler;
use crate::duplicate::duplicateHandler;
use crate::duplicate_projection::duplicateProjectionHandler;
use crate::dynamic_verbatim::{dynamicVerbatimHandler, ProjectLifecycle};
use crate::editing::{prefixedSupplementalHandler, unmappedFoldingHandler};
use crate::failing::failingHandler;
use crate::hoisting::hoistingHandler;
use crate::lisp::lispHandler;
use crate::protocol::testHandler;
use crate::supplemental::supplementalHandler;
use crate::supplemental_diagnostics::supplementalDiagnosticsHandler;
use crate::supplemental_globals::supplementalGlobalsHandler;
use crate::supplemental_module::supplementalModuleHandler;
use crate::synthesizing::synthesizingHandler;
use crate::transforming::Handler;
use crate::verbatim::{moduleVerbatimHandler, verbatimHandler};

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

// registry.go:31
type handlerConstructor = fn(Option<Arc<ProjectLifecycle>>) -> Arc<dyn testHandler>;

// registry.go:33 (Go's map; looked up by name.)
static MAPPER_HANDLERS: [(&str, handlerConstructor); 18] = [
    (TRANSFORMING_MAPPER, |_| Arc::new(Handler::default())),
    (VERBATIM_MAPPER, |_| Arc::new(verbatimHandler)),
    (MODULE_VERBATIM_MAPPER, |_| Arc::new(moduleVerbatimHandler)),
    (DYNAMIC_VERBATIM_MAPPER, |lifecycle| Arc::new(dynamicVerbatimHandler { lifecycle })),
    (DIAGNOSTIC_CODE_COLLISION_MAPPER, |_| Arc::new(diagnosticCodeCollisionHandler)),
    (FAILING_MAPPER, |_| Arc::new(failingHandler)),
    (SYNTHESIZING_MAPPER, |_| Arc::new(synthesizingHandler)),
    (COMPONENT_MAPPER, |_| Arc::new(componentHandler)),
    (DUPLICATE_MAPPER, |_| Arc::new(duplicateHandler)),
    (LISP_MAPPER, |_| Arc::new(lispHandler)),
    (SUPPLEMENTAL_MAPPER, |_| Arc::new(supplementalHandler)),
    (SUPPLEMENTAL_DIAGNOSTICS_MAPPER, |_| Arc::new(supplementalDiagnosticsHandler)),
    (SUPPLEMENTAL_GLOBALS_MAPPER, |_| Arc::new(supplementalGlobalsHandler)),
    (SUPPLEMENTAL_MODULE_MAPPER, |_| Arc::new(supplementalModuleHandler)),
    (PREFIXED_SUPPLEMENTAL_MAPPER, |_| Arc::new(prefixedSupplementalHandler)),
    (UNMAPPED_FOLDING_MAPPER, |_| Arc::new(unmappedFoldingHandler)),
    (HOISTING_MAPPER, |_| Arc::new(hoistingHandler)),
    (DUPLICATE_PROJECTION_MAPPER, |_| Arc::new(duplicateProjectionHandler)),
];

// registry.go:54
pub(crate) fn handler_for_mapper(command: &[String], lifecycle: Option<Arc<ProjectLifecycle>>) -> Result<Arc<dyn testHandler>, String> {
    if command.is_empty() {
        return Err("contentmappertest: empty mapper command".to_string());
    }
    let Some((_, constructor)) = MAPPER_HANDLERS.iter().find(|(name, _)| *name == command[0]) else {
        // fmt's %v of a []string.
        return Err(format!("contentmappertest: unknown mapper command [{}]", command.join(" ")));
    };
    Ok(constructor(lifecycle))
}
