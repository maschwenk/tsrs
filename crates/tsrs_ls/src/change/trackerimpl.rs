// PARTIAL (see mod.rs): trackerimpl.go, reduced to GetFormatCodeSettingsForWriting.

use tsrs_ast::SourceFile;
use tsrs_core::P;

use crate::lsutil::{self, FormatCodeSettings, SemicolonPreference};

// trackerimpl.go:252
pub fn get_format_code_settings_for_writing(options: FormatCodeSettings, source_file: P<SourceFile>) -> FormatCodeSettings {
    let mut options = options;
    let should_auto_detect_semicolon_preference = options.semicolons == SemicolonPreference::Ignore;
    let should_remove_semicolons = options.semicolons == SemicolonPreference::Remove || should_auto_detect_semicolon_preference && !lsutil::probably_uses_semicolons(source_file);
    if should_remove_semicolons {
        options.semicolons = SemicolonPreference::Remove;
    }

    options
}
