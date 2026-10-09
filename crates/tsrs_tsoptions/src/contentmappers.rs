// The definitions half of Go `internal/contentmapper` (contentmapper.go, and OptionPathSegment from host.go) and
// tsoptions/contentmappers.go. Go's tsoptions imports contentmapper; here the definitions live in tsoptions and the
// mapper host crate imports them (notes/contentmappers.md, "Crate layout").
//
// A mapper is declared in tsconfig (Definition), its implementation is described by fields in its npm
// package's package.json (Manifest), and the two are combined once the package is resolved (Mapper).

use tsrs_ast::{new_compiler_diagnostic, Diagnostic};
use tsrs_core::collections::{new_ordered_map_with_size_hint, OrderedMap};
use tsrs_core::json;
use tsrs_core::tspath;
use tsrs_core::{CompilerOptions, ModuleResolutionKind, RESOLUTION_MODE_NONE, P};
use tsrs_diagnostics as diagnostics;
use tsrs_module::packagejson;

use crate::gojson;
use crate::tsconfigparsing::{ParseConfigHost, ResolverHost};

// Definition is a content mapper as declared in a tsconfig's "contentMappers": the npm package that
// implements the mapper and the otherwise unsupported file extensions it registers.
// contentmapper.go:30
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Definition {
    pub package: String,
    pub extensions: Vec<String>,
    // json.Value: the raw JSON text, empty when absent.
    pub options: String,
}

// Manifest is the content-mapper information read from a package's package.json: its name and version
// (which form the mapper's identity), the argv used to run it, and the compiler options it declares it
// depends on.
// contentmapper.go:39
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub exec: Vec<String>,
    pub compiler_options: Vec<String>,
    pub dynamic_config: bool,
}

// Mapper is a resolved content mapper: its tsconfig Definition combined with the Manifest resolved from
// the package's package.json, plus the package directory used as the mapper's working directory.
// contentmapper.go:49
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapper {
    pub definition: Definition,
    pub manifest: Manifest,
    // PackageDirectory is the real path directory returned by package resolution for package-based mappers.
    pub package_directory: String,
    // ContributionID is provided by an LSP client extension for inferred project content mappers.
    pub contribution_id: String,
}

// contentmapper.go:58
pub(crate) const SUPPORTED_VIRTUAL_EXTENSIONS: [&str; 9] = [".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts", ".json"];

// contentmapper.go:62
pub fn is_supported_virtual_extension(extension: &str) -> bool {
    SUPPORTED_VIRTUAL_EXTENSIONS.contains(&extension)
}

impl Mapper {
    // DiagnosticName returns the best available user-facing name, including when manifest resolution failed.
    // contentmapper.go:67
    pub fn diagnostic_name(&self) -> &str {
        if !self.manifest.name.is_empty() {
            return &self.manifest.name;
        }
        if !self.definition.package.is_empty() {
            return &self.definition.package;
        }
        &self.contribution_id
    }

    // Identity returns the mapper's "name@version" identity, or just the name when it declares no version,
    // or an empty string when the mapper has not been resolved to a name.
    // contentmapper.go:80
    pub fn identity(&self) -> String {
        if !self.contribution_id.is_empty() {
            return format!("{} ({})", self.contribution_id, self.manifest_identity());
        }
        self.manifest_identity()
    }

    // contentmapper.go:87
    pub(crate) fn manifest_identity(&self) -> String {
        if self.manifest.name.is_empty() {
            return String::new();
        }
        if self.manifest.version.is_empty() {
            return self.manifest.name.clone();
        }
        format!("{}@{}", self.manifest.name, self.manifest.version)
    }

    // TransformIdentity returns a fingerprint of everything besides a file's content that determines the
    // output of transforming it with this mapper under the given options: the mapper's identity and the
    // values of the compiler options it declared it depends on. Folding it into a cache key means a change to
    // the mapper version or a relevant compiler option invalidates cached results. It is a pure function of
    // the mapper and options — the declared options come from the manifest, so it never starts the mapper
    // process.
    // contentmapper.go:104 (Go `xxh3.Uint128`: `Hi` is the high 64 bits, so `Bytes()` is `to_be_bytes()`.)
    pub fn transform_identity(&self, options: Option<&CompilerOptions>) -> u128 {
        // Go discards the error, which marshaling these option types never returns.
        let declared = self.marshal_declared_options(options).unwrap_or_default();
        // Go `json.Marshal(declared)`: an object in declared order holding the raw JSON values as marshaled.
        let mut options_json = String::from("{");
        for (i, (name, raw)) in declared.iter().enumerate() {
            if i > 0 {
                options_json.push(',');
            }
            json::write_compact_string(&mut options_json, name);
            options_json.push(':');
            options_json.push_str(raw);
        }
        options_json.push('}');
        let identity = self.identity();
        let mut buf = Vec::with_capacity(identity.len() + 2 + self.definition.options.len() + options_json.len());
        buf.extend_from_slice(identity.as_bytes());
        buf.push(0);
        buf.extend_from_slice(self.definition.options.as_bytes());
        buf.push(0);
        buf.extend_from_slice(options_json.as_bytes());
        xxhash_rust::xxh3::xxh3_128(&buf)
    }

