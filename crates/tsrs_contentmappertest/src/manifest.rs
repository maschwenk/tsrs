use crate::protocol::quote;
use crate::registry::{DYNAMIC_VERBATIM_MAPPER, TRANSFORMING_MAPPER};

// manifest.go:5
pub const PACKAGE_NAME: &str = "mapper";

// PackageJSON returns a package manifest selecting the requested mapper.
// manifest.go:8
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
        quote(PACKAGE_NAME),
        quote(mapper),
        compiler_options,
        dynamic_config
    )
}
