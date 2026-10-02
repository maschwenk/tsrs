use std::fmt::Display;

use tsrs_ast::{self as ast, new_compiler_diagnostic, new_diagnostic, Diagnostic, FileReference, Kind, Node, SourceFile};
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_module::PackageId;
use tsrs_tsoptions as tsoptions;

use crate::program::Program;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum fileIncludeKind {
    // References from file
    Import,
    ReferenceFile,
    TypeReferenceDirective,
    LibReferenceDirective,

    RootFile,
    LibFile,
    AutomaticTypeDirectiveFile,
}

pub struct FileIncludeReason {
    pub(crate) kind: fileIncludeKind,
    pub(crate) index: usize,
    pub(crate) is_default_lib: bool,
    pub(crate) referenced_file: Option<referencedFileData>,
    // Boxed: set on few reasons, and 88 bytes inline (203K reasons on the private monorepo, 144 -> 64 bytes each).
    pub(crate) automatic_type_directive: Option<Box<automaticTypeDirectiveFileData>>,
}

const _: () = assert!(std::mem::size_of::<FileIncludeReason>() == 64);

impl FileIncludeReason {
    pub(crate) fn new(kind: fileIncludeKind) -> FileIncludeReason {
        FileIncludeReason { kind, index: 0, is_default_lib: false, referenced_file: None, automatic_type_directive: None }
    }

    pub(crate) fn new_referenced(kind: fileIncludeKind, file: Path, index: i32, synthetic: Option<P<Node>>) -> P<FileIncludeReason> {
        let mut r = FileIncludeReason::new(kind);
        r.referenced_file = Some(referencedFileData { file, index, synthetic });
        P::new(r)
    }
}

pub(crate) struct referencedFileData {
    pub(crate) file: Path,
    pub(crate) index: i32,
    pub(crate) synthetic: Option<P<Node>>,
}

pub(crate) struct referenceFileLocation {
    pub(crate) file: P<SourceFile>,
    pub(crate) node: Option<P<Node>>,
    pub(crate) ref_: Option<P<FileReference>>,
    pub(crate) package_id: PackageId,
    pub(crate) is_synthetic: bool,
}

impl referenceFileLocation {
    pub(crate) fn text(&self) -> String {
        if let Some(node) = self.node {
            if !ast::node_is_synthesized(node) {
                let text = self.file.text();
                let start = tsrs_scanner::skip_trivia(text, node.pos()) as usize;
                text[start..node.end() as usize].to_string()
            } else {
                format!("\"{}\"", node.text())
            }
        } else {
            let r = self.ref_.unwrap();
            self.file.text()[r.text_range.pos() as usize..r.text_range.end() as usize].to_string()
        }
    }

    pub(crate) fn diagnostic_at(&self, message: &'static Message, args: &[&dyn Display]) -> P<Diagnostic> {
        if let Some(node) = self.node {
            tsoptions::create_diagnostic_for_node_in_source_file(self.file, node, message, args)
        } else {
            new_diagnostic(Some(self.file), self.ref_.unwrap().text_range, message, args)
        }
    }
}

pub(crate) struct automaticTypeDirectiveFileData {
    pub(crate) type_reference: String,
    pub(crate) package_id: PackageId,
}

impl FileIncludeReason {
    pub(crate) fn as_index(&self) -> usize {
        self.index
    }

    pub(crate) fn as_lib_file_index(&self) -> Option<usize> {
        if !self.is_default_lib {
            Some(self.index)
        } else {
            None
        }
    }

    pub(crate) fn is_referenced_file(&self) -> bool {
        self.kind <= fileIncludeKind::LibReferenceDirective
    }

    pub(crate) fn as_referenced_file_data(&self) -> &referencedFileData {
        self.referenced_file.as_ref().unwrap()
    }

    pub(crate) fn as_automatic_type_directive_file_data(&self) -> &automaticTypeDirectiveFileData {
        self.automatic_type_directive.as_ref().unwrap()
    }
}

pub(crate) fn is_referenced_file(r: Option<P<FileIncludeReason>>) -> bool {
    r.is_some_and(|r| r.is_referenced_file())
}