    // MarshalDeclaredOptions marshals just the compiler options this mapper declared it depends on, in the
    // declared order, skipping any that are unset. Marshaling only the declared fields avoids serializing the
    // whole CompilerOptions when a mapper depends on few options (or none).
    // contentmapper.go:119 (Go finds each field by its json tag through reflection, `compilerOptionFields`;
    // `gojson::compiler_option_to_go_json` does that lookup and the marshaling. The values are raw JSON text.)
    pub fn marshal_declared_options(&self, options: Option<&CompilerOptions>) -> Result<OrderedMap<String, String>, String> {
        let mut out = new_ordered_map_with_size_hint(self.manifest.compiler_options.len());
        let Some(options) = options else {
            return Ok(out);
        };
        if self.manifest.compiler_options.is_empty() {
            return Ok(out);
        }
        for name in &self.manifest.compiler_options {
            let Some(field) = gojson::compiler_option_to_go_json(options, name) else {
                continue;
            };
            let Some(value) = field else {
                continue;
            };
            let raw = json::marshal(&value)?;
            out.insert(name.clone(), raw);
        }
        Ok(out)
    }
}

// host.go:209
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptionPathSegment {
    pub property: String,
    pub index: usize,
    pub is_index: bool,
}

// resolveContentMapperManifest locates packageName in node_modules (walking up from the directory of
// containingFile via node module resolution) and reads its package.json to produce the mapper's manifest
// and package directory. It never executes the package. On failure it returns a diagnostic describing why
// the mapper could not be resolved; on success the diagnostic is nil.
// contentmappers.go:17
pub(crate) fn resolve_content_mapper_manifest(
    host: &'static dyn ParseConfigHost,
    containing_file: &str,
    package_name: &str,
) -> (Manifest, String, Option<P<Diagnostic>>) {
    let resolver_host: &'static ResolverHost = P::new(ResolverHost { host }).get();
    let resolver = tsrs_module::new_resolver(tsrs_module::ResolverOptions::new(
        resolver_host,
        P::new(CompilerOptions { module_resolution: ModuleResolutionKind::Bundler, ..Default::default() }),
    ));
    let resolved = resolver.resolve_package_directory(package_name, containing_file, RESOLUTION_MODE_NONE, None);
    let Some(resolved) = resolved.filter(|resolved| !resolved.resolved_file_name.is_empty()) else {
        return (
            Manifest::default(),
            String::new(),
            Some(new_compiler_diagnostic(&diagnostics::The_content_mapper_package_0_could_not_be_resolved, &[&package_name])),
        );
    };
    let package_directory = resolved.resolved_file_name.to_string();

    let package_json_path = tspath::combine_paths(&package_directory, &["package.json"]);
    let Some(contents) = host.fs().read_file(&package_json_path) else {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(&diagnostics::The_content_mapper_package_0_could_not_be_resolved, &[&package_name])),
        );
    };
    let fields = match packagejson::parse(&contents) {
        Ok(fields) => fields,
        Err(_) => {
            return (
                Manifest::default(),
                package_directory,
                Some(new_compiler_diagnostic(
                    &diagnostics::The_package_json_of_the_content_mapper_package_0_could_not_be_parsed,
                    &[&package_name],
                )),
            );
        }
    };
    // Go `name, _ := fields.Name.GetValue()`: the decoded value whether or not it was valid.
    let name = fields.name.value.clone();
    if name.is_empty() {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                &diagnostics::The_package_json_of_the_content_mapper_package_0_does_not_specify_a_name,
                &[&package_name],
            )),
        );
    }
    let version = fields.version.value.clone();

    // A content mapper package must declare how to run it: a "typescript.contentMapper" object with a non-empty
    // "exec" array of strings.
    let Some(cm) = fields.content_mapper.get_value() else {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                &diagnostics::The_package_json_of_the_content_mapper_package_0_does_not_declare_a_typescript_contentMapper_object,
                &[&package_name],
            )),
        );
    };
    let exec = match cm.exec.get_value() {
        Some(exec) if !exec.is_empty() => exec.clone(),
        _ => {
            return (
                Manifest::default(),
                package_directory,
                Some(new_compiler_diagnostic(
                    &diagnostics::The_typescript_contentMapper_exec_of_the_content_mapper_package_0_must_be_a_non_empty_array_of_strings,
                    &[&package_name],
                )),
            );
        }
    };
    let compiler_options = cm.compiler_options.value.clone();
    let dynamic_config = cm.dynamic_config.value;
    (Manifest { name, version, exec, compiler_options, dynamic_config }, package_directory, None)
}
