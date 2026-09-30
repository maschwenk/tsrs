use std::sync::{LazyLock, OnceLock};

use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;

use crate::commandlineoption::CommandLineOption;
use crate::declsbuild::BuildOpts;
use crate::declscompiler::OptionsDeclarations;
use crate::declswatch::OptionsForWatch;
use crate::namemap::{BuildNameMap, CompilerNameMap, NameMap};

pub struct DidYouMeanOptionsDiagnostics {
    pub(crate) alternate_mode: Option<AlternateModeDiagnostics>,
    pub option_declarations: Vec<&'static CommandLineOption>,
    pub unknown_option_diagnostic: &'static Message,
    pub unknown_did_you_mean_diagnostic: &'static Message,
}

pub struct AlternateModeDiagnostics {
    pub(crate) diagnostic: &'static Message,
    pub(crate) options_name_map: Option<&'static NameMap>,
}

pub struct ParseCommandLineWorkerDiagnostics {
    pub(crate) did_you_mean: DidYouMeanOptionsDiagnostics,
    pub(crate) options_name_map: OnceLock<NameMap>,
    pub option_type_mismatch_diagnostic: &'static Message,
}

pub static CompilerOptionsDidYouMeanDiagnostics: LazyLock<ParseCommandLineWorkerDiagnostics> =
    LazyLock::new(|| get_parse_command_line_worker_diagnostics(&OptionsDeclarations));

pub(crate) fn get_parse_command_line_worker_diagnostics(decls: &[&'static CommandLineOption]) -> ParseCommandLineWorkerDiagnostics {
    // this will only return the correct diagnostics for `compiler` mode, and is factored into a function for testing reasons.
    ParseCommandLineWorkerDiagnostics {
        did_you_mean: DidYouMeanOptionsDiagnostics {
            alternate_mode: Some(AlternateModeDiagnostics {
                diagnostic: &diagnostics::Compiler_option_0_may_only_be_used_with_build,
                options_name_map: Some(&BuildNameMap),
            }),
            option_declarations: decls.to_vec(),
            unknown_option_diagnostic: &diagnostics::Unknown_compiler_option_0,
            unknown_did_you_mean_diagnostic: &diagnostics::Unknown_compiler_option_0_Did_you_mean_1,
        },
        options_name_map: OnceLock::new(),
        option_type_mismatch_diagnostic: &diagnostics::Compiler_option_0_expects_an_argument,
    }
}

pub(crate) static watchOptionsDidYouMeanDiagnostics: LazyLock<ParseCommandLineWorkerDiagnostics> =
    LazyLock::new(|| ParseCommandLineWorkerDiagnostics {
        did_you_mean: DidYouMeanOptionsDiagnostics {
            // no alternateMode
            alternate_mode: None,
            option_declarations: OptionsForWatch.clone(),
            unknown_option_diagnostic: &diagnostics::Unknown_watch_option_0,
            unknown_did_you_mean_diagnostic: &diagnostics::Unknown_watch_option_0_Did_you_mean_1,
        },
        options_name_map: OnceLock::new(),
        option_type_mismatch_diagnostic: &diagnostics::Watch_option_0_requires_a_value_of_type_1,
    });

pub(crate) static buildOptionsDidYouMeanDiagnostics: LazyLock<ParseCommandLineWorkerDiagnostics> =
    LazyLock::new(|| ParseCommandLineWorkerDiagnostics {
        did_you_mean: DidYouMeanOptionsDiagnostics {
            alternate_mode: Some(AlternateModeDiagnostics {
                diagnostic: &diagnostics::Compiler_option_0_may_not_be_used_with_build,
                options_name_map: Some(&CompilerNameMap),
            }),
            option_declarations: BuildOpts.clone(),
            unknown_option_diagnostic: &diagnostics::Unknown_build_option_0,
            unknown_did_you_mean_diagnostic: &diagnostics::Unknown_build_option_0_Did_you_mean_1,
        },
        options_name_map: OnceLock::new(),
        option_type_mismatch_diagnostic: &diagnostics::Build_option_0_requires_a_value_of_type_1,
    });
