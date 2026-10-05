use std::sync::LazyLock;

use tsrs_diagnostics as diagnostics;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, DefaultValueDescription};

use crate::declscompiler::COMMON_OPTIONS_WITH_BUILD;

// BEGIN GENERATED
pub static TSC_BUILD_OPTION: CommandLineOption = CommandLineOption {
    name: "build",
    kind: CommandLineOptionKind::Boolean,
    short_name: "b",
    show_in_simplified_help_view: true,
    category: Some(&diagnostics::Command_line_Options),
    description: Some(&diagnostics::Build_one_or_more_projects_and_their_dependencies_if_out_of_date),
    default_value_description: DefaultValueDescription::Bool(false),
    ..CommandLineOption::DEFAULT
};

static OPTIONS_FOR_BUILD_ITEMS: [CommandLineOption; 6] = [
    CommandLineOption {
        name: "verbose",
        short_name: "v",
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Enable_verbose_logging),
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "dry",
        short_name: "d",
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Show_what_would_be_built_or_deleted_if_specified_with_clean),
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "force",
        short_name: "f",
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Build_all_projects_including_those_that_appear_to_be_up_to_date),
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "clean",
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Delete_the_outputs_of_all_projects),
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "builders",
        kind: CommandLineOptionKind::Number,
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Set_the_number_of_projects_to_build_concurrently),
        default_value_description: DefaultValueDescription::Message(&diagnostics::X_4_unless_singleThreaded_is_passed),
        min_value: 1,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "stopBuildOnErrors",
        category: Some(&diagnostics::Command_line_Options),
        description: Some(&diagnostics::Skip_building_downstream_projects_on_error_in_upstream_project),
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
];

pub static OPTIONS_FOR_BUILD: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    vec![
        &TSC_BUILD_OPTION,
        &OPTIONS_FOR_BUILD_ITEMS[0],
        &OPTIONS_FOR_BUILD_ITEMS[1],
        &OPTIONS_FOR_BUILD_ITEMS[2],
        &OPTIONS_FOR_BUILD_ITEMS[3],
        &OPTIONS_FOR_BUILD_ITEMS[4],
        &OPTIONS_FOR_BUILD_ITEMS[5],
    ]
});
// END GENERATED

pub static BUILD_OPTS: LazyLock<Vec<&'static CommandLineOption>> =
    LazyLock::new(|| COMMON_OPTIONS_WITH_BUILD.iter().chain(OPTIONS_FOR_BUILD.iter()).copied().collect());
