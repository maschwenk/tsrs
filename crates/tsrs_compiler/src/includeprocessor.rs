use std::sync::{Mutex, OnceLock};

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, new_compiler_diagnostic, Diagnostic, DiagnosticsCollection, Node};
use tsrs_core::tspath::Path;
use tsrs_core::{ModuleKind, P};
use tsrs_diagnostics as diagnostics;
use tsrs_tsoptions as tsoptions;

use crate::file_include::{self, get_referenced_location, is_referenced_file, referenceFileLocation, FileIncludeReason};
use crate::processing_diagnostic::{includeExplainingDiagnostic, processingDiagnostic};
use crate::program::Program;

#[derive(Default)]
pub(crate) struct fileIncludeData {
    pub(crate) file_include_reasons: FxHashMap<Path, Vec<P<FileIncludeReason>>>,
    pub(crate) processing_diagnostics: Vec<processingDiagnostic>,
}

#[derive(Default)]
pub(crate) struct includeProcessor {
    pub(crate) reason_diagnostics: Mutex<FxHashMap<(P<FileIncludeReason>, bool), P<Diagnostic>>>,
    reason_to_reference_location: Mutex<FxHashMap<P<FileIncludeReason>, &'static referenceFileLocation>>,
    include_reason_to_related_info: Mutex<FxHashMap<P<FileIncludeReason>, Option<P<Diagnostic>>>>,
    redirect_and_file_format: Mutex<FxHashMap<Path, Option<Vec<P<Diagnostic>>>>>,
    computed_diagnostics: OnceLock<Mutex<DiagnosticsCollection>>,
    compiler_options_syntax: OnceLock<Option<P<Node>>>,
}

impl includeProcessor {
    pub(crate) fn get_diagnostics(&self, p: &Program) -> &Mutex<DiagnosticsCollection> {
        self.computed_diagnostics.get_or_init(|| {
            let mut computed = DiagnosticsCollection::default();
            for d in p.processing_diagnostics().iter() {
                computed.add(d.to_diagnostic(p));
            }
            for resolutions in p.resolved_modules.values() {
                for resolved_module in resolutions.values() {
                    for &diag in resolved_module.resolution_diagnostics.iter() {
                        computed.add(diag);
                    }
                }
            }
            for type_resolutions in p.type_resolutions_in_file.values() {
                for resolved_type_ref in type_resolutions.values() {
                    for &diag in resolved_type_ref.resolution_diagnostics.iter() {
                        computed.add(diag);
                    }
                }
            }
            Mutex::new(computed)
        })
    }

