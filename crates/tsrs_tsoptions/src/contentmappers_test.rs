// tsoptions/contentmappers_test.go

use tsrs_core::collections::OrderedMap;
use tsrs_core::tspath::ComparePathsOptions;
use tsrs_core::{CompilerOptions, JsxEmit, ScriptTarget, Tristate, P};
use tsrs_diagnostics as diagnostics;

use crate::contentmappers::{resolve_content_mapper_manifest, Definition, Manifest, Mapper};
use crate::parsedcommandline::{new_parsed_command_line, ParsedCommandLine};
use crate::parsedoptions::ParsedOptions;
use crate::tsoptionstest::new_vfs_parse_config_host;

fn mapper(package: &str, extensions: &[&str]) -> Mapper {
    Mapper {
        definition: Definition {
            package: package.to_string(),
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// contentmappers_test.go:20 (Go compares the *Mapper pointers; here the elements of the command line's list.)
#[test]
fn test_get_content_mapper_for_file_name_uses_longest_extension() {
    let command_line = ParsedCommandLine {
        parsed_config: ParsedOptions { content_mappers: vec![mapper("z", &[".z"]), mapper("yz", &[".y.z"])], ..Default::default() },
        ..ParsedCommandLine::empty()
    };
    let z_mapper = &command_line.content_mappers()[0];
    let yz_mapper = &command_line.content_mappers()[1];

    assert!(std::ptr::eq(command_line.get_content_mapper_for_file_name("/src/Component.y.z").unwrap(), yz_mapper));
    assert!(std::ptr::eq(command_line.get_content_mapper_for_file_name("/src/Component.z").unwrap(), z_mapper));
}

// contentmappers_test.go:30
#[test]
fn test_get_content_mapper_for_file_name_uses_host_case_sensitivity() {
    let insensitive = ParsedCommandLine {
        parsed_config: ParsedOptions { content_mappers: vec![mapper("", &[".vue"])], ..Default::default() },
        compare_paths_options: ComparePathsOptions { use_case_sensitive_file_names: false, ..Default::default() },
        ..ParsedCommandLine::empty()
    };
    let sensitive = ParsedCommandLine {
        parsed_config: ParsedOptions { content_mappers: vec![mapper("", &[".vue"])], ..Default::default() },
        compare_paths_options: ComparePathsOptions { use_case_sensitive_file_names: true, ..Default::default() },
        ..ParsedCommandLine::empty()
    };

    assert!(std::ptr::eq(
        insensitive.get_content_mapper_for_file_name("/src/Component.VUE").unwrap(),
        &insensitive.content_mappers()[0]
    ));
    assert!(sensitive.get_content_mapper_for_file_name("/src/Component.VUE").is_none());
}

// contentmappers_test.go:46
#[test]
fn test_get_output_file_names_excludes_mapper_owned_outputs() {
    let mut command_line = new_parsed_command_line(
        P::new(CompilerOptions {
            out_dir: "/dist".to_string(),
            declaration: Tristate::True,
            declaration_map: Tristate::True,
            source_map: Tristate::True,
            ..Default::default()
        }),
        vec!["/src/Component.vue".to_string()],
        Vec::new(),
        ComparePathsOptions { current_directory: "/".to_string(), use_case_sensitive_file_names: true },
    );
    command_line.parsed_config.content_mappers = vec![mapper("", &[".vue"])];

    assert_eq!(command_line.get_output_file_names(), vec!["/dist/Component.d.vue.ts".to_string()]);
}

// contentmappers_test.go:68
#[test]
fn test_resolve_content_mapper_manifest() {
    let host = new_vfs_parse_config_host(
        &[
            (
                "/home/project/node_modules/vue-ts-mapper/package.json",
                r#"{
            "name": "vue-ts-mapper",
            "version": "1.2.3",
            "typescript": { "contentMapper": { "exec": ["node", "./dist/mapper.js"], "compilerOptions": ["target", "jsx"] } }
        }"#,
            ),
            (
                "/home/node_modules/@scope/noversion/package.json",
                r#"{
            "name": "@scope/noversion",
            "typescript": { "contentMapper": { "exec": ["run"] } }
        }"#,
            ),
            (
                "/home/project/node_modules/no-name/package.json",
                r#"{
            "version": "1.0.0"
        }"#,
            ),
            (
                "/home/project/node_modules/no-manifest/package.json",
                r#"{
            "name": "no-manifest"
        }"#,
            ),
            (
                "/home/project/node_modules/no-exec/package.json",
                r#"{
            "name": "no-exec",
            "typescript": { "contentMapper": {} }
        }"#,
            ),
            (
                "/home/project/node_modules/bad-exec/package.json",
                r#"{
            "name": "bad-exec",
            "typescript": { "contentMapper": { "exec": "node ./mapper.js" } }
        }"#,
            ),
        ],
        "/home/project",
        true, /*useCaseSensitiveFileNames*/
    );

    // Name, version, and the verbatim exec argv are preserved.
    let (manifest, package_directory, diagnostic) = resolve_content_mapper_manifest(host, "/home/project/tsconfig.json", "vue-ts-mapper");
    assert!(diagnostic.is_none());
    assert_eq!(manifest.name, "vue-ts-mapper");
    assert_eq!(manifest.version, "1.2.3");
    assert_eq!(package_directory, "/home/project/node_modules/vue-ts-mapper");
    assert_eq!(manifest.exec, ["node", "./dist/mapper.js"]);
    assert_eq!(manifest.compiler_options, ["target", "jsx"]);

    // Resolution walks up node_modules; a package with no version resolves to a name and empty version.
    let (manifest, _, diagnostic) = resolve_content_mapper_manifest(host, "/home/project/src/tsconfig.json", "@scope/noversion");
    assert!(diagnostic.is_none());
    assert_eq!(manifest.name, "@scope/noversion");
    assert_eq!(manifest.version, "");

    // A package that is not installed reports a resolution diagnostic.
    let (_, _, diagnostic) = resolve_content_mapper_manifest(host, "/home/project/tsconfig.json", "missing-mapper");
    assert_eq!(diagnostic.unwrap().code(), diagnostics::The_content_mapper_package_0_could_not_be_resolved.code());

    // A package whose package.json has no name reports a diagnostic.
    let (_, package_directory, diagnostic) = resolve_content_mapper_manifest(host, "/home/project/tsconfig.json", "no-name");
    assert_eq!(package_directory, "/home/project/node_modules/no-name");
    assert_eq!(
        diagnostic.unwrap().code(),
        diagnostics::The_package_json_of_the_content_mapper_package_0_does_not_specify_a_name.code()
    );

    // A package that does not declare a "typescript.contentMapper" object reports a diagnostic.
    let (_, _, diagnostic) = resolve_content_mapper_manifest(host, "/home/project/tsconfig.json", "no-manifest");
    assert_eq!(
        diagnostic.unwrap().code(),
        diagnostics::The_package_json_of_the_content_mapper_package_0_does_not_declare_a_typescript_contentMapper_object.code()
    );

    // A "typescript.contentMapper" with no "exec", or an "exec" of the wrong type, reports a diagnostic.
    for pkg in ["no-exec", "bad-exec"] {
        let (_, _, diagnostic) = resolve_content_mapper_manifest(host, "/home/project/tsconfig.json", pkg);
        let Some(diagnostic) = diagnostic else {
            panic!("expected a diagnostic for {pkg}");
        };
        assert_eq!(
            diagnostic.code(),
            diagnostics::The_typescript_contentMapper_exec_of_the_content_mapper_package_0_must_be_a_non_empty_array_of_strings.code()
        );
    }
}