pub(crate) fn get_referenced_location(r: P<FileIncludeReason>, program: &Program) -> referenceFileLocation {
    let ref_ = r.as_referenced_file_data();
    let file = program.get_source_file_by_path(&ref_.file).unwrap();
    match r.kind {
        fileIncludeKind::Import => {
            let mut specifier: Option<P<Node>> = None;
            let mut is_synthetic = false;
            let imports = file.imports();
            if let Some(synthetic) = ref_.synthetic {
                specifier = Some(synthetic);
                is_synthetic = true;
            } else if (ref_.index as usize) < imports.len() {
                specifier = Some(imports[ref_.index as usize]);
            } else {
                let mut aug_index = imports.len() as i32;
                for &imp in file.module_augmentations.get() {
                    if imp.kind() == Kind::StringLiteral {
                        if aug_index == ref_.index {
                            specifier = Some(imp);
                            break;
                        }
                        aug_index += 1;
                    }
                }
            }
            let specifier = specifier.unwrap();
            let package_id = program
                .get_resolved_module_from_module_specifier(file, specifier)
                .map(|r| r.package_id.clone())
                .unwrap_or_default();
            referenceFileLocation { file, node: Some(specifier), ref_: None, package_id, is_synthetic }
        }
        fileIncludeKind::ReferenceFile => referenceFileLocation {
            file,
            node: None,
            ref_: Some(file.referenced_files.get()[ref_.index as usize]),
            package_id: PackageId::default(),
            is_synthetic: false,
        },
        fileIncludeKind::TypeReferenceDirective => referenceFileLocation {
            file,
            node: None,
            ref_: Some(file.type_reference_directives.get()[ref_.index as usize]),
            package_id: PackageId::default(),
            is_synthetic: false,
        },
        fileIncludeKind::LibReferenceDirective => referenceFileLocation {
            file,
            node: None,
            ref_: Some(file.lib_reference_directives.get()[ref_.index as usize]),
            package_id: PackageId::default(),
            is_synthetic: false,
        },
        _ => panic!("unknown reason: {:?}", r.kind),
    }
}

pub(crate) fn to_diagnostic(r: P<FileIncludeReason>, program: &Program, relative_file_name: bool) -> P<Diagnostic> {
    let key = (r, relative_file_name);
    if let Some(diagnostic) = program.include_processor.reason_diagnostics.lock().unwrap().get(&key) {
        return *diagnostic;
    }
    let diagnostic = compute_diagnostic(r, program, &|file_name: &str| {
        if relative_file_name {
            tspath::get_relative_path_from_directory(program.get_current_directory(), file_name, &program.compare_paths_options)
        } else {
            file_name.to_string()
        }
    });
    *program.include_processor.reason_diagnostics.lock().unwrap().entry(key).or_insert(diagnostic)
}

fn compute_diagnostic(r: P<FileIncludeReason>, program: &Program, to_file_name: &dyn Fn(&str) -> String) -> P<Diagnostic> {
    if r.is_referenced_file() {
        return compute_reference_file_diagnostic(r, program, to_file_name);
    }
    match r.kind {
        fileIncludeKind::RootFile => {
            if program.opts.config.config_file.is_some() {
                let config = program.opts.config;
                let file_name = tspath::get_normalized_absolute_path(&config.file_names()[r.as_index()], program.get_current_directory());
                let matched_file_spec = config.get_matched_file_spec(&file_name);
                if !matched_file_spec.is_empty() {
                    return new_compiler_diagnostic(
                        &diagnostics::Part_of_files_list_in_tsconfig_json,
                        &[&matched_file_spec, &to_file_name(&file_name)],
                    );
                }
                let (matched_include_spec, is_default_include_spec) = config.get_matched_include_spec(&file_name);
                if !matched_include_spec.is_empty() {
                    if is_default_include_spec {
                        new_compiler_diagnostic(&diagnostics::Matched_by_default_include_pattern_Asterisk_Asterisk_Slash_Asterisk, &[])
                    } else {
                        new_compiler_diagnostic(
                            &diagnostics::Matched_by_include_pattern_0_in_1,
                            &[&matched_include_spec, &to_file_name(&config.config_name())],
                        )
                    }
                } else {
                    new_compiler_diagnostic(&diagnostics::Root_file_specified_for_compilation, &[])
                }
            } else {
                new_compiler_diagnostic(&diagnostics::Root_file_specified_for_compilation, &[])
            }
        }
        fileIncludeKind::AutomaticTypeDirectiveFile => {
            let data = r.as_automatic_type_directive_file_data();
            if !program.options().uses_wildcard_types() {
                if !data.package_id.name.is_empty() {
                    new_compiler_diagnostic(
                        &diagnostics::Entry_point_of_type_library_0_specified_in_compilerOptions_with_packageId_1,
                        &[&data.type_reference, &data.package_id.to_string()],
                    )
                } else {
                    new_compiler_diagnostic(&diagnostics::Entry_point_of_type_library_0_specified_in_compilerOptions, &[&data.type_reference])
                }
            } else if !data.package_id.name.is_empty() {
                new_compiler_diagnostic(
                    &diagnostics::Entry_point_for_implicit_type_library_0_with_packageId_1,
                    &[&data.type_reference, &data.package_id.to_string()],
                )
            } else {
                new_compiler_diagnostic(&diagnostics::Entry_point_for_implicit_type_library_0, &[&data.type_reference])
            }
        }
        fileIncludeKind::LibFile => {
            if let Some(index) = r.as_lib_file_index() {
                return new_compiler_diagnostic(&diagnostics::Library_0_specified_in_compilerOptions, &[&program.options().lib.as_ref().unwrap()[index]]);
            }
            let target = program.options().get_emit_script_target().string();
            if !target.is_empty() {
                new_compiler_diagnostic(&diagnostics::Default_library_for_target_0, &[&target])
            } else {
                new_compiler_diagnostic(&diagnostics::Default_library, &[])
            }
        }
        _ => panic!("unknown reason: {:?}", r.kind),
    }
}

