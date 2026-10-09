use tsrs_ast::{is_external_module, SourceFileParseOptions};
use tsrs_core::{tspath, ScriptKind};
use tsrs_tsoptions::{Definition, Manifest, Mapper};

use crate::{parse_result, MappedResult, TransformResultFiles};

fn parse_options() -> SourceFileParseOptions {
    SourceFileParseOptions {
        file_name: "/component.astro".to_string(),
        path: tspath::Path::new("/component.astro".to_string()),
        ..Default::default()
    }
}

fn mapper(extensions: &[&str]) -> Mapper {
    Mapper {
        definition: Definition { extensions: extensions.iter().map(|e| e.to_string()).collect(), ..Default::default() },
        manifest: Manifest { name: "mapper".to_string(), ..Default::default() },
        ..Default::default()
    }
}

fn mapped(text: &str, virtual_extension: &str) -> MappedResult {
    MappedResult {
        text: text.to_string(),
        virtual_extension: virtual_extension.to_string(),
        mappings: Some(tsrs_spanmap::new(&[])),
        ..Default::default()
    }
}

// transform_test.go:14
#[test]
fn test_parse_result_supplemental_file_extensions() {
    let mappings = tsrs_spanmap::new(&[]);
    let result = TransformResultFiles {
        virtual_extension: ".ts".to_string(),
        mappings: Some(mappings),
        supplemental: [".js", ".jsx", ".ts", ".tsx", ".mts", ".cts", ".json"].iter().map(|ext| mapped("", ext)).collect(),
        ..Default::default()
    };
    let files = parse_result(parse_options(), "", &mapper(&[".astro"]), "transform-identity", result).unwrap();
    let canonical = files.canonical.unwrap();
    assert_eq!(canonical.content_mapper_transform_identity(), "transform-identity");
    let canonical_supplementals = canonical.supplemental_source_files();
    assert_eq!(canonical_supplementals.len(), files.supplemental.len());

    let expected = [
        ("/component.astro.0.js", ScriptKind::JS),
        ("/component.astro.1.jsx", ScriptKind::JSX),
        ("/component.astro.2.ts", ScriptKind::TS),
        ("/component.astro.3.tsx", ScriptKind::TSX),
        ("/component.astro.4.mts", ScriptKind::TS),
        ("/component.astro.5.cts", ScriptKind::TS),
        ("/component.astro.6.json", ScriptKind::JSON),
    ];
    assert_eq!(files.supplemental.len(), expected.len());
    for (i, (file_name, script_kind)) in expected.into_iter().enumerate() {
        let file = files.supplemental[i];
        assert_eq!(file.file_name(), file_name);
        assert_eq!(&*file.path().0, file_name);
        assert_eq!(file.script_kind(), script_kind);
        assert_eq!(file.content_mapper_transform_identity(), "transform-identity");
        assert!(canonical_supplementals[i] == file);
        assert!(file.canonical_source_file() == Some(canonical));
    }
}

// transform_test.go:66
#[test]
fn test_parse_result_allows_supplemental_modules() {
    let files = parse_result(
        parse_options(),
        "",
        &mapper(&[".astro"]),
        "",
        TransformResultFiles {
            text: "export {};".to_string(),
            virtual_extension: ".ts".to_string(),
            mappings: Some(tsrs_spanmap::new(&[])),
            supplemental: vec![mapped("export const value = 1;", ".mts")],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(is_external_module(files.supplemental[0]));
}

// transform_test.go:89
#[test]
fn test_parse_result_does_not_leak_canonical_module_forcing_to_supplementals() {
    let files = parse_result(
        parse_options(),
        "",
        &mapper(&[]),
        "",
        TransformResultFiles {
            text: "const canonical = 1;".to_string(),
            virtual_extension: ".mts".to_string(),
            mappings: Some(tsrs_spanmap::new(&[])),
            supplemental: vec![mapped("const supplemental = 1;", ".ts")],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(files.canonical.unwrap().parse_options().external_module_indicator_options.force);
    assert!(!files.supplemental[0].parse_options().external_module_indicator_options.force);
}
