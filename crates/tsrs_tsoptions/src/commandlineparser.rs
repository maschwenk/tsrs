use rustc_hash::FxHashSet;
use tsrs_ast::{new_compiler_diagnostic, Diagnostic, Node, SourceFile};
use tsrs_core::collections::{OrderedMap, OrderedMapExt};
use tsrs_core::stringutil;
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{BuildOptions, CompilerOptions, WatchOptions, P};
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use tsrs_vfs::FS;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, CompilerOptionsValue};
use crate::declsbuild::TSC_BUILD_OPTION;
use crate::diagnostics::{
    BUILD_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS, WATCH_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS, AlternateModeDiagnostics,
    COMPILER_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS, ParseCommandLineWorkerDiagnostics,
};
use crate::errors::{create_diagnostic_for_invalid_enum_type, get_compiler_option_value_type_string};
use crate::namemap::{get_name_map_from_list, BUILD_NAME_MAP, COMPILER_NAME_MAP, NameMap, WATCH_NAME_MAP};
use crate::parsedbuildcommandline::ParsedBuildCommandLine;
use crate::parsedcommandline::{new_parsed_command_line, ParsedCommandLine};
use crate::parsinghelpers::{
    convert_to_options_with_absolute_paths, parse_compiler_options, BuildOptionsParser, CompilerOptionsParser, WatchOptionsParser,
};
use crate::tsconfigparsing::{convert_map_to_options, validate_json_option_value, COMMAND_LINE_COMPILER_OPTIONS_MAP, ParseConfigHost};

impl CommandLineParser {
    pub(crate) fn alternate_mode(&self) -> Option<&'static AlternateModeDiagnostics> {
        self.worker_diagnostics.did_you_mean.alternate_mode.as_ref()
    }

    pub(crate) fn options_declarations(&self) -> &'static [&'static CommandLineOption] {
        &self.worker_diagnostics.did_you_mean.option_declarations
    }

    pub(crate) fn unknown_option_diagnostic(&self) -> &'static Message {
        self.worker_diagnostics.did_you_mean.unknown_option_diagnostic
    }

    pub(crate) fn unknown_did_you_mean_diagnostic(&self) -> &'static Message {
        self.worker_diagnostics.did_you_mean.unknown_did_you_mean_diagnostic
    }
}

// The Go struct also holds the file system; here it is passed to the methods that read response files.
pub(crate) struct CommandLineParser {
    pub(crate) worker_diagnostics: &'static ParseCommandLineWorkerDiagnostics,
    pub(crate) options_map: NameMap,
    pub(crate) current_directory: String,
    pub(crate) options: OrderedMap<String, CompilerOptionsValue>,
    pub(crate) file_names: Vec<String>,
    pub(crate) errors: Vec<P<Diagnostic>>,
    pub(crate) response_file_stack: FxHashSet<Path>,
}

pub fn parse_command_line(command_line: &[String], host: &dyn ParseConfigHost) -> ParsedCommandLine {
    let parser =
        parse_command_line_worker(&COMPILER_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS, command_line, Some(host.fs()), host.get_current_directory());
    let options = convert_to_options_with_absolute_paths(
        parser.options.clone(),
        &COMMAND_LINE_COMPILER_OPTIONS_MAP,
        host.get_current_directory(),
    );
    let compiler_options = convert_map_to_options(&options, CompilerOptionsParser(CompilerOptions::default())).0;
    let watch_options = convert_map_to_options(&options, WatchOptionsParser(WatchOptions::default())).0;
    let mut result = new_parsed_command_line(
        P::new(compiler_options),
        parser.file_names,
        Vec::new(),
        ComparePathsOptions {
            use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(),
            current_directory: host.get_current_directory().to_string(),
        },
    );
    result.parsed_config.watch_options = Some(watch_options);
    result.errors = parser.errors;
    result.raw = CompilerOptionsValue::Object(parser.options);
    result
}

