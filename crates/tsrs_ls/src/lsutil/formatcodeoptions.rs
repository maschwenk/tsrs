use tsrs_core::json::Value;
use tsrs_core::Tristate;

// formatcodeoptions.go:11
// Go `type IndentStyle int`: parseIndentStyle stores any number it is given, so the type is an open set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct IndentStyle(pub i32);

impl IndentStyle {
    pub const None: IndentStyle = IndentStyle(0);
    pub const Block: IndentStyle = IndentStyle(1);
    pub const Smart: IndentStyle = IndentStyle(2);
}

// formatcodeoptions.go:19
pub(crate) fn parse_indent_style(v: &Value) -> IndentStyle {
    match v {
        Value::String(s) => match s.to_lowercase().as_str() {
            "none" => return IndentStyle::None,
            "block" => return IndentStyle::Block,
            "smart" => return IndentStyle::Smart,
            _ => {}
        },
        Value::Number(s) => return IndentStyle(*s as i32),
        _ => {}
    }
    IndentStyle::Smart
}

// formatcodeoptions.go:38
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SemicolonPreference {
    #[default]
    None, // "" (Go zero value)
    Ignore,
    Insert,
    Remove,
}

impl SemicolonPreference {
    pub fn as_str(self) -> &'static str {
        match self {
            SemicolonPreference::None => "",
            SemicolonPreference::Ignore => "ignore",
            SemicolonPreference::Insert => "insert",
            SemicolonPreference::Remove => "remove",
        }
    }
}

// formatcodeoptions.go:46
pub(crate) fn parse_semicolon_preference(v: &Value) -> SemicolonPreference {
    if let Value::String(s) = v {
        match s.to_lowercase().as_str() {
            "ignore" => return SemicolonPreference::Ignore,
            "insert" => return SemicolonPreference::Insert,
            "remove" => return SemicolonPreference::Remove,
            _ => {}
        }
    }
    SemicolonPreference::Ignore
}

// formatcodeoptions.go:60
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EditorSettings {
    pub base_indent_size: i32,
    pub indent_size: i32,
    pub tab_size: i32,
    pub new_line_character: String,
    pub convert_tabs_to_spaces: Tristate,
    pub indent_style: IndentStyle,
    pub trim_trailing_whitespace: Tristate,
}

// formatcodeoptions.go:70
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FormatCodeSettings {
    pub editor_settings: EditorSettings,
    pub insert_space_after_comma_delimiter: Tristate,
    pub insert_space_after_semicolon_in_for_statements: Tristate,
    pub insert_space_before_and_after_binary_operators: Tristate,
    pub insert_space_after_constructor: Tristate,
    pub insert_space_after_keywords_in_control_flow_statements: Tristate,
    pub insert_space_after_function_keyword_for_anonymous_functions: Tristate,
    pub insert_space_after_opening_and_before_closing_nonempty_parenthesis: Tristate,
    pub insert_space_after_opening_and_before_closing_nonempty_brackets: Tristate,
    pub insert_space_after_opening_and_before_closing_nonempty_braces: Tristate,
    pub insert_space_after_opening_and_before_closing_empty_braces: Tristate,
    pub insert_space_after_opening_and_before_closing_template_string_braces: Tristate,
    pub insert_space_after_opening_and_before_closing_jsx_expression_braces: Tristate,
    pub insert_space_after_type_assertion: Tristate,
    pub insert_space_before_function_parenthesis: Tristate,
    pub place_open_brace_on_new_line_for_functions: Tristate,
    pub place_open_brace_on_new_line_for_control_blocks: Tristate,
    pub insert_space_before_type_annotation: Tristate,
    pub indent_multi_line_object_literal_beginning_on_blank_line: Tristate,
    pub semicolons: SemicolonPreference,
    pub indent_switch_case: Tristate,
}

impl std::ops::Deref for FormatCodeSettings {
    type Target = EditorSettings;
    #[inline]
    fn deref(&self) -> &EditorSettings {
        &self.editor_settings
    }
}

impl std::ops::DerefMut for FormatCodeSettings {
    #[inline]
    fn deref_mut(&mut self) -> &mut EditorSettings {
        &mut self.editor_settings
    }
}

// formatcodeoptions.go:94
pub fn from_ls_format_options(f: &FormatCodeSettings, opt: tsrs_lsproto::FormattingOptions) -> FormatCodeSettings {
    let mut updated_settings = f.clone();
    updated_settings.tab_size = opt.tab_size as i32;
    updated_settings.indent_size = opt.tab_size as i32;
    updated_settings.convert_tabs_to_spaces = tsrs_core::bool_to_tristate(opt.insert_spaces);
    if let Some(trim_trailing_whitespace) = opt.trim_trailing_whitespace {
        updated_settings.trim_trailing_whitespace = tsrs_core::bool_to_tristate(trim_trailing_whitespace);
    }
    updated_settings
}

impl FormatCodeSettings {
    // formatcodeoptions.go:105
    pub fn to_ls_format_options(&self) -> tsrs_lsproto::FormattingOptions {
        let trim_trailing_whitespace = self.trim_trailing_whitespace.is_true();
        tsrs_lsproto::FormattingOptions {
            tab_size: self.tab_size as u32,
            insert_spaces: self.convert_tabs_to_spaces.is_true(),
            trim_trailing_whitespace: Some(trim_trailing_whitespace),
            ..Default::default()
        }
    }
}

// formatcodeoptions.go:114
pub fn get_default_format_code_settings() -> FormatCodeSettings {
    FormatCodeSettings {
        editor_settings: EditorSettings {
            indent_size: tsrs_printer::get_default_indent_size() as i32,
            tab_size: tsrs_printer::get_default_indent_size() as i32,
            new_line_character: "\n".to_string(),
            convert_tabs_to_spaces: Tristate::True,
            indent_style: IndentStyle::Smart,
            trim_trailing_whitespace: Tristate::True,
            ..Default::default()
        },
        insert_space_after_constructor: Tristate::False,
        insert_space_after_comma_delimiter: Tristate::True,
        insert_space_after_semicolon_in_for_statements: Tristate::True,
        insert_space_before_and_after_binary_operators: Tristate::True,
        insert_space_after_keywords_in_control_flow_statements: Tristate::True,
        insert_space_after_function_keyword_for_anonymous_functions: Tristate::False,
        insert_space_after_opening_and_before_closing_nonempty_parenthesis: Tristate::False,
        insert_space_after_opening_and_before_closing_nonempty_brackets: Tristate::False,
        insert_space_after_opening_and_before_closing_nonempty_braces: Tristate::True,
        insert_space_after_opening_and_before_closing_template_string_braces: Tristate::False,
        insert_space_after_opening_and_before_closing_jsx_expression_braces: Tristate::False,
        insert_space_before_function_parenthesis: Tristate::False,
        place_open_brace_on_new_line_for_functions: Tristate::False,
        place_open_brace_on_new_line_for_control_blocks: Tristate::False,
        semicolons: SemicolonPreference::Ignore,
        indent_switch_case: Tristate::True,
        ..Default::default()
    }
}
