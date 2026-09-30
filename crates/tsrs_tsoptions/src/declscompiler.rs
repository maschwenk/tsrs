use std::sync::LazyLock;

use tsrs_core::{PollingKind, ScriptTarget, Tristate, WatchDirectoryKind, WatchFileKind};
use tsrs_diagnostics as diagnostics;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, DefaultValueDescription, ExtraValidation};
use crate::parsinghelpers::for_each_compiler_options_field;
use crate::tsconfigparsing::COMMAND_LINE_COMPILER_OPTIONS_MAP;
use tsrs_core::CompilerOptions;

pub static OPTIONS_DECLARATIONS: LazyLock<Vec<&'static CommandLineOption>> =
    LazyLock::new(|| COMMON_OPTIONS_WITH_BUILD.iter().chain(OPTIONS_FOR_COMPILER.iter()).copied().collect());

// BEGIN GENERATED
static COMMON_OPTIONS_WITH_BUILD_ITEMS: [CommandLineOption; 29] = [
    CommandLineOption {
        name: "help",
        short_name: "h",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        is_command_line_only: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Print_this_message),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "help",
        short_name: "?",
        kind: CommandLineOptionKind::Boolean,
        is_command_line_only: true,
        category: Some(&diagnostics::Command_line_Options),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "watch",
        short_name: "w",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        is_command_line_only: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Watch_input_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "preserveWatchOutput",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: false,
        category: Some(&diagnostics::Output_Formatting),
        description: Some(&diagnostics::Disable_wiping_the_console_in_watch_mode),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "listFiles",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Print_all_of_the_files_read_during_the_compilation),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "explainFiles",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Print_files_read_during_the_compilation_including_why_it_was_included),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "listEmittedFiles",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Print_the_names_of_emitted_files_after_a_compilation),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "pretty",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Output_Formatting),
        description: Some(&diagnostics::Enable_color_and_formatting_in_TypeScript_s_output_to_make_compiler_errors_easier_to_read),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "traceResolution",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Log_paths_used_during_the_moduleResolution_process),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "diagnostics",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Output_compiler_performance_information_after_building),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "extendedDiagnostics",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Output_more_detailed_compiler_performance_information_after_building),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "generateCpuProfile",
        kind: CommandLineOptionKind::String,
        is_file_path: true,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Emit_a_v8_CPU_profile_of_the_compiler_run_for_debugging),
        default_value_description: DefaultValueDescription::Str("profile.cpuprofile"),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "generateTrace",
        kind: CommandLineOptionKind::String,
        is_file_path: true,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Generates_an_event_trace_and_a_list_of_types),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "incremental",
        short_name: "i",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Projects),
        description: Some(&diagnostics::Save_tsbuildinfo_files_to_allow_for_incremental_compilation_of_projects),
        transpile_option_value: Tristate::Unknown,
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_false_unless_composite_is_set),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "declaration",
        short_name: "d",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        transpile_option_value: Tristate::Unknown,
        description: Some(&diagnostics::Generate_d_ts_files_from_TypeScript_and_JavaScript_files_in_your_project),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_false_unless_composite_is_set),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "declarationMap",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        default_value_description: DefaultValueDescription::Bool(false),
        description: Some(&diagnostics::Create_sourcemaps_for_d_ts_files),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "emitDeclarationOnly",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Only_output_d_ts_files_and_not_JavaScript_files),
        transpile_option_value: Tristate::Unknown,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "sourceMap",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        default_value_description: DefaultValueDescription::Bool(false),
        description: Some(&diagnostics::Create_source_map_files_for_emitted_JavaScript_files),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "inlineSourceMap",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Include_sourcemap_files_inside_the_emitted_JavaScript),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noCheck",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: false,
        category: Some(&diagnostics::Compiler_Diagnostics),
        description: Some(&diagnostics::Disable_full_type_checking_only_critical_parse_and_emit_errors_will_be_reported),
        transpile_option_value: Tristate::True,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "deduplicatePackages",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Deduplicate_packages_with_the_same_name_and_version),
        default_value_description: DefaultValueDescription::Bool(true),
        affects_program_structure: true,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noEmit",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Disable_emitting_files_from_a_compilation),
        transpile_option_value: Tristate::Unknown,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "assumeChangesOnlyAffectDirectDependencies",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Have_recompiles_in_projects_that_use_incremental_and_watch_mode_assume_that_changes_within_a_file_will_only_affect_files_directly_depending_on_it),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "locale",
        kind: CommandLineOptionKind::String,
        category: Some(&diagnostics::Command_line_Options),
        is_command_line_only: true,
        description: Some(&diagnostics::Set_the_language_of_the_messaging_from_TypeScript_This_does_not_affect_emit),
        default_value_description: DefaultValueDescription::Message(&diagnostics::Platform_specific),
        extra_validation: ExtraValidation::Locale,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "quiet",
        short_name: "q",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Do_not_print_diagnostics),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "singleThreaded",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Run_in_single_threaded_mode),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "pprofDir",
        kind: CommandLineOptionKind::String,
        is_file_path: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Generate_pprof_CPU_Slashmemory_profiles_to_the_given_directory),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "checkers",
        kind: CommandLineOptionKind::Number,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Set_the_number_of_checkers_per_project),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_4_unless_singleThreaded_is_passed),
        min_value: 1,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "runExternalCode",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Command_line_Options),
        is_command_line_only: true,
        description: Some(&diagnostics::Allow_loading_external_content_mapper_plugins_that_execute_code_during_compilation),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
];

