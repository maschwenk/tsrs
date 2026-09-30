use std::sync::LazyLock;

use tsrs_core::{PollingKind, ScriptTarget, Tristate, WatchDirectoryKind, WatchFileKind};
use tsrs_diagnostics as diagnostics;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, DefaultValueDescription, ExtraValidation};

// BEGIN GENERATED
static OptionsForWatch_items: [CommandLineOption; 7] = [
    CommandLineOption {
        name: "watchInterval",
        kind: CommandLineOptionKind::Number,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "watchFile",
        kind: CommandLineOptionKind::Enum,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Specify_how_the_TypeScript_watch_mode_works),
        default_value_description: DefaultValueDescription::WatchFileKind(WatchFileKind::UseFsEvents),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "watchDirectory",
        kind: CommandLineOptionKind::Enum,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Specify_how_directories_are_watched_on_systems_that_lack_recursive_file_watching_functionality),
        default_value_description: DefaultValueDescription::WatchDirectoryKind(WatchDirectoryKind::UseFsEvents),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "fallbackPolling",
        kind: CommandLineOptionKind::Enum,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Specify_what_approach_the_watcher_should_use_if_the_system_runs_out_of_native_file_watchers),
        default_value_description: DefaultValueDescription::PollingKind(PollingKind::PriorityInterval),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "synchronousWatchDirectory",
        kind: CommandLineOptionKind::Boolean,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Synchronously_call_callbacks_and_update_the_state_of_directory_watchers_on_platforms_that_don_t_support_recursive_watching_natively),
        default_value_description: DefaultValueDescription::Bool(false),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "excludeDirectories",
        kind: CommandLineOptionKind::List,
        allow_config_dir_template_substitution: true,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Remove_a_list_of_directories_from_the_watch_process),
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "excludeFiles",
        kind: CommandLineOptionKind::List,
        allow_config_dir_template_substitution: true,
        category: Some(&diagnostics::Watch_and_Build_Modes),
        description: Some(&diagnostics::Remove_a_list_of_files_from_the_watch_mode_s_processing),
        ..CommandLineOption::DEFAULT
    },
];

pub static OptionsForWatch: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    vec![
        &OptionsForWatch_items[0],
        &OptionsForWatch_items[1],
        &OptionsForWatch_items[2],
        &OptionsForWatch_items[3],
        &OptionsForWatch_items[4],
        &OptionsForWatch_items[5],
        &OptionsForWatch_items[6],
    ]
});
// END GENERATED
