// contentmapper_test.go: program construction with content-mapped files, through a fake content mapper project.

use std::sync::Arc;

use tsrs_ast::{Diagnostic, MappedDiagnosticDirective, MappedDiagnosticDirectivePolicy};
use tsrs_contentmapper::{
    self as contentmapper, Definition, Manifest, Mapper, MappedResult, OptionDiagnostic, ProjectError, ProjectErrorKind, Request,
    TransformErrorKind, TransformResultFiles,
};
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, ResolutionMode, TextRange, Tristate, P};
use tsrs_diagnostics as diagnostics;
use tsrs_spanmap::{Feature, Kind, Segment};
use tsrs_tsoptions::{new_parsed_command_line, ParsedCommandLine};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::{new_compiler_host, new_program, Context, Program, ProgramOptions};

type transformFn = dyn Fn(&str, &str) -> Result<TransformResultFiles, contentmapper::Error> + Send + Sync;

// contentmapper_test.go:22
struct fakeContentMapperHost {
    transform: Arc<transformFn>,
}

impl contentmapper::Project for fakeContentMapperHost {
    fn refresh(&self) -> Result<(), contentmapper::Error> {
        Ok(())
    }
    fn identities(&self) -> Result<Vec<String>, contentmapper::Error> {
        Ok(Vec::new())
    }
    fn identity(&self, _mapper: &Mapper) -> Result<String, contentmapper::Error> {
        Ok("test".to_string())
    }
    fn watched_files(&self) -> Result<Vec<String>, contentmapper::Error> {
        Ok(Vec::new())
    }
    fn diagnostics(&self) -> Vec<OptionDiagnostic> {
        Vec::new()
    }
    fn close(&self) -> Result<(), contentmapper::Error> {
        Ok(())
    }
    fn transform(&self, _mapper: &Mapper, request: Request<'_>) -> Result<TransformResultFiles, contentmapper::Error> {
        (self.transform)(request.file_name, request.content)
    }
}

fn fake_host(
    transform: impl Fn(&str, &str) -> Result<TransformResultFiles, contentmapper::Error> + Send + Sync + 'static,
) -> Arc<dyn contentmapper::Project> {
    Arc::new(fakeContentMapperHost { transform: Arc::new(transform) })
}

fn options(module: ModuleKind, module_resolution: ModuleResolutionKind) -> CompilerOptions {
    CompilerOptions { skip_lib_check: Tristate::True, module, module_resolution, ..Default::default() }
}

// contentmapper_test.go:37
fn new_content_mapper_program(
    content_mapper_project: Arc<dyn contentmapper::Project>,
    files: &[(&str, &str)],
    root_files: &[&str],
) -> Option<&'static Program> {
    new_content_mapper_program_with_options(
        content_mapper_project,
        files,
        root_files,
        options(ModuleKind::ESNext, ModuleResolutionKind::Bundler),
    )
}

// contentmapper_test.go:45 (None is Go's t.Skip when the bundled files are not embedded.)
fn new_content_mapper_program_with_options(
    content_mapper_project: Arc<dyn contentmapper::Project>,
    files: &[(&str, &str)],
    root_files: &[&str],
    options: CompilerOptions,
) -> Option<&'static Program> {
    if !bundled::EMBEDDED {
        return None;
    }
    let fs: Arc<dyn FS> =
        Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), false /*useCaseSensitiveFileNames*/)));

    let mut config: ParsedCommandLine = new_parsed_command_line(
        P::new(options),
        root_files.iter().map(|f| f.to_string()).collect(),
        Vec::new(),
        Default::default(),
    );
    config.parsed_config.content_mappers = vec![Mapper {
        definition: Definition { package: "vue".to_string(), extensions: vec![".vue".to_string()], options: String::new() },
        manifest: Manifest { name: "vue-mapper".to_string(), version: "1.0.0".to_string(), ..Default::default() },
        ..Default::default()
    }];
    let mut opts = ProgramOptions::new(
        P::new(config),
        new_compiler_host("/src", fs, &bundled::lib_path(), None, None, Some(content_mapper_project)),
    );
    // Load files on the calling goroutine for deterministic diagnostics ordering.
    opts.single_threaded = Tristate::True;
    Some(new_program(opts))
}