pub(crate) static COMMON_OPTIONS_WITH_BUILD: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    vec![
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[0],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[1],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[2],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[3],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[4],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[5],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[6],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[7],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[8],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[9],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[10],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[11],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[12],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[13],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[14],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[15],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[16],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[17],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[18],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[19],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[20],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[21],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[22],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[23],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[24],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[25],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[26],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[27],
        &COMMON_OPTIONS_WITH_BUILD_ITEMS[28],
    ]
});

static OPTIONS_FOR_COMPILER_ITEMS: [CommandLineOption; 97] = [
    CommandLineOption {
        name: "all",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Show_all_compiler_options),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "version",
        short_name: "v",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Print_the_compiler_s_version),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "init",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Initializes_a_TypeScript_project_and_creates_a_tsconfig_json_file),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "project",
        short_name: "p",
        kind: CommandLineOptionKind::String,
        is_file_path: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Compile_the_project_given_the_path_to_its_configuration_file_or_to_a_folder_with_a_tsconfig_json),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "showConfig",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Command_line_Options),
        is_command_line_only: true,
        description: Some(&diagnostics::Print_the_final_configuration_instead_of_building),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "listFilesOnly",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Command_line_Options),
        is_command_line_only: true,
        description: Some(&diagnostics::Print_names_of_files_that_are_part_of_the_compilation_and_then_stop_processing),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "ignoreConfig",
        kind: CommandLineOptionKind::Boolean,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Command_line_Options),
        is_command_line_only: true,
        description: Some(&diagnostics::Ignore_the_tsconfig_found_and_build_with_commandline_options_and_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "target",
        short_name: "t",
        kind: CommandLineOptionKind::Enum,
        affects_source_file: true,
        affects_module_resolution: true,
        affects_emit: true,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Set_the_JavaScript_language_version_for_emitted_JavaScript_and_include_compatible_library_declarations),
        default_value_description: DefaultValueDescription::ScriptTarget(ScriptTarget::LatestStandard),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "module",
        short_name: "m",
        kind: CommandLineOptionKind::Enum,
        affects_module_resolution: true,
        affects_emit: true,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_what_module_code_is_generated),
        default_value_description: DefaultValueDescription::Tristate(Tristate::Unknown),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "lib",
        kind: CommandLineOptionKind::List,
        affects_program_structure: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Specify_a_set_of_bundled_library_declaration_files_that_describe_the_target_runtime_environment),
        transpile_option_value: Tristate::Unknown,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowJs",
        kind: CommandLineOptionKind::Boolean,
        allow_js_flag: true,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::JavaScript_Support),
        description: Some(&diagnostics::Allow_JavaScript_files_to_be_a_part_of_your_program_Use_the_checkJs_option_to_get_errors_from_these_files),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_false_unless_checkJs_is_set),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "checkJs",
        kind: CommandLineOptionKind::Boolean,
        affects_module_resolution: true,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::JavaScript_Support),
        description: Some(&diagnostics::Enable_error_reporting_in_type_checked_JavaScript_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "jsx",
        kind: CommandLineOptionKind::Enum,
        affects_source_file: true,
        affects_emit: true,
        affects_build_info: true,
        affects_module_resolution: true,
        affects_semantic_diagnostics: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Specify_what_JSX_code_is_generated),
        default_value_description: DefaultValueDescription::Tristate(Tristate::Unknown),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "outFile",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        affects_declaration_path: true,
        is_file_path: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Specify_a_file_that_bundles_all_outputs_into_one_JavaScript_file_If_declaration_is_true_also_designates_a_file_that_bundles_all_d_ts_output),
        transpile_option_value: Tristate::Unknown,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "outDir",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        affects_declaration_path: true,
        is_file_path: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Specify_an_output_folder_for_all_emitted_files),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "rootDir",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        affects_declaration_path: true,
        is_file_path: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_the_root_folder_within_your_source_files),
        default_value_description: DefaultValueDescription::Message(&diagnostics::Computed_from_the_list_of_input_files),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "composite",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        is_tsconfig_only: true,
        category: Some(&diagnostics::Projects),
        transpile_option_value: Tristate::Unknown,
        default_value_description: DefaultValueDescription::Bool(false),
        description: Some(&diagnostics::Enable_constraints_that_allow_a_TypeScript_project_to_be_used_with_project_references),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "tsBuildInfoFile",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        is_file_path: true,
        category: Some(&diagnostics::Projects),
        transpile_option_value: Tristate::Unknown,
        default_value_description: DefaultValueDescription::Str(".tsbuildinfo"),
        description: Some(&diagnostics::Specify_the_path_to_tsbuildinfo_incremental_compilation_file),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "removeComments",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Emit),
        default_value_description: DefaultValueDescription::Bool(false),
        description: Some(&diagnostics::Disable_emitting_comments),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "importHelpers",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        affects_source_file: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Allow_importing_helper_functions_from_tslib_once_per_project_instead_of_including_them_per_file),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "downlevelIteration",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Emit_more_compliant_but_verbose_and_less_performant_JavaScript_for_iteration),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "isolatedModules",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Ensure_that_each_file_can_be_safely_transpiled_without_relying_on_other_imports),
        transpile_option_value: Tristate::True,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "verbatimModuleSyntax",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Do_not_transform_or_elide_any_imports_or_exports_not_marked_as_type_only_ensuring_they_are_written_in_the_output_file_s_format_based_on_the_module_setting),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "isolatedDeclarations",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Require_sufficient_annotation_on_exports_so_other_tools_can_trivially_generate_declaration_files),
        default_value_description: DefaultValueDescription::Bool(false),
        affects_build_info: true,
        affects_semantic_diagnostics: true,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "erasableSyntaxOnly",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Do_not_allow_runtime_constructs_that_are_not_part_of_ECMAScript),
        default_value_description: DefaultValueDescription::Bool(false),
        affects_build_info: true,
        affects_semantic_diagnostics: true,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "libReplacement",
        kind: CommandLineOptionKind::Boolean,
        affects_program_structure: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Enable_lib_replacement),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "strict",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enable_all_strict_type_checking_options),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noImplicitAny",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enable_error_reporting_for_expressions_and_declarations_with_an_implied_any_type),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "strictNullChecks",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::When_type_checking_take_into_account_null_and_undefined),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "strictFunctionTypes",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::When_assigning_functions_check_to_ensure_parameters_and_the_return_values_are_subtype_compatible),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "strictBindCallApply",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Check_that_the_arguments_for_bind_call_and_apply_methods_match_the_original_function),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "strictPropertyInitialization",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Check_for_class_properties_that_are_declared_but_not_set_in_the_constructor),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "strictBuiltinIteratorReturn",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Built_in_iterators_are_instantiated_with_a_TReturn_type_of_undefined_instead_of_any),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noImplicitThis",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enable_error_reporting_when_this_is_given_the_type_any),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "useUnknownInCatchVariables",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        strict_flag: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Default_catch_clause_variables_as_unknown_instead_of_any),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_unless_strict_is_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "alwaysStrict",
        kind: CommandLineOptionKind::Boolean,
        affects_source_file: true,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Ensure_use_strict_is_always_emitted),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "stableTypeOrdering",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Ensure_types_are_ordered_stably_and_deterministically_across_compilations),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noUnusedLocals",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enable_error_reporting_when_local_variables_aren_t_read),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noUnusedParameters",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Raise_an_error_when_a_function_parameter_isn_t_read),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "exactOptionalPropertyTypes",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Interpret_optional_property_types_as_written_rather_than_adding_undefined),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noImplicitReturns",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enable_error_reporting_for_codepaths_that_do_not_explicitly_return_in_a_function),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noFallthroughCasesInSwitch",
        kind: CommandLineOptionKind::Boolean,
        affects_bind_diagnostics: true,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enable_error_reporting_for_fallthrough_cases_in_switch_statements),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noUncheckedIndexedAccess",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Add_undefined_to_a_type_when_accessed_using_an_index),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noImplicitOverride",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Ensure_overriding_members_in_derived_classes_are_marked_with_an_override_modifier),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noPropertyAccessFromIndexSignature",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        show_in_simplified_help_view: false,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Enforces_using_indexed_accessors_for_keys_declared_using_an_indexed_type),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "moduleResolution",
        kind: CommandLineOptionKind::Enum,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_how_TypeScript_looks_up_a_file_from_a_given_module_specifier),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_nodenext_if_module_is_nodenext_node16_if_module_is_node16_or_node18_otherwise_bundler),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "baseUrl",
        kind: CommandLineOptionKind::String,
        affects_module_resolution: true,
        is_file_path: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_the_base_directory_to_resolve_non_relative_module_names),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "paths",
        kind: CommandLineOptionKind::Object,
        affects_module_resolution: true,
        allow_config_dir_template_substitution: true,
        is_tsconfig_only: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_a_set_of_entries_that_re_map_imports_to_additional_lookup_locations),
        transpile_option_value: Tristate::Unknown,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "rootDirs",
        kind: CommandLineOptionKind::List,
        is_tsconfig_only: true,
        affects_module_resolution: true,
        allow_config_dir_template_substitution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Allow_multiple_folders_to_be_treated_as_one_when_resolving_modules),
        transpile_option_value: Tristate::Unknown,
        default_value_description: DefaultValueDescription::Message(&diagnostics::Computed_from_the_list_of_input_files),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "typeRoots",
        kind: CommandLineOptionKind::List,
        affects_module_resolution: true,
        allow_config_dir_template_substitution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_multiple_folders_that_act_like_Slashnode_modules_Slash_types),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "types",
        kind: CommandLineOptionKind::List,
        affects_program_structure: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Specify_type_package_names_to_be_included_without_being_referenced_in_a_source_file),
        transpile_option_value: Tristate::Unknown,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowSyntheticDefaultImports",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Allow_import_x_from_y_when_a_module_doesn_t_have_a_default_export),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "esModuleInterop",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_emit: true,
        affects_build_info: true,
        show_in_simplified_help_view: true,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Emit_additional_JavaScript_to_ease_support_for_importing_CommonJS_modules_This_enables_allowSyntheticDefaultImports_for_type_compatibility),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "preserveSymlinks",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Disable_resolving_symlinks_to_their_realpath_This_correlates_to_the_same_flag_in_node),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowUmdGlobalAccess",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Allow_accessing_UMD_globals_from_modules),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "moduleSuffixes",
        kind: CommandLineOptionKind::List,
        list_preserve_falsy_values: true,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::List_of_file_name_suffixes_to_search_when_resolving_a_module),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowImportingTsExtensions",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Allow_imports_to_include_TypeScript_file_extensions_Requires_moduleResolution_bundler_and_either_noEmit_or_emitDeclarationOnly_to_be_set),
        default_value_description: DefaultValueDescription::Bool(false),
        transpile_option_value: Tristate::Unknown,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "rewriteRelativeImportExtensions",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Rewrite_ts_tsx_mts_and_cts_file_extensions_in_relative_import_paths_to_their_JavaScript_equivalent_in_output_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "resolvePackageJsonExports",
        kind: CommandLineOptionKind::Boolean,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Use_the_package_json_exports_field_when_resolving_package_imports),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_when_moduleResolution_is_node16_nodenext_or_bundler_otherwise_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "resolvePackageJsonImports",
        kind: CommandLineOptionKind::Boolean,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Use_the_package_json_imports_field_when_resolving_imports),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_when_moduleResolution_is_node16_nodenext_or_bundler_otherwise_false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "customConditions",
        kind: CommandLineOptionKind::List,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Conditions_to_set_in_addition_to_the_resolver_specific_defaults_when_resolving_imports),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noUncheckedSideEffectImports",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Check_side_effect_imports),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "sourceRoot",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Specify_the_root_path_for_debuggers_to_find_the_reference_source_code),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "mapRoot",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Specify_the_location_where_debugger_should_locate_map_files_instead_of_generated_locations),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "inlineSources",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Include_source_code_in_the_sourcemaps_inside_the_emitted_JavaScript),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "experimentalDecorators",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Enable_experimental_support_for_legacy_experimental_decorators),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "emitDecoratorMetadata",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Emit_design_type_metadata_for_decorated_declarations_in_source_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "jsxFactory",
        kind: CommandLineOptionKind::String,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Specify_the_JSX_factory_function_used_when_targeting_React_JSX_emit_e_g_React_createElement_or_h),
        default_value_description: DefaultValueDescription::Str("`React.createElement`"),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "jsxFragmentFactory",
        kind: CommandLineOptionKind::String,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Specify_the_JSX_Fragment_reference_used_for_fragments_when_targeting_React_JSX_emit_e_g_React_Fragment_or_Fragment),
        default_value_description: DefaultValueDescription::Str("React.Fragment"),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "jsxImportSource",
        kind: CommandLineOptionKind::String,
        affects_semantic_diagnostics: true,
        affects_emit: true,
        affects_build_info: true,
        affects_module_resolution: true,
        affects_source_file: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Specify_module_specifier_used_to_import_the_JSX_factory_functions_when_using_jsx_Colon_react_jsx_Asterisk),
        default_value_description: DefaultValueDescription::Str("react"),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "resolveJsonModule",
        kind: CommandLineOptionKind::Boolean,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Enable_importing_json_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowArbitraryExtensions",
        kind: CommandLineOptionKind::Boolean,
        affects_program_structure: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Enable_importing_files_with_any_extension_provided_a_declaration_file_is_present),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "reactNamespace",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Specify_the_object_invoked_for_createElement_This_only_applies_when_targeting_react_JSX_emit),
        default_value_description: DefaultValueDescription::Str("`React`"),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "skipDefaultLibCheck",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        category: Some(&diagnostics::Completeness),
        description: Some(&diagnostics::Skip_type_checking_d_ts_files_that_are_included_with_TypeScript),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "emitBOM",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Emit_a_UTF_8_Byte_Order_Mark_BOM_in_the_beginning_of_output_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "newLine",
        kind: CommandLineOptionKind::Enum,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Set_the_newline_character_for_emitting_files),
        default_value_description: DefaultValueDescription::Str("lf"),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noErrorTruncation",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Output_Formatting),
        description: Some(&diagnostics::Disable_truncating_types_in_error_messages),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noLib",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Language_and_Environment),
        affects_program_structure: true,
        description: Some(&diagnostics::Disable_including_any_library_files_including_the_default_lib_d_ts),
        transpile_option_value: Tristate::True,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noResolve",
        kind: CommandLineOptionKind::Boolean,
        affects_module_resolution: true,
        category: Some(&diagnostics::Modules),
        description: Some(&diagnostics::Disallow_import_s_require_s_or_reference_s_from_expanding_the_number_of_files_TypeScript_should_add_to_a_project),
        transpile_option_value: Tristate::True,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "stripInternal",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Disable_emitting_declarations_that_have_internal_in_their_JSDoc_comments),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "disableSizeLimit",
        kind: CommandLineOptionKind::Boolean,
        affects_program_structure: true,
        category: Some(&diagnostics::Editor_Support),
        description: Some(&diagnostics::Remove_the_20mb_cap_on_total_source_code_size_for_JavaScript_files_in_the_TypeScript_language_server),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "disableSourceOfProjectReferenceRedirect",
        kind: CommandLineOptionKind::Boolean,
        is_tsconfig_only: true,
        category: Some(&diagnostics::Projects),
        description: Some(&diagnostics::Disable_preferring_source_files_instead_of_declaration_files_when_referencing_composite_projects),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "disableSolutionSearching",
        kind: CommandLineOptionKind::Boolean,
        is_tsconfig_only: true,
        category: Some(&diagnostics::Projects),
        description: Some(&diagnostics::Opt_a_project_out_of_multi_project_reference_checking_when_editing),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "disableReferencedProjectLoad",
        kind: CommandLineOptionKind::Boolean,
        is_tsconfig_only: true,
        category: Some(&diagnostics::Projects),
        description: Some(&diagnostics::Reduce_the_number_of_projects_loaded_automatically_by_TypeScript),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noEmitHelpers",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Disable_generating_custom_helper_functions_like_extends_in_compiled_output),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "noEmitOnError",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        transpile_option_value: Tristate::Unknown,
        description: Some(&diagnostics::Disable_emitting_files_if_any_type_checking_errors_are_reported),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "preserveConstEnums",
        kind: CommandLineOptionKind::Boolean,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Emit),
        description: Some(&diagnostics::Disable_erasing_const_enum_declarations_in_generated_code),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "declarationDir",
        kind: CommandLineOptionKind::String,
        affects_emit: true,
        affects_build_info: true,
        affects_declaration_path: true,
        is_file_path: true,
        category: Some(&diagnostics::Emit),
        transpile_option_value: Tristate::Unknown,
        description: Some(&diagnostics::Specify_the_output_directory_for_generated_declaration_files),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "skipLibCheck",
        kind: CommandLineOptionKind::Boolean,
        affects_build_info: true,
        category: Some(&diagnostics::Completeness),
        description: Some(&diagnostics::Skip_type_checking_all_d_ts_files),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowUnusedLabels",
        kind: CommandLineOptionKind::Boolean,
        affects_bind_diagnostics: true,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Disable_error_reporting_for_unused_labels),
        default_value_description: DefaultValueDescription::Tristate(Tristate::Unknown),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "allowUnreachableCode",
        kind: CommandLineOptionKind::Boolean,
        affects_bind_diagnostics: true,
        affects_semantic_diagnostics: true,
        affects_build_info: true,
        category: Some(&diagnostics::Type_Checking),
        description: Some(&diagnostics::Disable_error_reporting_for_unreachable_code),
        default_value_description: DefaultValueDescription::Tristate(Tristate::Unknown),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "forceConsistentCasingInFileNames",
        kind: CommandLineOptionKind::Boolean,
        affects_module_resolution: true,
        category: Some(&diagnostics::Interop_Constraints),
        description: Some(&diagnostics::Ensure_that_casing_is_correct_in_imports),
        default_value_description: DefaultValueDescription::Bool(true),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "maxNodeModuleJsDepth",
        kind: CommandLineOptionKind::Number,
        affects_module_resolution: true,
        category: Some(&diagnostics::JavaScript_Support),
        description: Some(&diagnostics::Specify_the_maximum_folder_depth_used_for_checking_JavaScript_files_from_node_modules_Only_applicable_with_allowJs),
        default_value_description: DefaultValueDescription::Int(0),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "useDefineForClassFields",
        kind: CommandLineOptionKind::Boolean,
        affects_semantic_diagnostics: true,
        affects_emit: true,
        affects_build_info: true,
        category: Some(&diagnostics::Language_and_Environment),
        description: Some(&diagnostics::Emit_ECMAScript_standard_compliant_class_fields),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_true_for_ES2022_and_above_including_ESNext),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "plugins",
        kind: CommandLineOptionKind::List,
        is_tsconfig_only: true,
        description: Some(&diagnostics::Specify_a_list_of_language_service_plugins_to_include),
        category: Some(&diagnostics::Editor_Support),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "moduleDetection",
        kind: CommandLineOptionKind::Enum,
        affects_source_file: true,
        affects_module_resolution: true,
        description: Some(&diagnostics::Control_what_method_is_used_to_detect_module_format_JS_files),
        category: Some(&diagnostics::Language_and_Environment),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_auto_Colon_Treat_files_with_imports_exports_import_meta_jsx_with_jsx_Colon_react_jsx_or_esm_format_with_module_Colon_node16_as_modules),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "ignoreDeprecations",
        kind: CommandLineOptionKind::String,
        default_value_description: DefaultValueDescription::Tristate(Tristate::Unknown),
        ..CommandLineOption::DEFAULT
    },
];

