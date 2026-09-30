use std::fmt::Display;

use rustc_hash::FxHashSet;
use tsrs_ast::{new_compiler_diagnostic, Diagnostic, DiagnosticExt};
use tsrs_core::tspath::{self, Path};
use tsrs_core::{stringutil, P};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_tsoptions as tsoptions;

use crate::file_include::{self, fileIncludeKind, get_referenced_location, is_referenced_file, FileIncludeReason};
use crate::program::Program;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum processingDiagnosticKind {
    UnknownReference,
    ExplainingFileInclude,
}

pub(crate) struct processingDiagnostic {
    pub(crate) kind: processingDiagnosticKind,
    pub(crate) reason: Option<P<FileIncludeReason>>,
    pub(crate) explanation: Option<includeExplainingDiagnostic>,
}

impl processingDiagnostic {
    pub(crate) fn unknown_reference(reason: P<FileIncludeReason>) -> processingDiagnostic {
        processingDiagnostic { kind: processingDiagnosticKind::UnknownReference, reason: Some(reason), explanation: None }
    }

    pub(crate) fn explaining(explanation: includeExplainingDiagnostic) -> processingDiagnostic {
        processingDiagnostic { kind: processingDiagnosticKind::ExplainingFileInclude, reason: None, explanation: Some(explanation) }
    }

    fn as_file_include_reason(&self) -> P<FileIncludeReason> {
        self.reason.unwrap()
    }

    fn as_include_explaining_diagnostic(&self) -> &includeExplainingDiagnostic {
        self.explanation.as_ref().unwrap()
    }
}

pub(crate) struct includeExplainingDiagnostic {
    pub(crate) file: Option<Path>,
    pub(crate) diagnostic_reason: Option<P<FileIncludeReason>>,
    pub(crate) message: &'static Message,
    pub(crate) args: Vec<String>,
}

impl processingDiagnostic {
    pub(crate) fn to_diagnostic(&self, program: &Program) -> P<Diagnostic> {
        match self.kind {
            processingDiagnosticKind::UnknownReference => {
                let ref_ = self.as_file_include_reason();
                let loc = get_referenced_location(ref_, program);
                let file_ref = loc.ref_.unwrap();
                match ref_.kind {
                    fileIncludeKind::TypeReferenceDirective => {
                        loc.diagnostic_at(&diagnostics::Cannot_find_type_definition_file_for_0, &[&file_ref.file_name])
                    }
                    fileIncludeKind::LibReferenceDirective => {
                        let lib_name = tspath::to_file_name_lower_case(&file_ref.file_name);
                        let unqualified = lib_name.strip_prefix("lib.").unwrap_or(&lib_name);
                        let unqualified_lib_name = unqualified.strip_suffix(".d.ts").unwrap_or(unqualified);
                        let suggestion = tsrs_core::get_spelling_suggestion(
                            unqualified_lib_name,
                            tsoptions::LIBS.iter().copied(),
                            |s| *s,
                            |a, b| a.cmp(b) as i32,
                        )
                        .unwrap_or("");
                        let message = if !suggestion.is_empty() {
                            &diagnostics::Cannot_find_lib_definition_for_0_Did_you_mean_1
                        } else {
                            &diagnostics::Cannot_find_lib_definition_for_0
                        };
                        loc.diagnostic_at(message, &[&lib_name, &suggestion])
                    }
                    _ => panic!("unknown include kind"),
                }
            }
            processingDiagnosticKind::ExplainingFileInclude => self.create_diagnostic_explaining_file(program),
        }
    }

    fn create_diagnostic_explaining_file(&self, program: &Program) -> P<Diagnostic> {
        let diag = self.as_include_explaining_diagnostic();
        let mut include_details: Option<Vec<P<Diagnostic>>> = None;
        let mut related_info: Option<Vec<P<Diagnostic>>> = None;
        let mut redirect_info: Option<Vec<P<Diagnostic>>> = None;
        let mut preferred_location: Option<P<FileIncludeReason>> = None;
        let mut seen_reasons: FxHashSet<P<FileIncludeReason>> = FxHashSet::default();
        if is_referenced_file(diag.diagnostic_reason)
            && !program.include_processor.get_reference_location(diag.diagnostic_reason.unwrap(), program).is_synthetic
        {
            preferred_location = diag.diagnostic_reason;
        }

        let mut process_include = |include_reason: P<FileIncludeReason>,
                                   include_details: &mut Option<Vec<P<Diagnostic>>>,
                                   related_info: &mut Option<Vec<P<Diagnostic>>>,
                                   preferred_location: &mut Option<P<FileIncludeReason>>| {
            if !seen_reasons.insert(include_reason) {
                return;
            }
            include_details.get_or_insert_with(Vec::new).push(file_include::to_diagnostic(include_reason, program, false));
            // processRelatedInfo
            if preferred_location.is_none()
                && include_reason.is_referenced_file()
                && !program.include_processor.get_reference_location(include_reason, program).is_synthetic
            {
                *preferred_location = Some(include_reason);
            } else if *preferred_location != Some(include_reason) {
                if let Some(info) = program.include_processor.get_related_info(include_reason, program) {
                    related_info.get_or_insert_with(Vec::new).push(info);
                }
            }
        };

        if let Some(file) = &diag.file {
            let reasons = program.file_include_data.file_include_reasons.get(file).cloned().unwrap_or_default();
            include_details = Some(Vec::with_capacity(reasons.len()));
            for reason in reasons {
                process_include(reason, &mut include_details, &mut related_info, &mut preferred_location);
            }
            redirect_info = program.include_processor.explain_redirect_and_implied_format(program, file, &|file_name: &str| file_name.to_string());
        }
        if let Some(reason) = diag.diagnostic_reason {
            process_include(reason, &mut include_details, &mut related_info, &mut preferred_location);
        }
        let seen_len = seen_reasons.len();
        let mut chain: Option<Vec<P<Diagnostic>>> = None;
        if let Some(include_details) = &include_details {
            if preferred_location.is_none() || seen_len != 1 {
                let file_reason = new_compiler_diagnostic(&diagnostics::The_file_is_in_the_program_because_Colon, &[]);
                file_reason.set_message_chain(include_details);
                chain = Some(vec![file_reason]);
            }
        }
        if let Some(redirect_info) = redirect_info {
            chain.get_or_insert_with(Vec::new).extend(redirect_info);
        }

        let args: Vec<&dyn Display> = diag.args.iter().map(|a| a as &dyn Display).collect();
        let mut result = None;
        if let Some(preferred_location) = preferred_location {
            result = Some(program.include_processor.get_reference_location(preferred_location, program).diagnostic_at(diag.message, &args));
        }
        let result = result.unwrap_or_else(|| new_compiler_diagnostic(diag.message, &args));
        if let Some(chain) = chain {
            result.set_message_chain(&chain);
        }
        if let Some(related_info) = related_info {
            result.set_related_info(&related_info);
        }
        result
    }
}