fn result(text: &str, virtual_extension: &str, mappings: P<tsrs_spanmap::SpanMap>) -> TransformResultFiles {
    TransformResultFiles {
        text: text.to_string(),
        virtual_extension: virtual_extension.to_string(),
        mappings: Some(mappings),
        ..Default::default()
    }
}

// contentmapper_test.go:73. The mapper's virtual extension decides the module format: a `.mts` output is ESM even
// though `.vue` itself implies none.
#[test]
fn test_content_mapper_virtual_extension_sets_implied_node_format() {
    let Some(program) = new_content_mapper_program_with_options(
        fake_host(|_, _| Ok(result("export {};", ".mts", tsrs_spanmap::new(&[])))),
        &[("/src/Component.vue", "<template />")],
        &["/src/Component.vue"],
        options(ModuleKind::NodeNext, ModuleResolutionKind::NodeNext),
    ) else {
        return;
    };

    let file = program.get_source_file("/src/Component.vue").expect("mapped file");
    assert_eq!(program.get_source_file_meta_data(&file.path()).implied_node_format, ResolutionMode::ESM);
}

// contentmapper_test.go:95. A mapped diagnostic directive suppresses the file's diagnostics, but a missing global
// type that checking the file discovers is still reported once with the file's incremental diagnostics; an
// `expect` directive that suppressed nothing in the file reports the mapper's unused-directive diagnostic.
#[test]
fn test_content_mapper_directives_preserve_incremental_globals() {
    for policy in [MappedDiagnosticDirectivePolicy::Ignore, MappedDiagnosticDirectivePolicy::Expect] {
        const TEXT: &str = "export function values() { function* generator() { yield 1; } }";
        let Some(program) = new_content_mapper_program_with_options(
            fake_host(move |_, content| {
                // The virtual source is unchanged; the directive covers the entire file, including offset zero.
                let len = content.len() as i32;
                Ok(TransformResultFiles {
                    text: content.to_string(),
                    virtual_extension: ".ts".to_string(),
                    mappings: Some(tsrs_spanmap::new(&[Segment {
                        original_end: len,
                        virtual_end: len,
                        kind: Kind::Verbatim,
                        features: Feature::All,
                        ..Default::default()
                    }])),
                    diagnostic_directives: vec![MappedDiagnosticDirective {
                        virtual_range: TextRange::new(0, len),
                        original_range: TextRange::new(0, len),
                        policy,
                        source: "vue",
                        unused_code: 2578,
                        unused_message_text: "Unused mapped expect directive.",
                    }],
                    ..Default::default()
                })
            }),
            &[("/src/Component.vue", TEXT)],
            &["/src/Component.vue"],
            CompilerOptions {
                lib: Some(vec!["lib.es5.d.ts".to_string()]),
                ..options(ModuleKind::ESNext, ModuleResolutionKind::Bundler)
            },
        ) else {
            return;
        };
        let ctx = Context::default();
        assert_eq!(program.get_global_diagnostics(&ctx).len(), 0);
        let file = program.get_source_file("/src/Component.vue").unwrap();
        let diags = program.get_semantic_diagnostics_for_incremental(&ctx, &[file]).remove(&file).unwrap();
        let (mut globals, mut unused) = (0, 0);
        for diag in diags {
            if diag.file().is_none() {
                assert_eq!(diag.code(), diagnostics::Cannot_find_global_type_0.code());
                assert_eq!(diag.message_args()[0], "IterableIterator");
                globals += 1;
            } else {
                assert_eq!(diag.file(), Some(file));
                assert_eq!(diag.source(), "vue");
                assert_eq!(diag.code(), 2578);
                unused += 1;
            }
        }
        assert_eq!(globals, 1, "{policy:?}");
        assert_eq!(unused, if policy == MappedDiagnosticDirectivePolicy::Expect { 1 } else { 0 }, "{policy:?}");
    }
}

