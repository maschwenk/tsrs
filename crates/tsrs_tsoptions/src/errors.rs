use std::fmt::Display;

use tsrs_ast::{new_compiler_diagnostic, new_diagnostic, Diagnostic, Node, SourceFile};
use tsrs_core::{TextRange, P};
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind};
use crate::commandlineparser::CommandLineParser;
use crate::diagnostics::AlternateModeDiagnostics;
use crate::tsconfigparsing::{command_line_options_to_map, CommandLineOptionNameMap};

pub(crate) fn create_diagnostic_for_invalid_enum_type(
    opt: &CommandLineOption,
    source_file: Option<P<SourceFile>>,
    node: Option<P<Node>>,
) -> P<Diagnostic> {
    let names_of_type: Vec<&str> = opt.enum_map().unwrap().keys().copied().collect();
    let string_names = format_enum_type_keys(opt, names_of_type);
    let opt_name = format!("--{}", opt.name);
    create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
        source_file,
        node,
        &diagnostics::Argument_for_0_option_must_be_Colon_1,
        &[&opt_name, &string_names],
    )
}

pub(crate) fn format_enum_type_keys(opt: &CommandLineOption, keys: Vec<&str>) -> String {
    let mut keys = keys;
    if let Some(deprecated) = opt.deprecated_keys() {
        keys.retain(|key| !deprecated.contains(key));
    }
    format!("'{}'", keys.join("', '"))
}

pub(crate) fn get_compiler_option_value_type_string(option: &CommandLineOption) -> String {
    match option.kind {
        CommandLineOptionKind::ListOrElement => {
            format!("{} or Array", get_compiler_option_value_type_string(option.elements().unwrap()))
        }
        CommandLineOptionKind::List => "Array".to_string(),
        _ => option.kind.as_str().to_string(),
    }
}

impl CommandLineParser {
    pub(crate) fn create_unknown_option_error(
        &self,
        unknown_option: &str,
        unknown_option_error_text: &str,
        node: Option<P<Node>>,
        source_file: Option<P<SourceFile>>,
    ) -> P<Diagnostic> {
        create_unknown_option_error(
            unknown_option,
            self.unknown_option_diagnostic(),
            unknown_option_error_text,
            node,
            source_file,
            self.alternate_mode(),
            Some(self.unknown_did_you_mean_diagnostic()),
            Some(&command_line_options_to_map(&self.worker_diagnostics.did_you_mean.option_declarations)),
        )
    }
}

// createUnknownOptionError creates a diagnostic for an unknown option. If
// unknownDidYouMeanDiagnostic and optionsNameMap are provided, it also checks
// for a spelling suggestion and emits a "did you mean" diagnostic instead.
pub(crate) fn create_unknown_option_error(
    unknown_option: &str,
    unknown_option_diagnostic: &'static Message,
    unknown_option_error_text: &str,                        // optional
    node: Option<P<Node>>,                                  // optional
    source_file: Option<P<SourceFile>>,                     // optional
    alternate_mode: Option<&AlternateModeDiagnostics>,      // optional
    unknown_did_you_mean_diagnostic: Option<&'static Message>, // optional; nil skips suggestion
    options_name_map: Option<&CommandLineOptionNameMap>,    // optional; nil skips suggestion
) -> P<Diagnostic> {
    if let Some(alternate_mode) = alternate_mode {
        if let Some(options_name_map) = alternate_mode.options_name_map {
            if let Some(other_option) = options_name_map.get(&unknown_option.to_lowercase()) {
                // tscbuildoption
                let mut diagnostic = alternate_mode.diagnostic;
                if other_option.name == "build" {
                    diagnostic = &diagnostics::Option_build_must_be_the_first_command_line_argument;
                }
                return create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                    source_file,
                    node,
                    diagnostic,
                    &[&unknown_option],
                );
            }
        }
    }
    let unknown_option_error_text = if unknown_option_error_text.is_empty() { unknown_option } else { unknown_option_error_text };
    if let (Some(unknown_did_you_mean_diagnostic), Some(options_name_map)) = (unknown_did_you_mean_diagnostic, options_name_map) {
        if let Some(possible_option) = options_name_map.get_spelling_suggestion(unknown_option) {
            return create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                source_file,
                node,
                unknown_did_you_mean_diagnostic,
                &[&unknown_option_error_text, &possible_option.name],
            );
        }
    }
    create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
        source_file,
        node,
        unknown_option_diagnostic,
        &[&unknown_option_error_text],
    )
}

pub fn create_diagnostic_for_node_in_source_file(
    source_file: P<SourceFile>,
    node: P<Node>,
    message: &'static Message,
    args: &[&dyn Display],
) -> P<Diagnostic> {
    new_diagnostic(
        Some(source_file),
        TextRange::new(tsrs_scanner::skip_trivia(source_file.text(), node.pos()), node.end()),
        message,
        args,
    )
}

pub fn create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
    source_file: Option<P<SourceFile>>,
    node: Option<P<Node>>,
    message: &'static Message,
    args: &[&dyn Display],
) -> P<Diagnostic> {
    if let (Some(source_file), Some(node)) = (source_file, node) {
        return create_diagnostic_for_node_in_source_file(source_file, node, message, args);
    }
    new_compiler_diagnostic(message, args)
}

pub(crate) fn extra_key_diagnostics(s: &str) -> Option<&'static Message> {
    match s {
        "compilerOptions" => Some(&diagnostics::Unknown_compiler_option_0),
        "watchOptions" => Some(&diagnostics::Unknown_watch_option_0),
        "typeAcquisition" => Some(&diagnostics::Unknown_type_acquisition_option_0),
        "buildOptions" => Some(&diagnostics::Unknown_build_option_0),
        _ => None,
    }
}

pub(crate) fn extra_key_did_you_mean_diagnostics(s: &str) -> Option<&'static Message> {
    match s {
        "compilerOptions" => Some(&diagnostics::Unknown_compiler_option_0_Did_you_mean_1),
        "watchOptions" => Some(&diagnostics::Unknown_watch_option_0_Did_you_mean_1),
        "typeAcquisition" => Some(&diagnostics::Unknown_type_acquisition_option_0_Did_you_mean_1),
        "buildOptions" => Some(&diagnostics::Unknown_build_option_0_Did_you_mean_1),
        _ => None,
    }
}