pub(crate) static OPTIONS_FOR_COMPILER: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    vec![
        &OPTIONS_FOR_COMPILER_ITEMS[0],
        &OPTIONS_FOR_COMPILER_ITEMS[1],
        &OPTIONS_FOR_COMPILER_ITEMS[2],
        &OPTIONS_FOR_COMPILER_ITEMS[3],
        &OPTIONS_FOR_COMPILER_ITEMS[4],
        &OPTIONS_FOR_COMPILER_ITEMS[5],
        &OPTIONS_FOR_COMPILER_ITEMS[6],
        &OPTIONS_FOR_COMPILER_ITEMS[7],
        &OPTIONS_FOR_COMPILER_ITEMS[8],
        &OPTIONS_FOR_COMPILER_ITEMS[9],
        &OPTIONS_FOR_COMPILER_ITEMS[10],
        &OPTIONS_FOR_COMPILER_ITEMS[11],
        &OPTIONS_FOR_COMPILER_ITEMS[12],
        &OPTIONS_FOR_COMPILER_ITEMS[13],
        &OPTIONS_FOR_COMPILER_ITEMS[14],
        &OPTIONS_FOR_COMPILER_ITEMS[15],
        &OPTIONS_FOR_COMPILER_ITEMS[16],
        &OPTIONS_FOR_COMPILER_ITEMS[17],
        &OPTIONS_FOR_COMPILER_ITEMS[18],
        &OPTIONS_FOR_COMPILER_ITEMS[19],
        &OPTIONS_FOR_COMPILER_ITEMS[20],
        &OPTIONS_FOR_COMPILER_ITEMS[21],
        &OPTIONS_FOR_COMPILER_ITEMS[22],
        &OPTIONS_FOR_COMPILER_ITEMS[23],
        &OPTIONS_FOR_COMPILER_ITEMS[24],
        &OPTIONS_FOR_COMPILER_ITEMS[25],
        &OPTIONS_FOR_COMPILER_ITEMS[26],
        &OPTIONS_FOR_COMPILER_ITEMS[27],
        &OPTIONS_FOR_COMPILER_ITEMS[28],
        &OPTIONS_FOR_COMPILER_ITEMS[29],
        &OPTIONS_FOR_COMPILER_ITEMS[30],
        &OPTIONS_FOR_COMPILER_ITEMS[31],
        &OPTIONS_FOR_COMPILER_ITEMS[32],
        &OPTIONS_FOR_COMPILER_ITEMS[33],
        &OPTIONS_FOR_COMPILER_ITEMS[34],
        &OPTIONS_FOR_COMPILER_ITEMS[35],
        &OPTIONS_FOR_COMPILER_ITEMS[36],
        &OPTIONS_FOR_COMPILER_ITEMS[37],
        &OPTIONS_FOR_COMPILER_ITEMS[38],
        &OPTIONS_FOR_COMPILER_ITEMS[39],
        &OPTIONS_FOR_COMPILER_ITEMS[40],
        &OPTIONS_FOR_COMPILER_ITEMS[41],
        &OPTIONS_FOR_COMPILER_ITEMS[42],
        &OPTIONS_FOR_COMPILER_ITEMS[43],
        &OPTIONS_FOR_COMPILER_ITEMS[44],
        &OPTIONS_FOR_COMPILER_ITEMS[45],
        &OPTIONS_FOR_COMPILER_ITEMS[46],
        &OPTIONS_FOR_COMPILER_ITEMS[47],
        &OPTIONS_FOR_COMPILER_ITEMS[48],
        &OPTIONS_FOR_COMPILER_ITEMS[49],
        &OPTIONS_FOR_COMPILER_ITEMS[50],
        &OPTIONS_FOR_COMPILER_ITEMS[51],
        &OPTIONS_FOR_COMPILER_ITEMS[52],
        &OPTIONS_FOR_COMPILER_ITEMS[53],
        &OPTIONS_FOR_COMPILER_ITEMS[54],
        &OPTIONS_FOR_COMPILER_ITEMS[55],
        &OPTIONS_FOR_COMPILER_ITEMS[56],
        &OPTIONS_FOR_COMPILER_ITEMS[57],
        &OPTIONS_FOR_COMPILER_ITEMS[58],
        &OPTIONS_FOR_COMPILER_ITEMS[59],
        &OPTIONS_FOR_COMPILER_ITEMS[60],
        &OPTIONS_FOR_COMPILER_ITEMS[61],
        &OPTIONS_FOR_COMPILER_ITEMS[62],
        &OPTIONS_FOR_COMPILER_ITEMS[63],
        &OPTIONS_FOR_COMPILER_ITEMS[64],
        &OPTIONS_FOR_COMPILER_ITEMS[65],
        &OPTIONS_FOR_COMPILER_ITEMS[66],
        &OPTIONS_FOR_COMPILER_ITEMS[67],
        &OPTIONS_FOR_COMPILER_ITEMS[68],
        &OPTIONS_FOR_COMPILER_ITEMS[69],
        &OPTIONS_FOR_COMPILER_ITEMS[70],
        &OPTIONS_FOR_COMPILER_ITEMS[71],
        &OPTIONS_FOR_COMPILER_ITEMS[72],
        &OPTIONS_FOR_COMPILER_ITEMS[73],
        &OPTIONS_FOR_COMPILER_ITEMS[74],
        &OPTIONS_FOR_COMPILER_ITEMS[75],
        &OPTIONS_FOR_COMPILER_ITEMS[76],
        &OPTIONS_FOR_COMPILER_ITEMS[77],
        &OPTIONS_FOR_COMPILER_ITEMS[78],
        &OPTIONS_FOR_COMPILER_ITEMS[79],
        &OPTIONS_FOR_COMPILER_ITEMS[80],
        &OPTIONS_FOR_COMPILER_ITEMS[81],
        &OPTIONS_FOR_COMPILER_ITEMS[82],
        &OPTIONS_FOR_COMPILER_ITEMS[83],
        &OPTIONS_FOR_COMPILER_ITEMS[84],
        &OPTIONS_FOR_COMPILER_ITEMS[85],
        &OPTIONS_FOR_COMPILER_ITEMS[86],
        &OPTIONS_FOR_COMPILER_ITEMS[87],
        &OPTIONS_FOR_COMPILER_ITEMS[88],
        &OPTIONS_FOR_COMPILER_ITEMS[89],
        &OPTIONS_FOR_COMPILER_ITEMS[90],
        &OPTIONS_FOR_COMPILER_ITEMS[91],
        &OPTIONS_FOR_COMPILER_ITEMS[92],
        &OPTIONS_FOR_COMPILER_ITEMS[93],
        &OPTIONS_FOR_COMPILER_ITEMS[94],
        &OPTIONS_FOR_COMPILER_ITEMS[95],
        &OPTIONS_FOR_COMPILER_ITEMS[96],
    ]
});
// END GENERATED