fn supplemental_host() -> Arc<dyn contentmapper::Project> {
    fake_host(|_, _| {
        Ok(TransformResultFiles {
            supplemental: vec![MappedResult {
                text: "export {};".to_string(),
                virtual_extension: ".mts".to_string(),
                mappings: Some(tsrs_spanmap::new(&[])),
                diagnostic_directives: Vec::new(),
            }],
            ..result("export {};", ".ts", tsrs_spanmap::new(&[]))
        })
    })
}

fn composite_options() -> CompilerOptions {
    CompilerOptions { composite: Tristate::True, ..options(ModuleKind::ESNext, ModuleResolutionKind::Bundler) }
}

fn is_unlisted_file_diagnostic(diagnostic: &P<Diagnostic>) -> bool {
    diagnostic.code()
        == diagnostics::File_0_is_not_listed_within_the_file_list_of_project_1_Projects_must_list_all_files_or_use_an_include_pattern.code()
}

// contentmapper_test.go:162 "listed canonical root". A composite project lists the canonical `.vue` file; its
// supplemental output is part of that root and must not be reported as an unlisted file.
#[test]
fn test_composite_project_content_mapper_supplemental_roots_listed_canonical_root() {
    let Some(program) =
        new_content_mapper_program_with_options(supplemental_host(), &[("/src/Component.vue", "<template />")], &["/src/Component.vue"], composite_options())
    else {
        return;
    };

    let program_diagnostics = collect_content_mapper_diagnostics(program);
    let has_unlisted_file_diagnostic = program_diagnostics.iter().any(is_unlisted_file_diagnostic);
    assert!(!has_unlisted_file_diagnostic, "supplemental output should be covered by its listed canonical root: {program_diagnostics:?}");
}

