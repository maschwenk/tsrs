use tsrs_ast::Diagnostic;
use tsrs_core::P;

use crate::tsconfigparsing::ParseConfigHost;

// The subset of Go `contentmapper.Definition` / `Manifest` / `Mapper` that config parsing produces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Definition {
    pub package: String,
    pub extensions: Vec<String>,
    // json.Value
    pub options: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub exec: Vec<String>,
    pub compiler_options: Vec<String>,
    pub dynamic_config: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapper {
    pub definition: Definition,
    pub manifest: Manifest,
    // PackageDirectory is the real path directory returned by package resolution for package-based mappers.
    pub package_directory: String,
    // ContributionID is provided by an LSP client extension for inferred project content mappers.
    pub contribution_id: String,
}

// resolveContentMapperManifest locates packageName in node_modules (walking up from the directory of
// containingFile via node module resolution) and reads its package.json to produce the mapper's manifest
// and package directory. It never executes the package. On failure it returns a diagnostic describing why
// the mapper could not be resolved; on success the diagnostic is nil.
//
// Content mappers only run with --runExternalCode, which executes third-party programs; this port does not
// support them (only their tsconfig validation is ported).
pub(crate) fn resolve_content_mapper_manifest(
    _host: &'static dyn ParseConfigHost,
    _containing_file: &str,
    _package_name: &str,
) -> (Manifest, String, Option<P<Diagnostic>>) {
    #[expect(clippy::unimplemented, reason = "content mappers are not ported; only their tsconfig validation is")]
    {
        unimplemented!("content mappers (--runExternalCode)")
    }
}