fn compute_reference_file_diagnostic(r: P<FileIncludeReason>, program: &Program, to_file_name: &dyn Fn(&str) -> String) -> P<Diagnostic> {
    let reference_location = program.include_processor.get_reference_location(r, program);
    let reference_text = reference_location.text();
    let file_name = to_file_name(reference_location.file.file_name());
    let package_id = &reference_location.package_id;
    match r.kind {
        fileIncludeKind::Import => {
            if !reference_location.is_synthetic {
                if !package_id.name.is_empty() {
                    new_compiler_diagnostic(
                        &diagnostics::Imported_via_0_from_file_1_with_packageId_2,
                        &[&reference_text, &file_name, &package_id.to_string()],
                    )
                } else {
                    new_compiler_diagnostic(&diagnostics::Imported_via_0_from_file_1, &[&reference_text, &file_name])
                }
            } else if program
                .import_helpers_import_specifiers
                .get(reference_location.file.path())
                .is_some_and(|s| Some(*s) == reference_location.node)
            {
                if !package_id.name.is_empty() {
                    new_compiler_diagnostic(
                        &diagnostics::Imported_via_0_from_file_1_with_packageId_2_to_import_importHelpers_as_specified_in_compilerOptions,
                        &[&reference_text, &file_name, &package_id.to_string()],
                    )
                } else {
                    new_compiler_diagnostic(
                        &diagnostics::Imported_via_0_from_file_1_to_import_importHelpers_as_specified_in_compilerOptions,
                        &[&reference_text, &file_name],
                    )
                }
            } else if !package_id.name.is_empty() {
                new_compiler_diagnostic(
                    &diagnostics::Imported_via_0_from_file_1_with_packageId_2_to_import_jsx_and_jsxs_factory_functions,
                    &[&reference_text, &file_name, &package_id.to_string()],
                )
            } else {
                new_compiler_diagnostic(
                    &diagnostics::Imported_via_0_from_file_1_to_import_jsx_and_jsxs_factory_functions,
                    &[&reference_text, &file_name],
                )
            }
        }
        fileIncludeKind::ReferenceFile => {
            new_compiler_diagnostic(&diagnostics::Referenced_via_0_from_file_1, &[&reference_text, &file_name])
        }
        fileIncludeKind::TypeReferenceDirective => {
            if !package_id.name.is_empty() {
                new_compiler_diagnostic(
                    &diagnostics::Type_library_referenced_via_0_from_file_1_with_packageId_2,
                    &[&reference_text, &file_name, &package_id.to_string()],
                )
            } else {
                new_compiler_diagnostic(&diagnostics::Type_library_referenced_via_0_from_file_1, &[&reference_text, &file_name])
            }
        }
        fileIncludeKind::LibReferenceDirective => {
            new_compiler_diagnostic(&diagnostics::Library_referenced_via_0_from_file_1, &[&reference_text, &file_name])
        }
        _ => panic!("unknown reason: {:?}", r.kind),
    }
}