// contentmapper_test.go:180 "imported canonical file". A canonical file reached only through an import is not
// listed, and neither is its supplemental output: both are reported.
#[test]
fn test_composite_project_content_mapper_supplemental_roots_imported_canonical_file() {
    let Some(program) = new_content_mapper_program_with_options(
        supplemental_host(),
        &[("/src/index.ts", r#"import "./Component.vue";"#), ("/src/Component.vue", "<template />")],
        &["/src/index.ts"],
        composite_options(),
    ) else {
        return;
    };

    let unlisted_file_diagnostic_count = collect_content_mapper_diagnostics(program).iter().filter(|d| is_unlisted_file_diagnostic(d)).count();
    assert_eq!(unlisted_file_diagnostic_count, 2);
}

// contentmapper_test.go:200
fn collect_content_mapper_diagnostics(program: &'static Program) -> Vec<P<Diagnostic>> {
    let ctx = Context::default();
    let mut all = program.get_syntactic_diagnostics(&ctx, None);
    all.extend(program.get_semantic_diagnostics(&ctx, None));
    all.extend(program.get_program_diagnostics());
    all
}

// contentmapper_test.go:209. Overlapping mapping segments are a mapper bug: the import of the mapped file gets
// the dedicated mapping diagnostic instead of a parse of the bad output.
#[test]
fn test_content_mapper_invalid_mappings() {
    const TRANSFORMED: &str = "export const x = 1;\n";
    const ORIGINAL: &str = "<template>x</template>\n";
    let Some(program) = new_content_mapper_program(
        fake_host(|_, _| {
            let mappings = tsrs_spanmap::new(&[
                Segment { virtual_start: 0, virtual_end: 10, original_start: 0, original_end: 0, kind: Kind::Atom, ..Default::default() },
                Segment {
                    virtual_start: 5,
                    virtual_end: TRANSFORMED.len() as i32,
                    original_start: 0,
                    original_end: 0,
                    kind: Kind::Atom,
                    ..Default::default()
                },
            ]);
            Ok(result(TRANSFORMED, ".ts", mappings))
        }),
        &[("/src/app.ts", r#"import "./Component.vue";"#), ("/src/Component.vue", ORIGINAL)],
        &["/src/app.ts"],
    ) else {
        return;
    };
    let program_diagnostics = collect_content_mapper_diagnostics(program);
    let found = program_diagnostics.iter().any(|diagnostic| {
        diagnostic.code() == diagnostics::The_content_mapper_0_produced_overlapping_or_out_of_order_position_mappings_near_virtual_offset_1.code()
    });
    assert!(found, "expected an invalid mapping diagnostic, got: {program_diagnostics:?}");
}

// contentmapper_test.go:241 "successful synthesized empty file". An empty original whose transform succeeds is a
// mapped file, not a failure stub.
#[test]
fn test_content_mapper_source_file_state_successful_synthesized_empty_file() {
    let Some(program) = new_content_mapper_program(
        fake_host(|_, _| Ok(result("export {};", ".ts", tsrs_spanmap::new(&[])))),
        &[("/src/empty.vue", "")],
        &["/src/empty.vue"],
    ) else {
        return;
    };
    let file = program.get_source_file("/src/empty.vue").expect("mapped file");
    assert_eq!(file.original_text(), "");
    assert_eq!(file.content_mapper(), "vue-mapper@1.0.0");
    assert!(!file.is_content_mapper_failure_stub());
}

// contentmapper_test.go:255 "failed transform". The file stays in the program as an empty stub that keeps the
// original text and the mapper's identity.
#[test]
fn test_content_mapper_source_file_state_failed_transform() {
    let Some(program) = new_content_mapper_program(
        fake_host(|_, _| Err(contentmapper::Error::Other("failed".to_string()))),
        &[("/src/fail.vue", "original")],
        &["/src/fail.vue"],
    ) else {
        return;
    };
    let file = program.get_source_file("/src/fail.vue").expect("stub file");
    assert_eq!(file.original_text(), "original");
    assert_eq!(file.content_mapper(), "vue-mapper@1.0.0");
    assert!(file.is_content_mapper_failure_stub());
}

// contentmapper_test.go:269 "project error is localized". A project error behind a transform error is reported
// with its own message in the failure's chain, not as a generic transform failure.
#[test]
fn test_content_mapper_source_file_state_project_error_is_localized() {
    let Some(program) = new_content_mapper_program(
        fake_host(|_, _| {
            Err(contentmapper::new_transform_error(
                TransformErrorKind::Project,
                Some(ProjectError { kind: ProjectErrorKind::MalformedResponse }.into()),
            )
            .into())
        }),
        &[("/src/fail.vue", "original")],
        &["/src/fail.vue"],
    ) else {
        return;
    };
    let program_diagnostics = collect_content_mapper_diagnostics(program);
    let found = program_diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message_chain()
            .iter()
            .any(|message| message.code() == diagnostics::The_content_mapper_returned_a_project_response_that_could_not_be_decoded.code())
    });
    assert!(found, "expected a localized project response diagnostic, got: {program_diagnostics:?}");
}

// contentmapper_test.go:288. Each invalid project response has its own message (code and text).
#[test]
fn test_content_mapper_project_error_diagnostics() {
    for (kind, code, message_text) in [
        (
            ProjectErrorKind::MissingConfigIdentity,
            diagnostics::The_content_mapper_did_not_return_configIdentity_which_is_required_when_the_content_mapper_has_dynamicConfig_Colon_true_in_its_package_json.code(),
            r#"The content mapper did not return 'configIdentity', which is required when the content mapper has '"dynamicConfig": true' in its package.json."#,
        ),
        (
            ProjectErrorKind::UnexpectedConfigIdentity,
            diagnostics::The_content_mapper_returned_configIdentity_which_is_only_allowed_when_it_declares_dynamicConfig_Colon_true_in_its_package_json.code(),
            r#"The content mapper returned 'configIdentity', which is only allowed when it declares '"dynamicConfig": true' in its package.json."#,
        ),
        (
            ProjectErrorKind::UnexpectedWatchedFiles,
            diagnostics::The_content_mapper_returned_watchedFiles_which_is_only_allowed_when_it_declares_dynamicConfig_Colon_true_in_its_package_json.code(),
            r#"The content mapper returned 'watchedFiles', which is only allowed when it declares '"dynamicConfig": true' in its package.json."#,
        ),
    ] {
        let message = crate::content_mapper_project_error_diagnostic(&ProjectError { kind }.into());
        assert_eq!(message.code(), code);
        assert_eq!(message.localize(&[]), message_text);
    }
}
