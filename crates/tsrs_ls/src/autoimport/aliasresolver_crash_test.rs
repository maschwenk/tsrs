use std::sync::Arc;

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::Program;
use tsrs_core::context::Context;
use tsrs_core::tspath::Path;
use tsrs_core::{ScriptKind, EMPTY_COMPILER_OPTIONS, P};
use tsrs_module::packagejson::InfoCacheEntry;
use tsrs_module::{ResolutionHost, ResolverOptions};
use tsrs_vfs::{vfstest, FS};

use super::aliasresolver::new_alias_resolver;
use super::registry::{ProjectID, RegistryCloneHost};

// aliasresolver_crash_test.go:20
struct fakeCloneHost {
    fs: Box<dyn FS>,
}

impl ResolutionHost for fakeCloneHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        "/"
    }
}

impl RegistryCloneHost for fakeCloneHost {
    fn get_default_project(&self, _path: &Path) -> (Option<ProjectID>, Option<&'static Program>) {
        (None, None)
    }
    fn get_program_for_project(&self, _project_id: &ProjectID) -> Option<&'static Program> {
        None
    }
    fn get_package_json(&self, _file_name: &str) -> P<InfoCacheEntry> {
        // Go returns nil; nothing in this test asks for a package.json.
        panic!("unexpected GetPackageJson")
    }
    fn get_source_file(&self, _file_name: &str, _path: &Path) -> Option<P<SourceFile>> {
        None
    }
    fn dispose(&self) {}
}

// aliasresolver_crash_test.go:41
// Regression test for microsoft/typescript-go#4322.
//
// During auto-import export extraction, the checker is built on top of an
// aliasResolver standing in for a real program. This file has a type error, and
// extracting exports should still complete without crashing.
#[test]
fn test_alias_resolver_get_diagnostics_does_not_panic() {
    let file_name = "/pkg/index.ts";
    let text = "declare function f(arg: { a: string }): () => void;\nexport const x = f({ a: 1 });\n";

    let fs = vfstest::from_map([(file_name, text)], true /*useCaseSensitiveFileNames*/);
    let host: &'static fakeCloneHost = Box::leak(Box::new(fakeCloneHost { fs: Box::new(fs) }));

    let source_file = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: file_name.to_string(), path: Path(file_name.to_string()), ..Default::default() },
        text,
        ScriptKind::TS,
    );
    tsrs_binder::bind_source_file(source_file);

    let resolver = Box::leak(Box::new(tsrs_module::new_resolver(ResolverOptions::new(host, P::from_static(&*EMPTY_COMPILER_OPTIONS)))));
    let r = new_alias_resolver(vec![source_file], Default::default(), host, resolver, Arc::new(|f: &str| Path(f.to_string())), Box::new(|_, _| {}));
    let r = Box::leak(Box::new(r));

    let mut ch = tsrs_checker::new_checker(r);

    // Type-checking this file's diagnostics must not panic.
    ch.get_diagnostics_exported(&Context::background(), source_file);
}