pub(crate) fn to_related_info(r: P<FileIncludeReason>, program: &Program) -> Option<P<Diagnostic>> {
    if r.is_referenced_file() {
        return compute_reference_file_related_info(r, program);
    }
    let config_file = program.opts.config.config_file.as_ref()?;
    let config = program.opts.config;
    let config_source_file = config_file.source_file;
    match r.kind {
        fileIncludeKind::RootFile => {
            let file_name = tspath::get_normalized_absolute_path(&config.file_names()[r.as_index()], program.get_current_directory());
            let matched_file_spec = config.get_matched_file_spec(&file_name);
            if !matched_file_spec.is_empty() {
                if let Some(files_node) = tsoptions::get_ts_config_prop_array_element_value(Some(config_source_file), "files", &matched_file_spec) {
                    return Some(tsoptions::create_diagnostic_for_node_in_source_file(
                        config_source_file,
                        files_node,
                        &diagnostics::File_is_matched_by_files_list_specified_here,
                        &[],
                    ));
                }
            } else {
                let (matched_include_spec, is_default_include_spec) = config.get_matched_include_spec(&file_name);
                if !matched_include_spec.is_empty() && !is_default_include_spec {
                    if let Some(include_node) =
                        tsoptions::get_ts_config_prop_array_element_value(Some(config_source_file), "include", &matched_include_spec)
                    {
                        return Some(tsoptions::create_diagnostic_for_node_in_source_file(
                            config_source_file,
                            include_node,
                            &diagnostics::File_is_matched_by_include_pattern_specified_here,
                            &[],
                        ));
                    }
                }
            }
        }
        fileIncludeKind::AutomaticTypeDirectiveFile => {
            if !program.options().uses_wildcard_types() {
                let data = r.as_automatic_type_directive_file_data();
                if let Some(types_syntax) = tsoptions::get_options_syntax_by_array_element_value(
                    program.include_processor.get_compiler_options_object_literal_syntax(program),
                    "types",
                    &data.type_reference,
                ) {
                    return Some(tsoptions::create_diagnostic_for_node_in_source_file(
                        config_source_file,
                        types_syntax,
                        &diagnostics::File_is_entry_point_of_type_library_specified_here,
                        &[],
                    ));
                }
            }
        }
        fileIncludeKind::LibFile => {
            if let Some(index) = r.as_lib_file_index() {
                if let Some(lib_syntax) = tsoptions::get_options_syntax_by_array_element_value(
                    program.include_processor.get_compiler_options_object_literal_syntax(program),
                    "lib",
                    &program.options().lib.as_ref().unwrap()[index],
                ) {
                    return Some(tsoptions::create_diagnostic_for_node_in_source_file(
                        config_source_file,
                        lib_syntax,
                        &diagnostics::File_is_library_specified_here,
                        &[],
                    ));
                }
            } else {
                let target = program.options().get_emit_script_target().string();
                if !target.is_empty() {
                    if let Some(target_value_syntax) = tsoptions::for_each_property_assignment(
                        program.include_processor.get_compiler_options_object_literal_syntax(program),
                        "target",
                        tsoptions::get_callback_for_finding_property_assignment_by_value(target),
                        None,
                    ) {
                        return Some(tsoptions::create_diagnostic_for_node_in_source_file(
                            config_source_file,
                            target_value_syntax,
                            &diagnostics::File_is_default_library_for_target_specified_here,
                            &[],
                        ));
                    }
                }
            }
        }
        _ => panic!("unknown reason: {:?}", r.kind),
    }
    None
}

fn compute_reference_file_related_info(r: P<FileIncludeReason>, program: &Program) -> Option<P<Diagnostic>> {
    let reference_location = program.include_processor.get_reference_location(r, program);
    if reference_location.is_synthetic {
        return None;
    }
    Some(match r.kind {
        fileIncludeKind::Import => reference_location.diagnostic_at(&diagnostics::File_is_included_via_import_here, &[]),
        fileIncludeKind::ReferenceFile => reference_location.diagnostic_at(&diagnostics::File_is_included_via_reference_here, &[]),
        fileIncludeKind::TypeReferenceDirective => {
            reference_location.diagnostic_at(&diagnostics::File_is_included_via_type_library_reference_here, &[])
        }
        fileIncludeKind::LibReferenceDirective => {
            reference_location.diagnostic_at(&diagnostics::File_is_included_via_library_reference_here, &[])
        }
        _ => panic!("unknown reason: {:?}", r.kind),
    })
}
