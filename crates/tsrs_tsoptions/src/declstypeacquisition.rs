use std::sync::LazyLock;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, DefaultValueDescription};
use crate::tsconfigparsing::command_line_options_to_map;

pub(crate) static typeAcquisitionDeclaration: LazyLock<CommandLineOption> = LazyLock::new(|| CommandLineOption {
    name: "typeAcquisition",
    kind: CommandLineOptionKind::Object,
    element_options: Some(command_line_options_to_map(&typeAcquisitionDecls)),
    ..CommandLineOption::DEFAULT
});

// Do not delete this without updating the website's tsconfig generation.
pub(crate) static typeAcquisitionDecls_items: [CommandLineOption; 4] = [
    CommandLineOption {
        name: "enable",
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption { name: "include", kind: CommandLineOptionKind::List, ..CommandLineOption::DEFAULT },
    CommandLineOption { name: "exclude", kind: CommandLineOptionKind::List, ..CommandLineOption::DEFAULT },
    CommandLineOption {
        name: "disableFilenameBasedTypeAcquisition",
        kind: CommandLineOptionKind::Boolean,
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
];

pub(crate) static typeAcquisitionDecls: LazyLock<Vec<&'static CommandLineOption>> =
    LazyLock::new(|| typeAcquisitionDecls_items.iter().collect());