pub fn parse_build_command_line(command_line: &[String], host: &dyn ParseConfigHost) -> ParsedBuildCommandLine {
    let parser =
        parse_command_line_worker(&BUILD_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS, command_line, Some(host.fs()), host.get_current_directory());
    let mut compiler_options = CompilerOptions::default();
    for (key, value) in parser.options.iter() {
        let build_option = BUILD_NAME_MAP.get(key);
        let is_tsc_build_option = build_option.is_some_and(|o| std::ptr::eq(o, &raw const TSC_BUILD_OPTION));
        let same_as_compiler_option = match (build_option, COMPILER_NAME_MAP.get(key)) {
            (Some(a), Some(b)) => std::ptr::eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if is_tsc_build_option || same_as_compiler_option {
            parse_compiler_options(key, value, &mut compiler_options);
        }
    }
    let mut result = ParsedBuildCommandLine {
        build_options: convert_map_to_options(&parser.options, BuildOptionsParser(BuildOptions::default())).0,
        compiler_options,
        watch_options: convert_map_to_options(&parser.options, WatchOptionsParser(WatchOptions::default())).0,
        projects: parser.file_names,
        errors: parser.errors,
        raw: CompilerOptionsValue::Object(parser.options),

        compare_paths_options: ComparePathsOptions {
            use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(),
            current_directory: host.get_current_directory().to_string(),
        },
        resolved_project_paths: std::sync::OnceLock::new(),
    };

    if result.projects.is_empty() {
        // tsc -b invoked with no extra arguments; act as if invoked with "tsc -b ."
        result.projects.push(".".to_string());
    }

    // Nonsensical combinations
    if result.build_options.clean.is_true() && result.build_options.force.is_true() {
        result.errors.push(new_compiler_diagnostic(&diagnostics::Options_0_and_1_cannot_be_combined, &[&"clean", &"force"]));
    }
    if result.build_options.clean.is_true() && result.build_options.verbose.is_true() {
        result.errors.push(new_compiler_diagnostic(&diagnostics::Options_0_and_1_cannot_be_combined, &[&"clean", &"verbose"]));
    }
    if result.build_options.clean.is_true() && result.compiler_options.watch.is_true() {
        result.errors.push(new_compiler_diagnostic(&diagnostics::Options_0_and_1_cannot_be_combined, &[&"clean", &"watch"]));
    }
    if result.compiler_options.watch.is_true() && result.build_options.dry.is_true() {
        result.errors.push(new_compiler_diagnostic(&diagnostics::Options_0_and_1_cannot_be_combined, &[&"watch", &"dry"]));
    }

    result
}

pub(crate) fn parse_command_line_worker(
    parse_command_line_with_diagnostics: &'static ParseCommandLineWorkerDiagnostics,
    command_line: &[String],
    fs: Option<&dyn FS>,
    current_directory: &str,
) -> CommandLineParser {
    let mut parser = CommandLineParser {
        current_directory: current_directory.to_string(),
        worker_diagnostics: parse_command_line_with_diagnostics,
        options_map: get_name_map_from_list(&[]),
        file_names: Vec::new(),
        options: OrderedMap::default(),
        errors: Vec::new(),
        response_file_stack: FxHashSet::default(),
    };
    parser.options_map = get_name_map_from_list(parser.options_declarations());
    parser.parse_strings(command_line, fs);
    parser
}

impl CommandLineParser {
    pub(crate) fn parse_strings(&mut self, args: &[String], fs: Option<&dyn FS>) {
        let mut i = 0;
        while i < args.len() {
            let s = &args[i];
            i += 1;
            if s.is_empty() {
                continue;
            }
            match s.as_bytes()[0] {
                b'@' => {
                    self.parse_response_file(&s[1..], fs);
                }
                b'-' => {
                    let input_option_name = get_input_option_name(s);
                    let opt = self.options_map.get_option_declaration_from_name(input_option_name, true /*allowShort*/);
                    if let Some(opt) = opt {
                        i = self.parse_option_value(args, i, opt, self.worker_diagnostics.option_type_mismatch_diagnostic);
                    } else {
                        let watch_opt = WATCH_NAME_MAP.get_option_declaration_from_name(input_option_name, true /*allowShort*/);
                        if let Some(watch_opt) = watch_opt {
                            i = self.parse_option_value(
                                args,
                                i,
                                watch_opt,
                                WATCH_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS.option_type_mismatch_diagnostic,
                            );
                        } else {
                            let err = self.create_unknown_option_error(input_option_name, s, None, None);
                            self.errors.push(err);
                        }
                    }
                }
                _ => {
                    self.file_names.push(s.clone());
                }
            }
        }
    }
}

pub(crate) fn get_input_option_name(input: &str) -> &str {
    // removes at most two leading '-' from the input string
    let input = input.strip_prefix('-').unwrap_or(input);
    input.strip_prefix('-').unwrap_or(input)
}

impl CommandLineParser {
    pub(crate) fn parse_response_file(&mut self, file_name: &str, fs: Option<&dyn FS>) {
        let file_name = tspath::get_normalized_absolute_path(file_name, &self.current_directory);
        let use_case_sensitive_file_names = match fs {
            Some(fs) => fs.use_case_sensitive_file_names(),
            None => false,
        };
        let path = tspath::to_path(&file_name, &self.current_directory, use_case_sensitive_file_names);
        if self.response_file_stack.contains(&path) {
            return;
        }
        self.response_file_stack.insert(path.clone());

        let errors = std::mem::take(&mut self.errors);
        let (file_contents, errors) = try_read_file(
            &file_name,
            |file_name| match fs {
                None => None,
                Some(fs) => fs.read_file(file_name),
            },
            errors,
        );
        self.errors = errors;

        if file_contents.is_empty() {
            self.response_file_stack.remove(&path);
            return;
        }

        let mut args: Vec<String> = Vec::new();
        let text: Vec<char> = file_contents.chars().collect();
        let text_length = text.len();
        let mut pos = 0;
        while pos < text_length {
            while pos < text_length && text[pos] <= ' ' {
                pos += 1;
            }
            if pos >= text_length {
                break;
            }
            let start = pos;
            if text[pos] == '"' {
                pos += 1;
                while pos < text_length && text[pos] != '"' {
                    pos += 1;
                }
                if pos < text_length {
                    args.push(text[start + 1..pos].iter().collect());
                    pos += 1;
                } else {
                    self.errors.push(new_compiler_diagnostic(
                        &diagnostics::Unterminated_quoted_string_in_response_file_0,
                        &[&file_name],
                    ));
                }
            } else {
                while pos < text_length && text[pos] > ' ' {
                    pos += 1;
                }
                args.push(text[start..pos].iter().collect());
            }
        }
        self.parse_strings(&args, fs);
        self.response_file_stack.remove(&path);
    }
}

pub(crate) fn try_read_file(
    file_name: &str,
    read_file: impl FnOnce(&str) -> Option<String>,
    mut errors: Vec<P<Diagnostic>>,
) -> (String, Vec<P<Diagnostic>>) {
    // this function adds a compiler diagnostic if the file cannot be read
    let text = match read_file(file_name) {
        Some(text) => text,
        None => {
            // !!! Divergence: the returned error will not give a useful message
            // errors = append(errors, ast.NewCompilerDiagnostic(diagnostics.Cannot_read_file_0_Colon_1, *e));
            errors.push(new_compiler_diagnostic(&diagnostics::Cannot_read_file_0, &[&file_name]));
            String::new()
        }
    };
    (text, errors)
}

impl CommandLineParser {
    pub(crate) fn parse_option_value(
        &mut self,
        args: &[String],
        mut i: usize,
        opt: &'static CommandLineOption,
        diag: &'static Message,
    ) -> usize {
        if opt.is_tsconfig_only && i <= args.len() {
            let opt_value = if i < args.len() { args[i].as_str() } else { "" };
            if opt_value == "null" {
                self.options.set(opt.name.to_string(), CompilerOptionsValue::Null);
                i += 1;
            } else if opt.kind == CommandLineOptionKind::Boolean {
                if opt_value == "false" {
                    self.options.set(opt.name.to_string(), CompilerOptionsValue::Bool(false));
                    i += 1;
                } else {
                    if opt_value == "true" {
                        i += 1;
                    }
                    self.errors.push(new_compiler_diagnostic(
                        &diagnostics::Option_0_can_only_be_specified_in_tsconfig_json_file_or_set_to_false_or_null_on_command_line,
                        &[&opt.name],
                    ));
                }
            } else {
                self.errors.push(new_compiler_diagnostic(
                    &diagnostics::Option_0_can_only_be_specified_in_tsconfig_json_file_or_set_to_null_on_command_line,
                    &[&opt.name],
                ));
                if !opt_value.is_empty() && !opt_value.starts_with('-') {
                    i += 1;
                }
            }
        } else {
            // Check to see if no argument was provided (e.g. "--locale" is the last command-line argument).
            if i >= args.len() {
                if opt.kind != CommandLineOptionKind::Boolean {
                    self.errors.push(new_compiler_diagnostic(diag, &[&opt.name, &get_compiler_option_value_type_string(opt)]));
                    if opt.kind == CommandLineOptionKind::List {
                        self.options.set(opt.name.to_string(), CompilerOptionsValue::StringArray(Vec::new()));
                    } else if opt.kind == CommandLineOptionKind::Enum {
                        self.errors.push(create_diagnostic_for_invalid_enum_type(opt, None, None));
                    }
                } else {
                    self.options.set(opt.name.to_string(), CompilerOptionsValue::Bool(true));
                }
                return i;
            }
            if args[i] != "null" {
                match opt.kind {
                    CommandLineOptionKind::Number => {
                        // !!! Make sure this parseInt matches JS parseInt
                        match args[i].parse::<i64>() {
                            Ok(num) => {
                                if num >= opt.min_value as i64 {
                                    self.options.set(opt.name.to_string(), CompilerOptionsValue::Int(num));
                                } else {
                                    self.errors.push(new_compiler_diagnostic(
                                        &diagnostics::Option_0_requires_value_to_be_greater_than_1,
                                        &[&opt.name, &opt.min_value.to_string()],
                                    ));
                                }
                            }
                            Err(_) => {
                                self.errors.push(new_compiler_diagnostic(diag, &[&opt.name, &"number"]));
                            }
                        }
                        i += 1;
                    }
                    CommandLineOptionKind::Boolean => {
                        // boolean flag has optional value true, false, others
                        let opt_value = args[i].as_str();

                        // check next argument as boolean flag value
                        if opt_value == "false" {
                            self.options.set(opt.name.to_string(), CompilerOptionsValue::Bool(false));
                        } else {
                            self.options.set(opt.name.to_string(), CompilerOptionsValue::Bool(true));
                        }
                        // try to consume next argument as value for boolean flag; do not consume argument if it is not "true" or "false"
                        if opt_value == "false" || opt_value == "true" {
                            i += 1;
                        }
                    }
                    CommandLineOptionKind::String => {
                        let (val, err) = validate_json_option_value(opt, CompilerOptionsValue::String(args[i].clone()), None, None);
                        if err.is_empty() {
                            self.options.set(opt.name.to_string(), val);
                        } else {
                            self.errors.extend(err);
                        }
                        i += 1;
                    }
                    CommandLineOptionKind::List => {
                        let (result, err) = self.parse_list_type_option(opt, &args[i]);
                        let advance = !result.is_empty() || !err.is_empty();
                        self.options.set(opt.name.to_string(), CompilerOptionsValue::Array(result));
                        self.errors.extend(err);
                        if advance {
                            i += 1;
                        }
                    }
                    CommandLineOptionKind::ListOrElement => {
                        // If not a primitive, the possible types are specified in what is effectively a map of options.
                        panic!("listOrElement not supported here");
                    }
                    _ => {
                        let (val, err) =
                            convert_json_option_of_enum_type(opt, args[i].trim_matches(|c: char| stringutil::is_white_space_like(c)), None, None);
                        self.options.set(opt.name.to_string(), val);
                        self.errors.extend(err);
                        i += 1;
                    }
                }
            } else {
                self.options.set(opt.name.to_string(), CompilerOptionsValue::Null);
                i += 1;
            }
        }
        i
    }

    pub(crate) fn parse_list_type_option(
        &self,
        opt: &'static CommandLineOption,
        value: &str,
    ) -> (Vec<CompilerOptionsValue>, Vec<P<Diagnostic>>) {
        parse_list_type_option(opt, value)
    }
}

pub fn parse_list_type_option(opt: &'static CommandLineOption, value: &str) -> (Vec<CompilerOptionsValue>, Vec<P<Diagnostic>>) {
    let value = go_trim_space(value);
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    if value.starts_with('-') {
        return (Vec::new(), errors);
    }
    if opt.kind == CommandLineOptionKind::ListOrElement && !value.contains(',') {
        let (val, err) = validate_json_option_value(opt, CompilerOptionsValue::String(value.to_string()), None, None);
        if !err.is_empty() {
            return (Vec::new(), err);
        }
        return (vec![CompilerOptionsValue::String(val.as_str().unwrap().to_string())], errors);
    }
    if value.is_empty() {
        return (Vec::new(), errors);
    }
    let values: Vec<&str> = value.split(',').collect();
    let elements = opt.elements().unwrap();
    match elements.kind {
        CommandLineOptionKind::String => {
            let mut result = Vec::new();
            for v in values {
                let (val, err) = validate_json_option_value(elements, CompilerOptionsValue::String(v.to_string()), None, None);
                if let CompilerOptionsValue::String(s) = &val {
                    if err.is_empty() && !s.is_empty() {
                        result.push(CompilerOptionsValue::String(s.clone()));
                        continue;
                    }
                }
                errors.extend(err);
            }
            (result, errors)
        }
        CommandLineOptionKind::Boolean | CommandLineOptionKind::Object | CommandLineOptionKind::Number => {
            // do nothing: only string and enum/object types currently allowed as list entries
            // 				!!! we don't actually have number list options, so I didn't implement number list parsing
            panic!("List of {} is not yet supported.", elements.kind.as_str());
        }
        _ => {
            let mut result = Vec::new();
            for v in values {
                let (val, err) =
                    convert_json_option_of_enum_type(elements, v.trim_matches(|c: char| stringutil::is_white_space_like(c)), None, None);
                if let CompilerOptionsValue::String(s) = &val {
                    if err.is_empty() && !s.is_empty() {
                        result.push(CompilerOptionsValue::String(s.clone()));
                        continue;
                    }
                }
                errors.extend(err);
            }
            (result, errors)
        }
    }
}

// strings.TrimSpace: trims Unicode white space (unicode.IsSpace).
pub(crate) fn go_trim_space(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{0085}' | '\u{00A0}') || (c > '\u{00FF}' && c.is_whitespace()))
}

pub(crate) fn convert_json_option_of_enum_type(
    opt: &'static CommandLineOption,
    value: &str,
    value_expression: Option<P<Node>>,
    source_file: Option<P<SourceFile>>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    if value.is_empty() {
        return (CompilerOptionsValue::Null, Vec::new());
    }
    let key = stringutil::go_strings_to_lower(value);
    let Some(type_map) = opt.enum_map() else {
        return (CompilerOptionsValue::Null, Vec::new());
    };
    if let Some(val) = type_map.get(key.as_str()) {
        return validate_json_option_value(opt, val.clone(), value_expression, source_file);
    }
    (CompilerOptionsValue::Null, vec![create_diagnostic_for_invalid_enum_type(opt, source_file, value_expression)])
}