fn options_have_changes(
    old_options: Option<&CompilerOptions>,
    new_options: Option<&CompilerOptions>,
    decl_filter: impl Fn(&CommandLineOption) -> bool,
) -> bool {
    let (old_options, new_options) = match (old_options, new_options) {
        (None, None) => return false,
        (Some(old), Some(new)) if std::ptr::eq(old, new) => return false,
        (Some(old), Some(new)) => (old, new),
        _ => return true,
    };
    // Go ForEachCompilerOptionValue walks the struct fields by reflection and looks each one up by name.
    macro_rules! compare_fields {
        ($($field:ident: $json:literal,)*) => {
            $(
                if let Some(option_declaration) = COMMAND_LINE_COMPILER_OPTIONS_MAP.get($json) {
                    if decl_filter(option_declaration) {
                        let changed = if option_declaration.strict_flag {
                            compare_strict(old_options, new_options, &old_options.$field, &new_options.$field)
                        } else if option_declaration.allow_js_flag {
                            old_options.get_allow_js() != new_options.get_allow_js()
                        } else {
                            old_options.$field != new_options.$field
                        };
                        if changed {
                            return true;
                        }
                    }
                }
            )*
        };
    }
    for_each_compiler_options_field!(compare_fields);
    false
}

fn compare_strict<T: std::any::Any>(old_options: &CompilerOptions, new_options: &CompilerOptions, old_value: &T, new_value: &T) -> bool {
    let old_value = (old_value as &dyn std::any::Any).downcast_ref::<Tristate>().copied().unwrap();
    let new_value = (new_value as &dyn std::any::Any).downcast_ref::<Tristate>().copied().unwrap();
    old_options.get_strict_option_value(old_value) != new_options.get_strict_option_value(new_value)
}

pub fn compiler_options_affect_semantic_diagnostics(old_options: Option<&CompilerOptions>, new_options: Option<&CompilerOptions>) -> bool {
    options_have_changes(old_options, new_options, |option| option.affects_semantic_diagnostics)
}

pub fn compiler_options_affect_declaration_path(old_options: Option<&CompilerOptions>, new_options: Option<&CompilerOptions>) -> bool {
    options_have_changes(old_options, new_options, |option| option.affects_declaration_path)
}

pub fn compiler_options_affect_emit(old_options: Option<&CompilerOptions>, new_options: Option<&CompilerOptions>) -> bool {
    options_have_changes(old_options, new_options, |option| option.affects_emit)
}