    pub(crate) fn get_reference_location(&self, r: P<FileIncludeReason>, program: &Program) -> &'static referenceFileLocation {
        if let Some(existing) = self.reason_to_reference_location.lock().unwrap().get(&r) {
            return existing;
        }
        let loc: &'static referenceFileLocation = Box::leak(Box::new(get_referenced_location(r, program)));
        *self.reason_to_reference_location.lock().unwrap().entry(r).or_insert(loc)
    }

    pub(crate) fn get_compiler_options_object_literal_syntax(&self, program: &Program) -> Option<P<Node>> {
        *self.compiler_options_syntax.get_or_init(|| {
            let config_file = program.opts.config.config_file.as_ref()?;
            let compiler_options_property =
                tsoptions::for_each_ts_config_prop_array(Some(config_file.source_file), "compilerOptions", |p| Some(p))?;
            let initializer = compiler_options_property.initializer()?;
            if ast::is_object_literal_expression(initializer) {
                Some(initializer)
            } else {
                None
            }
        })
    }

    pub(crate) fn get_related_info(&self, r: P<FileIncludeReason>, program: &Program) -> Option<P<Diagnostic>> {
        if let Some(existing) = self.include_reason_to_related_info.lock().unwrap().get(&r) {
            return *existing;
        }
        let related_info = file_include::to_related_info(r, program);
        *self.include_reason_to_related_info.lock().unwrap().entry(r).or_insert(related_info)
    }

    pub(crate) fn explain_redirect_and_implied_format(
        &self,
        program: &Program,
        file_path: &Path,
        to_file_name: &dyn Fn(&str) -> String,
    ) -> Option<Vec<P<Diagnostic>>> {
        if let Some(existing) = self.redirect_and_file_format.lock().unwrap().get(file_path) {
            return existing.clone();
        }
        let redirects_file = program.redirect_files_by_path.get(file_path);
        let mut source_file = None;
        let (file_name, path) = if let Some(redirects_file) = redirects_file {
            (redirects_file.file_name.clone(), redirects_file.path.clone())
        } else {
            let sf = program.get_source_file_by_path(file_path)?;
            source_file = Some(sf);
            (sf.file_name().to_string(), sf.path().clone())
        };
        let mut result: Option<Vec<P<Diagnostic>>> = None;
        let source = program.get_source_of_project_reference_if_output_included(&file_name, &path);
        if source != file_name {
            result.get_or_insert_with(Vec::new).push(new_compiler_diagnostic(
                &diagnostics::File_is_output_of_project_reference_source_0,
                &[&to_file_name(&source)],
            ));
        }

        if let Some(redirects_file) = redirects_file {
            let target_file = program.get_source_file_by_path(&redirects_file.target).unwrap();
            result.get_or_insert_with(Vec::new).push(new_compiler_diagnostic(
                &diagnostics::File_redirects_to_file_0,
                &[&to_file_name(target_file.file_name())],
            ));
        }

        if let Some(source_file) = source_file {
            if ast::is_external_or_common_js_module(source_file) {
                let meta_data = program.get_source_file_meta_data(&path);
                let package_json = format!("{}/package.json", meta_data.package_json_directory);
                match program.get_implied_node_format_for_emit(source_file) {
                    ModuleKind::ESNext => {
                        if meta_data.package_json_type == "module" {
                            result.get_or_insert_with(Vec::new).push(new_compiler_diagnostic(
                                &diagnostics::File_is_ECMAScript_module_because_0_has_field_type_with_value_module,
                                &[&to_file_name(&package_json)],
                            ));
                        }
                    }
                    ModuleKind::CommonJS => {
                        if !meta_data.package_json_type.is_empty() {
                            result.get_or_insert_with(Vec::new).push(new_compiler_diagnostic(
                                &diagnostics::File_is_CommonJS_module_because_0_has_field_type_whose_value_is_not_module,
                                &[&to_file_name(&package_json)],
                            ));
                        } else if !meta_data.package_json_directory.is_empty() {
                            result.get_or_insert_with(Vec::new).push(new_compiler_diagnostic(
                                &diagnostics::File_is_CommonJS_module_because_0_does_not_have_field_type,
                                &[&to_file_name(&package_json)],
                            ));
                        } else {
                            result.get_or_insert_with(Vec::new).push(new_compiler_diagnostic(
                                &diagnostics::File_is_CommonJS_module_because_package_json_was_not_found,
                                &[],
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }

        self.redirect_and_file_format.lock().unwrap().entry(file_path.clone()).or_insert(result).clone()
    }
}

impl fileIncludeData {
    pub(crate) fn add_processing_diagnostic(&mut self, d: processingDiagnostic) {
        self.processing_diagnostics.push(d);
    }

    pub(crate) fn add_processing_diagnostics_for_file_casing(
        &mut self,
        file: Path,
        existing_casing: &str,
        current_casing: &str,
        reason: Option<P<FileIncludeReason>>,
    ) {
        let already_referenced =
            self.file_include_reasons.get(&file).is_some_and(|reasons| reasons.iter().any(|r| r.is_referenced_file()));
        if !is_referenced_file(reason) && already_referenced {
            self.add_processing_diagnostic(processingDiagnostic::explaining(includeExplainingDiagnostic {
                file: Some(file),
                diagnostic_reason: reason,
                message: &diagnostics::Already_included_file_name_0_differs_from_file_name_1_only_in_casing,
                args: vec![existing_casing.to_string(), current_casing.to_string()],
            }));
        } else {
            self.add_processing_diagnostic(processingDiagnostic::explaining(includeExplainingDiagnostic {
                file: Some(file),
                diagnostic_reason: reason,
                message: &diagnostics::File_name_0_differs_from_already_included_file_name_1_only_in_casing,
                args: vec![current_casing.to_string(), existing_casing.to_string()],
            }));
        }
    }
}