// contentmapper/host_test.go:300 (TestMapperDiagnosticName: Mapper.DiagnosticName lives in this crate.)
#[test]
fn test_mapper_diagnostic_name() {
    let with = |package: &str, name: &str, contribution_id: &str| Mapper {
        definition: Definition { package: package.to_string(), ..Default::default() },
        manifest: Manifest { name: name.to_string(), ..Default::default() },
        contribution_id: contribution_id.to_string(),
        ..Default::default()
    };
    let tests = [
        (with("configured", "resolved", "contributed"), "resolved"),
        (with("configured", "", "contributed"), "configured"),
        (with("", "", "contributed"), "contributed"),
    ];
    for (mapper, want) in &tests {
        assert_eq!(mapper.diagnostic_name(), *want);
    }
}

// execute/incremental/buildinfo_contentmapper_test.go:25 (TestStaticContentMapperTransformIdentity: Mapper.Identity
// and TransformIdentity live in this crate.)
#[test]
fn test_static_content_mapper_transform_identity() {
    let named = |package: &str, options: &str, name: &str, version: &str, compiler_options: &[&str]| Mapper {
        definition: Definition { package: package.to_string(), options: options.to_string(), ..Default::default() },
        manifest: Manifest {
            name: name.to_string(),
            version: version.to_string(),
            compiler_options: compiler_options.iter().map(|o| o.to_string()).collect(),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(named("", "", "vue", "2.0.0", &[]).identity(), "vue@2.0.0");
    assert_eq!(named("anon", "", "", "", &[]).identity(), "");

    let jsx_mapper = named("jsx", "", "jsx", "1.0.0", &["jsx"]);
    let jsx_preserve_identity = jsx_mapper.transform_identity(Some(&CompilerOptions { jsx: JsxEmit::Preserve, ..Default::default() }));
    let jsx_react_identity = jsx_mapper.transform_identity(Some(&CompilerOptions { jsx: JsxEmit::React, ..Default::default() }));
    assert_ne!(jsx_preserve_identity, jsx_react_identity);

    let options_a = named("vue", r#"{"mode":"a"}"#, "vue", "1.0.0", &[]);
    let options_b = named("vue", r#"{"mode":"b"}"#, "vue", "1.0.0", &[]);
    assert_ne!(
        options_a.transform_identity(Some(&CompilerOptions::default())),
        options_b.transform_identity(Some(&CompilerOptions::default()))
    );
}

// The bytes TransformIdentity hashes (contentmapper.go:104): the identity, NUL, the tsconfig options, NUL, and the
// declared compiler options as Go's json.Marshal writes them: declared order, zero-valued and unknown names left out.
// A different encoding would change every content-mapped file's cache identity and build-info version.
#[test]
fn test_transform_identity_hashes_declared_options_json() {
    let mapper = Mapper {
        definition: Definition { package: "vue".to_string(), options: r#"{"mode":"a"}"#.to_string(), ..Default::default() },
        manifest: Manifest {
            name: "vue".to_string(),
            version: "1.0.0".to_string(),
            compiler_options: ["target", "noSuchOption", "strict", "noEmit", "jsx", "lib", "paths"].iter().map(|o| o.to_string()).collect(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut paths = OrderedMap::default();
    paths.insert("@/*".to_string(), vec!["src/*".to_string()]);
    let options = CompilerOptions {
        target: ScriptTarget::ESNext,
        strict: Tristate::False,
        jsx: JsxEmit::Preserve,
        lib: Some(vec!["es2022".to_string(), "dom".to_string()]),
        paths: Some(paths),
        ..Default::default()
    };

    let declared = mapper.marshal_declared_options(Some(&options)).unwrap();
    let declared: Vec<(&str, &str)> = declared.iter().map(|(name, raw)| (name.as_str(), raw.as_str())).collect();
    assert_eq!(
        declared,
        [("target", "99"), ("strict", "false"), ("jsx", "1"), ("lib", r#"["es2022","dom"]"#), ("paths", r#"{"@/*":["src/*"]}"#)]
    );
    let hashed = [
        b"vue@1.0.0".as_slice(),
        b"\0",
        br#"{"mode":"a"}"#,
        b"\0",
        br#"{"target":99,"strict":false,"jsx":1,"lib":["es2022","dom"],"paths":{"@/*":["src/*"]}}"#,
    ]
    .concat();
    assert_eq!(mapper.transform_identity(Some(&options)), xxhash_rust::xxh3::xxh3_128(&hashed));

    // Without options, or without declared names, the declared options are the empty object.
    assert!(mapper.marshal_declared_options(None).unwrap().is_empty());
    let undeclared = Mapper { manifest: Manifest { name: "vue".to_string(), ..Default::default() }, ..Default::default() };
    assert_eq!(undeclared.transform_identity(Some(&options)), xxhash_rust::xxh3::xxh3_128(b"vue\0\0{}"));
}
