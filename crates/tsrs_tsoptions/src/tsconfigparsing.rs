use std::fmt::Write as _;
use std::fmt::Display;
use std::sync::LazyLock;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{new_compiler_diagnostic, new_diagnostic, Diagnostic, Kind, Node, NodeFactory, SourceFile, SourceFileParseOptions};
use tsrs_core::collections::{OrderedMap, OrderedMapExt};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{jsnum, CompilerOptions, ProjectReference, ScriptKind, TextRange, Tristate, TypeAcquisition, P};
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use tsrs_vfs::vfsmatch;
use tsrs_vfs::FS;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, CompilerOptionsValue, ExtraValidation};
use crate::commandlineparser::{convert_json_option_of_enum_type, try_read_file};
use crate::contentmappers::{resolve_content_mapper_manifest, Mapper};
use crate::declscompiler::{OPTIONS_FOR_COMPILER, OPTIONS_DECLARATIONS};
use crate::declstypeacquisition::TYPE_ACQUISITION_DECLARATION;
use crate::errors::{
    create_diagnostic_for_node_in_source_file, create_diagnostic_for_node_in_source_file_or_compiler_diagnostic,
    create_unknown_option_error, extra_key_diagnostics, extra_key_did_you_mean_diagnostics, get_compiler_option_value_type_string,
};
use crate::parsedcommandline::ParsedCommandLine;
use crate::parsedoptions::ParsedOptions;
use crate::parsinghelpers::{
    merge_compiler_options, parse_compiler_options, parse_content_mapper, parse_json_to_string_key, parse_project_reference,
    parse_type_acquisition, CompilerOptionsParser, OptionParser, TypeAcquisitionParser,
};

struct ExtendsResult {
    options: CompilerOptions,
    include: Option<Vec<CompilerOptionsValue>>,
    exclude: Option<Vec<CompilerOptionsValue>>,
    files: Option<Vec<CompilerOptionsValue>>,
    content_mappers: Option<Vec<CompilerOptionsValue>>,
    compile_on_save: bool,
    extended_source_files: FxHashSet<String>,
}

pub(crate) static COMPILER_OPTIONS_DECLARATION: LazyLock<CommandLineOption> = LazyLock::new(|| CommandLineOption {
    name: "compilerOptions",
    kind: CommandLineOptionKind::Object,
    element_options: Some(COMMAND_LINE_COMPILER_OPTIONS_MAP.clone()),
    ..CommandLineOption::DEFAULT
});

pub(crate) static COMPILE_ON_SAVE_COMMAND_LINE_OPTION: CommandLineOption = CommandLineOption {
    name: "compileOnSave",
    kind: CommandLineOptionKind::Boolean,
    default_value_description: crate::commandlineoption::DefaultValueDescription::Bool(false),
    ..CommandLineOption::DEFAULT
};

static EXTENDS_OPTION_DECLARATION_ELEMENT: CommandLineOption =
    CommandLineOption { name: "extends", kind: CommandLineOptionKind::String, ..CommandLineOption::DEFAULT };

pub(crate) static EXTENDS_OPTION_DECLARATION: LazyLock<CommandLineOption> = LazyLock::new(|| CommandLineOption {
    name: "extends",
    kind: CommandLineOptionKind::ListOrElement,
    category: Some(&diagnostics::File_Management),
    element_options: Some(command_line_options_to_map(&[&EXTENDS_OPTION_DECLARATION_ELEMENT])),
    ..CommandLineOption::DEFAULT
});

static TSCONFIG_ROOT_OPTIONS_MAP_ITEMS: [CommandLineOption; 5] = [
    CommandLineOption {
        name: "references",
        kind: CommandLineOptionKind::List, // should be a list of projectReference
        // Category: diagnostics.Projects,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "contentMappers",
        kind: CommandLineOptionKind::List, // list of content mapper objects
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "files",
        kind: CommandLineOptionKind::List,
        // Category: diagnostics.File_Management,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "include",
        kind: CommandLineOptionKind::List,
        // Category: diagnostics.File_Management,
        // DefaultValueDescription: diagnostics.if_files_is_specified_otherwise_Asterisk_Asterisk_Slash_Asterisk,
        ..CommandLineOption::DEFAULT
    },
    CommandLineOption {
        name: "exclude",
        kind: CommandLineOptionKind::List,
        // Category: diagnostics.File_Management,
        // DefaultValueDescription: diagnostics.Node_modules_bower_components_jspm_packages_plus_the_value_of_outDir_if_one_is_specified,
        ..CommandLineOption::DEFAULT
    },
];

pub(crate) static TSCONFIG_ROOT_OPTIONS_MAP: LazyLock<CommandLineOption> = LazyLock::new(|| CommandLineOption {
    name: "undefined", // should never be needed since this is root
    kind: CommandLineOptionKind::Object,
    element_options: Some(command_line_options_to_map(&[
        &COMPILER_OPTIONS_DECLARATION,
        &TYPE_ACQUISITION_DECLARATION,
        &EXTENDS_OPTION_DECLARATION,
        &TSCONFIG_ROOT_OPTIONS_MAP_ITEMS[0],
        &TSCONFIG_ROOT_OPTIONS_MAP_ITEMS[1],
        &TSCONFIG_ROOT_OPTIONS_MAP_ITEMS[2],
        &TSCONFIG_ROOT_OPTIONS_MAP_ITEMS[3],
        &TSCONFIG_ROOT_OPTIONS_MAP_ITEMS[4],
        &COMPILE_ON_SAVE_COMMAND_LINE_OPTION,
    ])),
    ..CommandLineOption::DEFAULT
});

#[derive(Clone, Debug, Default)]
pub(crate) struct ConfigFileSpecs {
    pub(crate) files_specs: Option<Vec<CompilerOptionsValue>>,
    // Present to report errors (user specified specs), validatedIncludeSpecs are used for file name matching
    pub(crate) include_specs: Option<Vec<CompilerOptionsValue>>,
    // Present to report errors (user specified specs), validatedExcludeSpecs are used for file name matching
    pub(crate) exclude_specs: Option<Vec<CompilerOptionsValue>>,
    pub(crate) validated_files_spec: Vec<String>,
    pub(crate) validated_include_specs: Vec<String>,
    pub(crate) validated_exclude_specs: Vec<String>,
    pub(crate) validated_files_spec_before_substitution: Vec<String>,
    pub(crate) validated_include_specs_before_substitution: Vec<String>,
    pub(crate) is_default_include_spec: bool,
}

impl ConfigFileSpecs {
    pub(crate) fn matches_exclude(&self, file_name: &str, compare_paths_options: &ComparePathsOptions) -> bool {
        if self.validated_exclude_specs.is_empty() {
            return false;
        }
        let Some(exclude_matcher) = vfsmatch::new_spec_matcher(
            &self.validated_exclude_specs,
            &compare_paths_options.current_directory,
            vfsmatch::Usage::Exclude,
            compare_paths_options.use_case_sensitive_file_names,
        ) else {
            return false;
        };
        if exclude_matcher.match_string(file_name) {
            return true;
        }
        if !tspath::has_extension(file_name) {
            if exclude_matcher.match_string(&tspath::ensure_trailing_directory_separator(file_name)) {
                return true;
            }
        }
        false
    }

    pub(crate) fn get_matched_include_spec(&self, file_name: &str, compare_paths_options: &ComparePathsOptions) -> String {
        if self.validated_include_specs.is_empty() {
            return String::new();
        }
        for (index, spec) in self.validated_include_specs.iter().enumerate() {
            let include_matcher = vfsmatch::new_spec_matcher(
                std::slice::from_ref(spec),
                &compare_paths_options.current_directory,
                vfsmatch::Usage::Files,
                compare_paths_options.use_case_sensitive_file_names,
            );
            if let Some(include_matcher) = include_matcher {
                if include_matcher.match_string(file_name) {
                    return self.validated_include_specs_before_substitution[index].clone();
                }
            }
        }
        String::new()
    }

    pub(crate) fn get_matched_file_spec(&self, file_name: &str, compare_paths_options: &ComparePathsOptions) -> String {
        if self.validated_files_spec.is_empty() {
            return String::new();
        }
        let file_path = tspath::to_path(
            file_name,
            &compare_paths_options.current_directory,
            compare_paths_options.use_case_sensitive_file_names,
        );
        for (index, spec) in self.validated_files_spec.iter().enumerate() {
            if tspath::to_path(spec, &compare_paths_options.current_directory, compare_paths_options.use_case_sensitive_file_names)
                == file_path
            {
                return self.validated_files_spec_before_substitution[index].clone();
            }
        }
        String::new()
    }
}

pub trait ExtendedConfigCache: Sync {
    fn get_extended_config(
        &self,
        file_name: &str,
        path: &Path,
        resolution_stack: &[Path],
        host: &'static dyn ParseConfigHost,
    ) -> P<ExtendedConfigCacheEntry>;
}

pub struct ExtendedConfigCacheEntry {
    pub(crate) extended_result: Option<P<TsConfigSourceFile>>,
    pub(crate) extended_config: Option<ParsedTsconfig>,
    pub(crate) errors: Vec<P<Diagnostic>>,
}

impl ExtendedConfigCacheEntry {
    pub fn extended_file_names(&self) -> Vec<String> {
        if let Some(extended_result) = self.extended_result {
            return extended_result.extended_source_files.borrow().clone();
        }
        Vec::new()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedTsconfig {
    pub(crate) raw: CompilerOptionsValue,
    pub(crate) options: Option<CompilerOptions>,
    pub(crate) type_acquisition: Option<TypeAcquisition>,
    // Note that the case of the config path has not yet been normalized, as no files have been imported into the project yet
    pub(crate) extended_config_path: Option<Vec<String>>,
}

fn parse_own_config_of_json_source_file(
    source_file: P<SourceFile>,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    config_file_name: &str,
) -> (ParsedTsconfig, Vec<P<Diagnostic>>) {
    let mut compiler_options = get_default_compiler_options(config_file_name);
    let mut type_acquisition = get_default_type_acquisition(config_file_name);
    let mut extended_config_path: Option<Vec<String>> = None;
    let mut root_compiler_options: Vec<P<Node>> = Vec::new();
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    let mut on_property_set = |key_text: &str,
                               value: CompilerOptionsValue,
                               property_assignment: P<Node>,
                               parent_option: Option<&'static CommandLineOption>, // TsConfigOnlyOption,
                               option: Option<&'static CommandLineOption>|
     -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
        // Ensure value is verified except for extends which is handled in its own way for error reporting
        let mut value = value;
        let mut property_set_errors: Vec<P<Diagnostic>> = Vec::new();
        if let Some(option) = option {
            if !std::ptr::eq(option, &*EXTENDS_OPTION_DECLARATION) {
                let (v, e) = convert_json_option(
                    option,
                    value,
                    base_path,
                    Some(property_assignment),
                    Some(initializer_of(property_assignment)),
                    Some(source_file),
                );
                value = v;
                property_set_errors = e;
            }
        }
        if parent_option.is_some_and(|p| p.name != "undefined") && !value.is_null() {
            let parent_option = parent_option.unwrap();
            if let Some(option) = option.filter(|o| !o.name.is_empty()) {
                let mut parse_diagnostics = Vec::new();
                match parent_option.name {
                    "compilerOptions" => {
                        parse_diagnostics = parse_compiler_options(option.name, &value, &mut compiler_options);
                    }
                    "typeAcquisition" => {
                        parse_diagnostics = parse_type_acquisition(option.name, &value, &mut type_acquisition);
                    }
                    _ => {}
                }
                property_set_errors.extend(parse_diagnostics);
            } else if !key_text.is_empty() && extra_key_diagnostics(parent_option.name).is_some() {
                let unknown_name_diag = extra_key_diagnostics(parent_option.name).unwrap();
                if let Some(element_options) = &parent_option.element_options {
                    let mut possible_option = element_options.get(key_text);
                    if possible_option.is_none() {
                        possible_option = element_options.get_spelling_suggestion(key_text);
                    }
                    if let Some(possible_option) = possible_option.filter(|o| o.name != key_text) {
                        property_set_errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                            Some(source_file),
                            property_assignment.name(),
                            extra_key_did_you_mean_diagnostics(parent_option.name).unwrap(),
                            &[&key_text, &possible_option.name],
                        ));
                    } else {
                        property_set_errors.push(create_unknown_option_error(
                            key_text,
                            unknown_name_diag,
                            "", /*unknownOptionErrorText*/
                            property_assignment.name(),
                            Some(source_file),
                            None, /*alternateMode*/
                            None, /*unknownDidYouMeanDiagnostic*/
                            None, /*optionsNameMap*/
                        ));
                    }
                } else {
                    // errors = append(errors, ast.NewCompilerDiagnostic(diagnostics.Unknown_compiler_option_0_Did_you_mean_1, keyText, core.FindKey(parentOption.ElementOptions, keyText)))
                }
            }
        } else if parent_option.is_some_and(|p| std::ptr::eq(p, &*TSCONFIG_ROOT_OPTIONS_MAP)) {
            if option.is_some_and(|o| std::ptr::eq(o, &*EXTENDS_OPTION_DECLARATION)) {
                let (config_path, err) = get_extends_config_path_or_array(
                    &value,
                    host,
                    base_path,
                    config_file_name,
                    Some(property_assignment),
                    Some(initializer_of(property_assignment)),
                    Some(source_file),
                );
                extended_config_path = Some(config_path);
                property_set_errors.extend(err);
            } else if option.is_none() {
                if key_text == "excludes" {
                    property_set_errors.push(create_diagnostic_for_node_in_source_file(
                        source_file,
                        property_assignment.name().unwrap(),
                        &diagnostics::Unknown_option_excludes_Did_you_mean_exclude,
                        &[],
                    ));
                }
                if OPTIONS_FOR_COMPILER.iter().any(|option| option.name == key_text) {
                    root_compiler_options.push(property_assignment.name().unwrap());
                }
            }
        }
        (value, property_set_errors)
    };

    let (json, err) = convert_config_file_to_object(
        source_file,
        Some(&mut JsonConversionNotifier { root_options: &TSCONFIG_ROOT_OPTIONS_MAP, on_property_set: &mut on_property_set }),
    );
    errors.extend(err);
    if let CompilerOptionsValue::Object(json_object) = &json {
        if !root_compiler_options.is_empty() && !json_object.has(&"compilerOptions".to_string()) {
            errors.push(create_diagnostic_for_node_in_source_file(
                source_file,
                root_compiler_options[0],
                &diagnostics::X_0_should_be_set_inside_the_compilerOptions_object_of_the_config_json_file,
                &[&tsrs_ast::get_text_of_property_name(root_compiler_options[0])],
            ));
        }
    }
    (
        ParsedTsconfig {
            raw: json,
            options: Some(compiler_options),
            type_acquisition: Some(type_acquisition),
            extended_config_path,
        },
        errors,
    )
}

fn initializer_of(property_assignment: P<Node>) -> P<Node> {
    property_assignment.initializer().unwrap()
}

// Filled while the config is parsed; read-only (and read by checker threads) afterwards.
pub struct TsConfigSourceFile {
    pub extended_source_files: tsrs_core::FrozenCell<Vec<String>>,
    pub(crate) config_file_specs: tsrs_core::FrozenCell<Option<ConfigFileSpecs>>,
    pub source_file: P<SourceFile>,
}

impl TsConfigSourceFile {
    pub fn new(source_file: P<SourceFile>) -> P<TsConfigSourceFile> {
        P::new(TsConfigSourceFile {
            extended_source_files: tsrs_core::FrozenCell::new(Vec::new()),
            config_file_specs: tsrs_core::FrozenCell::new(None),
            source_file,
        })
    }
}

pub(crate) fn tsconfig_to_source_file(tsconfig_source_file: Option<P<TsConfigSourceFile>>) -> Option<P<SourceFile>> {
    tsconfig_source_file.map(|f| f.source_file)
}

pub fn new_tsconfig_source_file_from_file_path(config_file_name: &str, config_path: Path, config_source_text: &str) -> P<TsConfigSourceFile> {
    let source_file = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: config_file_name.to_string(), path: config_path, ..Default::default() },
        config_source_text,
        ScriptKind::JSON,
    );
    TsConfigSourceFile::new(source_file)
}

pub(crate) struct JsonConversionNotifier<'a> {
    root_options: &'static CommandLineOption,
    on_property_set: &'a mut dyn FnMut(
        &str,
        CompilerOptionsValue,
        P<Node>,
        Option<&'static CommandLineOption>,
        Option<&'static CommandLineOption>,
    ) -> (CompilerOptionsValue, Vec<P<Diagnostic>>),
}

pub(crate) fn convert_config_file_to_object(
    source_file: P<SourceFile>,
    json_conversion_notifier: Option<&mut JsonConversionNotifier>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    let statements = source_file.as_node().statements();
    let root_expression = if !statements.is_empty() { statements[0].expression() } else { None };
    if let Some(root_expression) = root_expression {
        if root_expression.kind() != Kind::ObjectLiteralExpression {
            let mut base_file_name = "tsconfig.json";
            if tspath::get_base_file_name(source_file.file_name()) == "jsconfig.json" {
                base_file_name = "jsconfig.json";
            }
            let errors = vec![create_diagnostic_for_node_in_source_file(
                source_file,
                root_expression,
                &diagnostics::The_root_value_of_a_0_file_must_be_an_object,
                &[&base_file_name],
            )];
            // Last-ditch error recovery. Somewhat useful because the JSON parser will recover from some parse errors by
            // synthesizing a top-level array literal expression. There's a reasonable chance the first element of that
            // array is a well-formed configuration object, made into an array element by stray characters.
            if tsrs_ast::is_array_literal_expression(root_expression) {
                let first_object = root_expression.elements().iter().copied().find(|e| tsrs_ast::is_object_literal_expression(*e));
                if let Some(first_object) = first_object {
                    return convert_to_json(source_file, Some(first_object), true /*returnValue*/, json_conversion_notifier);
                }
            }
            return (CompilerOptionsValue::Object(OrderedMap::default()), errors);
        }
    }
    convert_to_json(source_file, root_expression, true, json_conversion_notifier)
}

pub(crate) fn is_compiler_options_value(option: Option<&CommandLineOption>, value: &CompilerOptionsValue) -> bool {
    if let Some(option) = option {
        if value.is_null() {
            return !option.disallow_null_or_undefined();
        }
        if option.kind == CommandLineOptionKind::List {
            return value.is_slice();
        }
        if option.kind == CommandLineOptionKind::ListOrElement {
            if value.is_slice() {
                return true;
            } else {
                return is_compiler_options_value(option.elements(), value);
            }
        }
        if option.kind == CommandLineOptionKind::String {
            return value.is_string();
        }
        if option.kind == CommandLineOptionKind::Boolean {
            return matches!(value, CompilerOptionsValue::Bool(_));
        }
        if option.kind == CommandLineOptionKind::Number {
            return matches!(value, CompilerOptionsValue::Float(_));
        }
        if option.kind == CommandLineOptionKind::Object {
            return matches!(value, CompilerOptionsValue::Object(_));
        }
        if option.kind == CommandLineOptionKind::Enum && value.is_string() {
            return true;
        }
    }
    false
}

pub(crate) fn validate_json_option_value(
    opt: &CommandLineOption,
    val: CompilerOptionsValue,
    value_expression: Option<P<Node>>,
    source_file: Option<P<SourceFile>>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    if val.is_null() {
        return (CompilerOptionsValue::Null, Vec::new());
    }

    let mut errors: Vec<P<Diagnostic>> = Vec::new();

    match opt.extra_validation {
        ExtraValidation::Spec => {
            if let Some(diag) = spec_to_diagnostic(val.as_str().unwrap(), false) {
                errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(source_file, value_expression, diag, &[]));
            }
        }
        ExtraValidation::Locale => {
            if !locale_parse(val.as_str().unwrap()) {
                errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                    source_file,
                    value_expression,
                    &diagnostics::Locale_must_be_an_IETF_BCP_47_language_tag_Examples_Colon_0_1,
                    &[&"en", &"ja-jp"],
                ));
            }
        }
        ExtraValidation::None => {}
    }

    if !errors.is_empty() {
        return (CompilerOptionsValue::Null, errors);
    }
    (val, Vec::new())
}

fn convert_json_option_of_list_type(
    option: &'static CommandLineOption,
    values: CompilerOptionsValue,
    base_path: &str,
    property_assignment: Option<P<Node>>,
    value_expression: Option<P<Node>>,
    source_file: Option<P<SourceFile>>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    let mut expression: Option<P<Node>> = None;
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    match values {
        CompilerOptionsValue::Array(values) => {
            let mut mapped_values = Vec::with_capacity(values.len());
            for (index, v) in values.into_iter().enumerate() {
                if let Some(value_expression) = value_expression {
                    expression = Some(value_expression.elements()[index]);
                }
                let (result, err) =
                    convert_json_option(option.elements().unwrap(), v, base_path, property_assignment, expression, source_file);
                errors.extend(err);
                mapped_values.push(result);
            }
            let mut filtered_values = mapped_values;
            if !option.list_preserve_falsy_values {
                filtered_values.retain(|v| {
                    !v.is_null()
                        && *v != CompilerOptionsValue::Bool(false)
                        && *v != CompilerOptionsValue::Int(0)
                        && *v != CompilerOptionsValue::String(String::new())
                });
            }
            (CompilerOptionsValue::Array(filtered_values), errors)
        }
        CompilerOptionsValue::NilArray => (CompilerOptionsValue::NilArray, errors),
        _ => (CompilerOptionsValue::NilArray, errors),
    }
}

const CONFIG_DIR_TEMPLATE: &str = "${configDir}";

pub(crate) fn starts_with_config_dir_template(value: &str) -> bool {
    tsrs_core::stringutil::go_strings_to_lower(value).starts_with(&tsrs_core::stringutil::go_strings_to_lower(CONFIG_DIR_TEMPLATE))
}

fn starts_with_config_dir_template_value(value: &CompilerOptionsValue) -> bool {
    match value {
        CompilerOptionsValue::String(str) => starts_with_config_dir_template(str),
        _ => false,
    }
}

fn normalize_non_list_option_value(option: &CommandLineOption, base_path: &str, value: CompilerOptionsValue) -> CompilerOptionsValue {
    if option.is_file_path {
        let mut value = tspath::normalize_slashes(value.as_str().unwrap());
        if !starts_with_config_dir_template(&value) {
            value = tspath::get_normalized_absolute_path(&value, base_path);
        }
        if value.is_empty() {
            value = ".".to_string();
        }
        return CompilerOptionsValue::String(value);
    }
    value
}

pub(crate) fn convert_json_option(
    opt: &'static CommandLineOption,
    value: CompilerOptionsValue,
    base_path: &str,
    property_assignment: Option<P<Node>>,
    value_expression: Option<P<Node>>,
    source_file: Option<P<SourceFile>>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    if opt.is_command_line_only {
        let node_value = property_assignment.and_then(|p| p.name());
        if source_file.is_none() && node_value.is_none() {
            return (
                CompilerOptionsValue::Null,
                vec![new_compiler_diagnostic(&diagnostics::Option_0_can_only_be_specified_on_command_line, &[&opt.name])],
            );
        } else {
            return (
                CompilerOptionsValue::Null,
                vec![create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                    source_file,
                    node_value,
                    &diagnostics::Option_0_can_only_be_specified_on_command_line,
                    &[&opt.name],
                )],
            );
        }
    }
    if is_compiler_options_value(Some(opt), &value) {
        match opt.kind {
            CommandLineOptionKind::List => {
                return convert_json_option_of_list_type(opt, value, base_path, property_assignment, value_expression, source_file);
                // as ArrayLiteralExpression | undefined
            }
            CommandLineOptionKind::ListOrElement => {
                if value.is_slice() {
                    return convert_json_option_of_list_type(opt, value, base_path, property_assignment, value_expression, source_file);
                } else {
                    return convert_json_option(opt.elements().unwrap(), value, base_path, property_assignment, value_expression, source_file);
                }
            }
            CommandLineOptionKind::Enum => {
                if value.is_null() {
                    return (CompilerOptionsValue::Null, Vec::new());
                }
                return convert_json_option_of_enum_type(opt, value.as_str().unwrap(), value_expression, source_file);
            }
            _ => {}
        }

        let (validated_value, errors) = validate_json_option_value(opt, value, value_expression, source_file);
        if !errors.is_empty() || validated_value.is_null() {
            (validated_value, errors)
        } else {
            (normalize_non_list_option_value(opt, base_path, validated_value), errors)
        }
    } else {
        (
            CompilerOptionsValue::Null,
            vec![create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                source_file,
                value_expression,
                &diagnostics::Compiler_option_0_requires_a_value_of_type_1,
                &[&opt.name, &get_compiler_option_value_type_string(opt)],
            )],
        )
    }
}

fn get_extends_config_path_or_array(
    value: &CompilerOptionsValue,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    config_file_name: &str,
    property_assignment: Option<P<Node>>,
    value_expression: Option<P<Node>>,
    source_file: Option<P<SourceFile>>,
) -> (Vec<String>, Vec<P<Diagnostic>>) {
    let mut extended_config_path_array: Vec<String> = Vec::new();
    let mut new_base = base_path.to_string();
    if !config_file_name.is_empty() {
        new_base = directory_of_combined_path(config_file_name, base_path);
    }
    if value.is_null() {
        let (_, errors) =
            convert_json_option(&EXTENDS_OPTION_DECLARATION, value.clone(), base_path, property_assignment, value_expression, source_file);
        return (extended_config_path_array, errors);
    }
    if let CompilerOptionsValue::String(value) = value {
        let (val, err) = get_extends_config_path(value, host, &new_base, value_expression, source_file);
        if !val.is_empty() {
            extended_config_path_array.push(val);
        }
        return (extended_config_path_array, err);
    }
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    if value.is_slice() {
        let items: Vec<CompilerOptionsValue> = match value {
            CompilerOptionsValue::Array(a) => a.clone(),
            CompilerOptionsValue::StringArray(a) => a.iter().map(|s| CompilerOptionsValue::String(s.clone())).collect(),
            _ => Vec::new(),
        };
        for (index, file_name) in items.iter().enumerate() {
            let mut expression: Option<P<Node>> = None;
            if let Some(value_expression) = value_expression {
                expression = Some(value_expression.elements()[index]);
            }
            if let CompilerOptionsValue::String(file_name) = file_name {
                let (val, err) = get_extends_config_path(file_name, host, &new_base, expression, source_file);
                if !val.is_empty() {
                    extended_config_path_array.push(val);
                }
                errors.extend(err);
            } else {
                let (_, err) = convert_json_option(
                    EXTENDS_OPTION_DECLARATION.elements().unwrap(),
                    value.clone(),
                    base_path,
                    property_assignment,
                    expression,
                    source_file,
                );
                errors.extend(err);
            }
        }
    } else {
        let (_, err) =
            convert_json_option(&EXTENDS_OPTION_DECLARATION, value.clone(), base_path, property_assignment, value_expression, source_file);
        errors = err;
    }
    (extended_config_path_array, errors)
}

fn get_extends_config_path(
    extended_config: &str,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    value_expression: Option<P<Node>>,
    source_file: Option<P<SourceFile>>,
) -> (String, Vec<P<Diagnostic>>) {
    let extended_config = tspath::normalize_slashes(extended_config);
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    let error_file = source_file;
    if tspath::is_rooted_disk_path(&extended_config) || extended_config.starts_with("./") || extended_config.starts_with("../") {
        let mut extended_config_path = tspath::get_normalized_absolute_path(&extended_config, base_path);
        if !host.fs().file_exists(&extended_config_path) && !extended_config_path.ends_with(tspath::EXTENSION_JSON) {
            extended_config_path = extended_config_path + tspath::EXTENSION_JSON;
            if !host.fs().file_exists(&extended_config_path) {
                errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                    error_file,
                    value_expression,
                    &diagnostics::File_0_not_found,
                    &[&extended_config],
                ));
                return (String::new(), errors);
            }
        }
        return (extended_config_path, errors);
    }
    // If the path isn't a rooted or relative path, resolve like a module
    let resolver_host: &'static ResolverHost = P::new(ResolverHost { host }).get();
    let resolved = tsrs_module::resolve_config(&extended_config, &tspath::combine_paths(base_path, &["tsconfig.json"]), resolver_host);
    if resolved.is_resolved() {
        return (resolved.resolved_file_name.to_string(), errors);
    }
    if extended_config.is_empty() {
        errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
            error_file,
            value_expression,
            &diagnostics::Compiler_option_0_cannot_be_given_an_empty_string,
            &[&"extends"],
        ));
    } else {
        errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
            error_file,
            value_expression,
            &diagnostics::File_0_not_found,
            &[&extended_config],
        ));
    }
    (String::new(), errors)
}

// Go `map[string]*CommandLineOption`.
#[derive(Clone, Debug, Default)]
pub struct CommandLineOptionNameMap(pub FxHashMap<&'static str, &'static CommandLineOption>);

impl CommandLineOptionNameMap {
    pub fn get(&self, name: &str) -> Option<&'static CommandLineOption> {
        match self.0.get(name) {
            Some(opt) => Some(*opt),
            None => self.0.get(tsrs_core::stringutil::go_strings_to_lower(name).as_str()).copied(),
        }
    }

    pub fn get_spelling_suggestion(&self, name: &str) -> Option<&'static CommandLineOption> {
        tsrs_core::get_spelling_suggestion(name, self.0.values().copied(), |option| option.name, |a, b| a.name.cmp(b.name) as i32)
    }
}

pub(crate) fn command_line_options_to_map(compiler_options: &[&'static CommandLineOption]) -> CommandLineOptionNameMap {
    let mut result = FxHashMap::with_capacity_and_hasher(compiler_options.len() * 2, Default::default());
    for option in compiler_options {
        result.insert(option.name, *option);
        let lower: &'static str = tsrs_core::alloc_str(&tsrs_core::stringutil::go_strings_to_lower(option.name));
        result.insert(lower, *option);
    }
    CommandLineOptionNameMap(result)
}

pub static COMMAND_LINE_COMPILER_OPTIONS_MAP: LazyLock<CommandLineOptionNameMap> =
    LazyLock::new(|| command_line_options_to_map(&OPTIONS_DECLARATIONS));

pub(crate) fn convert_map_to_options<O: OptionParser>(compiler_options: &OrderedMap<String, CompilerOptionsValue>, mut result: O) -> O {
    // this assumes any `key`, `value` pair in `options` will have `value` already be the correct type. this function should no error handling
    for (key, value) in compiler_options.iter() {
        result.parse_option(key, value);
    }
    result
}

fn convert_options_from_json<O: OptionParser>(
    options_name_map: &CommandLineOptionNameMap,
    json_options: &CompilerOptionsValue,
    base_path: &str,
    mut result: O,
) -> (O, Vec<P<Diagnostic>>) {
    if json_options.is_null() {
        return (result, Vec::new());
    }
    let CompilerOptionsValue::Object(json_map) = json_options else {
        // !!! probably should be an error
        return (result, Vec::new());
    };
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    for (key, value) in json_map.iter() {
        let opt = options_name_map.get(key);
        if let Some(opt) = opt {
            if opt.name != key {
                // Case-insensitive match found but exact case doesn't match - provide "did you mean" suggestion
                errors.push(create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(
                    None,
                    None,
                    result.unknown_did_you_mean_diagnostic(),
                    &[key, &opt.name],
                ));
                continue;
            }
        }
        let Some(opt) = opt else {
            errors.push(create_unknown_option_error(
                key,
                result.unknown_option_diagnostic(),
                "",
                None,
                None,
                None,
                Some(result.unknown_did_you_mean_diagnostic()),
                Some(options_name_map),
            ));
            continue;
        };

        let (convert_json, err) = convert_json_option(opt, value.clone(), base_path, None, None, None);
        errors.extend(err);
        let compiler_options_err = result.parse_option(key, &convert_json);
        errors.extend(compiler_options_err);
    }
    (result, errors)
}

fn convert_array_literal_expression_to_json(
    source_file: P<SourceFile>,
    elements: &[P<Node>],
    element_option: Option<&'static CommandLineOption>,
    return_value: bool,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    if !return_value {
        for element in elements {
            convert_property_value_to_json(source_file, *element, element_option, return_value, None);
        }
        return (CompilerOptionsValue::Null, Vec::new());
    }
    // Filter out invalid values
    if elements.is_empty() {
        // Always return an empty array, even if elements is nil.
        // The parser will produce nil slices instead of allocating empty ones.
        return (CompilerOptionsValue::Array(Vec::new()), Vec::new());
    }
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    let mut value: Option<Vec<CompilerOptionsValue>> = None;
    for element in elements {
        let (converted_value, err) = convert_property_value_to_json(source_file, *element, element_option, return_value, None);
        errors.extend(err);
        if !converted_value.is_null() {
            value.get_or_insert_with(Vec::new).push(converted_value);
        }
    }
    match value {
        Some(value) => (CompilerOptionsValue::Array(value), errors),
        None => (CompilerOptionsValue::NilArray, errors),
    }
}

fn directory_of_combined_path(file_name: &str, base_path: &str) -> String {
    // Use the `getNormalizedAbsolutePath` function to avoid canonicalizing the path, as it must remain noncanonical
    // until consistent casing errors are reported
    tspath::get_directory_path(&tspath::get_normalized_absolute_path(file_name, base_path))
}

// ParseConfigFileTextToJson parses the text of the tsconfig.json file
// fileName is the path to the config file
// jsonText is the text of the config file
pub fn parse_config_file_text_to_json(file_name: &str, path: Path, json_text: &str) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    let json_source_file = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: file_name.to_string(), path, ..Default::default() },
        json_text,
        ScriptKind::JSON,
    );
    let (config, mut errors) = convert_config_file_to_object(json_source_file, /*jsonConversionNotifier*/ None);
    let parse_diagnostics = json_source_file.diagnostics();
    if !parse_diagnostics.is_empty() {
        errors = vec![parse_diagnostics[0]];
    }
    (config, errors)
}

pub trait ParseConfigHost: Send + Sync {
    fn fs(&self) -> &dyn FS;
    fn get_current_directory(&self) -> &str;
}

struct ResolverHost {
    host: &'static dyn ParseConfigHost,
}

impl tsrs_module::ResolutionHost for ResolverHost {
    fn fs(&self) -> &dyn FS {
        self.host.fs()
    }

    fn get_current_directory(&self) -> &str {
        self.host.get_current_directory()
    }
}

impl ResolverHost {
    fn trace(&self, msg: &str) {}
}

pub fn parse_json_source_file_config_file_content(
    source_file: P<TsConfigSourceFile>,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    existing_options: Option<&CompilerOptions>,
    existing_options_raw: Option<&CompilerOptionsValue>,
    config_file_name: &str,
    resolution_stack: &[Path],
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> ParsedCommandLine {
    // tracing?.push(tracing.Phase.Parse, "parseJsonSourceFileConfigFileContent", { path: sourceFile.fileName });
    // tracing?.pop();
    parse_json_config_file_content_worker(
        None, /*json*/
        Some(source_file),
        host,
        base_path,
        existing_options,
        existing_options_raw,
        config_file_name,
        resolution_stack,
        extended_config_cache,
    )
}

fn convert_object_literal_expression_to_json(
    source_file: P<SourceFile>,
    return_value: bool,
    node: P<Node>,
    object_option: Option<&'static CommandLineOption>,
    mut json_conversion_notifier: Option<&mut JsonConversionNotifier>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    let mut result: Option<OrderedMap<String, CompilerOptionsValue>> = if return_value { Some(OrderedMap::default()) } else { None };
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    for element in node.as_object_literal_expression().properties.nodes().iter().copied() {
        if element.kind() != Kind::PropertyAssignment {
            errors.push(new_diagnostic(Some(source_file), element.loc(), &diagnostics::Property_assignment_expected, &[]));
            continue;
        }

        if let Some(token) = element.question_token() {
            errors.push(new_diagnostic(
                Some(source_file),
                token.loc(),
                &diagnostics::The_0_modifier_can_only_be_used_in_TypeScript_files,
                &[&"?"],
            ));
        }
        let mut text_of_key = String::new();
        let name = element.name().unwrap();
        if !tsrs_ast::is_computed_non_literal_name(name) {
            text_of_key = tsrs_ast::try_get_text_of_property_name(name).map(|s| s).unwrap_or_default();
        }
        let key_text = text_of_key;
        let mut option: Option<&'static CommandLineOption> = None;
        if !key_text.is_empty() {
            if let Some(object_option) = object_option {
                if let Some(element_options) = &object_option.element_options {
                    option = element_options.get(&key_text);
                    if option.is_some_and(|o| o.name != key_text) {
                        option = None;
                    }
                }
            }
        }
        let (value, err) = convert_property_value_to_json(
            source_file,
            element.initializer().unwrap(),
            option,
            return_value,
            json_conversion_notifier.as_deref_mut(),
        );
        errors.extend(err);
        if !key_text.is_empty() {
            let notified_value = if json_conversion_notifier.is_some() { Some(value.clone()) } else { None };
            if let Some(result) = &mut result {
                result.set(key_text.clone(), value);
            }
            // Notify key value set, if user asked for it
            if let Some(notifier) = json_conversion_notifier.as_deref_mut() {
                let (_, err) = (notifier.on_property_set)(&key_text, notified_value.unwrap(), element, object_option, option);
                errors.extend(err);
            }
        }
    }
    match result {
        Some(result) => (CompilerOptionsValue::Object(result), errors),
        None => (CompilerOptionsValue::Null, errors),
    }
}

// convertToJson converts the json syntax tree into the json value and report errors
// This returns the json value (apart from checking errors) only if returnValue provided is true.
// Otherwise it just checks the errors and returns undefined
pub(crate) fn convert_to_json(
    source_file: P<SourceFile>,
    root_expression: Option<P<Node>>,
    return_value: bool,
    json_conversion_notifier: Option<&mut JsonConversionNotifier>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    let Some(root_expression) = root_expression else {
        if return_value {
            return (CompilerOptionsValue::EmptyStruct, Vec::new());
        } else {
            return (CompilerOptionsValue::Null, Vec::new());
        }
    };
    let root_options = json_conversion_notifier.as_ref().map(|n| n.root_options);
    convert_property_value_to_json(source_file, root_expression, root_options, return_value, json_conversion_notifier)
}

fn is_double_quoted_string(node: P<Node>) -> bool {
    tsrs_ast::is_string_literal(node)
}

fn convert_property_value_to_json(
    source_file: P<SourceFile>,
    value_expression: P<Node>,
    option: Option<&'static CommandLineOption>,
    return_value: bool,
    json_conversion_notifier: Option<&mut JsonConversionNotifier>,
) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    match value_expression.kind() {
        Kind::TrueKeyword => return (CompilerOptionsValue::Bool(true), Vec::new()),
        Kind::FalseKeyword => return (CompilerOptionsValue::Bool(false), Vec::new()),
        Kind::NullKeyword => return (CompilerOptionsValue::Null, Vec::new()), // todo: how to manage null

        Kind::StringLiteral => {
            if !is_double_quoted_string(value_expression) {
                return (
                    CompilerOptionsValue::String(value_expression.text().to_string()),
                    vec![new_diagnostic(
                        Some(source_file),
                        value_expression.loc(),
                        &diagnostics::String_literal_with_double_quotes_expected,
                        &[],
                    )],
                );
            }
            return (CompilerOptionsValue::String(value_expression.text().to_string()), Vec::new());
        }

        Kind::NumericLiteral => {
            return (CompilerOptionsValue::Float(jsnum::from_string(value_expression.text()).0), Vec::new());
        }
        Kind::PrefixUnaryExpression => {
            let prefix = value_expression.as_prefix_unary_expression();
            if prefix.operator == Kind::MinusToken && prefix.operand.kind() == Kind::NumericLiteral {
                return (CompilerOptionsValue::Float(-jsnum::from_string(prefix.operand.text()).0), Vec::new());
            }
            // not valid JSON syntax
        }
        Kind::ObjectLiteralExpression => {
            // Currently having element option declaration in the tsconfig with type "object"
            // determines if it needs onSetValidOptionKeyValueInParent callback or not
            // At moment there are only "compilerOptions", "typeAcquisition" and "typingOptions"
            // that satisfies it and need it to modify options set in them (for normalizing file paths)
            // vs what we set in the json
            // If need arises, we can modify this interface and callbacks as needed
            return convert_object_literal_expression_to_json(source_file, return_value, value_expression, option, json_conversion_notifier);
        }
        Kind::ArrayLiteralExpression => {
            return convert_array_literal_expression_to_json(source_file, value_expression.elements(), option, return_value);
        }
        _ => {}
    }
    // Not in expected format
    let errors = if let Some(option) = option {
        vec![new_diagnostic(
            Some(source_file),
            value_expression.loc(),
            &diagnostics::Compiler_option_0_requires_a_value_of_type_1,
            &[&option.name, &get_compiler_option_value_type_string(option)],
        )]
    } else {
        vec![new_diagnostic(
            Some(source_file),
            value_expression.loc(),
            &diagnostics::Property_value_can_only_be_string_literal_numeric_literal_true_false_null_object_literal_or_array_literal,
            &[],
        )]
    };
    (CompilerOptionsValue::Null, errors)
}

// ParseJsonConfigFileContent parses the contents of a config file (tsconfig.json).
// jsonNode: The contents of the config file to parse
// host: Instance of ParseConfigHost used to enumerate files in folder.
// basePath: A root directory to resolve relative path entries in the config file to. e.g. outDir
pub fn parse_json_config_file_content(
    json: CompilerOptionsValue,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    existing_options: Option<&CompilerOptions>,
    config_file_name: &str,
    resolution_stack: &[Path],
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> ParsedCommandLine {
    let normalized = normalize_json_value(json);
    let json_object = match normalized {
        CompilerOptionsValue::Object(m) => m,
        _ => OrderedMap::default(),
    };
    parse_json_config_file_content_worker(
        Some(json_object),
        None, /*sourceFile*/
        host,
        base_path,
        existing_options,
        None, /*existingOptionsRaw*/
        config_file_name,
        resolution_stack,
        extended_config_cache,
    )
}

// Go normalizes arbitrary Go values (map[string]any, typed slices) into the JSON value shapes; the
// Rust value type already only has those shapes, so only the recursion over containers remains.
fn normalize_json_value(value: CompilerOptionsValue) -> CompilerOptionsValue {
    match value {
        CompilerOptionsValue::Object(mut value) => {
            for (_, child) in value.iter_mut() {
                *child = normalize_json_value(std::mem::take(child));
            }
            CompilerOptionsValue::Object(value)
        }
        CompilerOptionsValue::Array(value) => CompilerOptionsValue::Array(value.into_iter().map(normalize_json_value).collect()),
        CompilerOptionsValue::StringArray(value) => {
            CompilerOptionsValue::Array(value.into_iter().map(CompilerOptionsValue::String).collect())
        }
        CompilerOptionsValue::NilArray => CompilerOptionsValue::Null,
        value => value,
    }
}

// convertToObject converts the json syntax tree into the json value
fn convert_to_object(source_file: P<SourceFile>) -> (CompilerOptionsValue, Vec<P<Diagnostic>>) {
    let statements = source_file.as_node().statements();
    let root_expression = if !statements.is_empty() { statements[0].expression() } else { None };
    convert_to_json(source_file, root_expression, true /*returnValue*/, None /*jsonConversionNotifier*/)
}

fn get_default_compiler_options(config_file_name: &str) -> CompilerOptions {
    let mut options = CompilerOptions::default();
    if !config_file_name.is_empty() && tspath::get_base_file_name(config_file_name) == "jsconfig.json" {
        let depth = 2;
        options = CompilerOptions {
            allow_js: Tristate::True,
            max_node_module_js_depth: Some(depth),
            skip_lib_check: Tristate::True,
            no_emit: Tristate::True,
            ..Default::default()
        };
    }
    options
}

fn get_default_type_acquisition(config_file_name: &str) -> TypeAcquisition {
    let mut options = TypeAcquisition::default();
    if !config_file_name.is_empty() && tspath::get_base_file_name(config_file_name) == "jsconfig.json" {
        options.enable = Tristate::True;
    }
    options
}

fn convert_compiler_options_from_json_worker(
    json_options: &CompilerOptionsValue,
    base_path: &str,
    config_file_name: &str,
) -> (CompilerOptions, Vec<P<Diagnostic>>) {
    let options = get_default_compiler_options(config_file_name);
    let (parser, errors) =
        convert_options_from_json(&COMMAND_LINE_COMPILER_OPTIONS_MAP, json_options, base_path, CompilerOptionsParser(options));
    let mut options = parser.0;
    if !config_file_name.is_empty() {
        options.config_file_path = tspath::normalize_slashes(config_file_name);
    }
    (options, errors)
}

fn convert_type_acquisition_from_json_worker(
    json_options: &CompilerOptionsValue,
    base_path: &str,
    config_file_name: &str,
) -> (TypeAcquisition, Vec<P<Diagnostic>>) {
    let options = get_default_type_acquisition(config_file_name);
    let (parser, errors) = convert_options_from_json(
        TYPE_ACQUISITION_DECLARATION.element_options.as_ref().unwrap(),
        json_options,
        base_path,
        TypeAcquisitionParser(options),
    );
    (parser.0, errors)
}

fn parse_own_config_of_json(
    mut json: OrderedMap<String, CompilerOptionsValue>,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    config_file_name: &str,
) -> (ParsedTsconfig, Vec<P<Diagnostic>>) {
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    if json.has(&"excludes".to_string()) {
        errors.push(new_compiler_diagnostic(&diagnostics::Unknown_option_excludes_Did_you_mean_exclude, &[]));
    }
    let null = CompilerOptionsValue::Null;
    let (options, err) =
        convert_compiler_options_from_json_worker(json.get("compilerOptions").unwrap_or(&null), base_path, config_file_name);
    let (type_acquisition, err2) =
        convert_type_acquisition_from_json_worker(json.get("typeAcquisition").unwrap_or(&null), base_path, config_file_name);
    errors.extend(err);
    errors.extend(err2);
    if let Some(compile_on_save) = json.get("compileOnSave") {
        let (converted, compile_on_save_errors) =
            convert_json_option(&COMPILE_ON_SAVE_COMMAND_LINE_OPTION, compile_on_save.clone(), base_path, None, None, None);
        errors.extend(compile_on_save_errors);
        json.set("compileOnSave".to_string(), converted);
    }
    // Go stores a (possibly nil) []string in an `any` field, which is never a nil interface.
    let mut extended_config_path: Vec<String> = Vec::new();
    if let Some(extends) = json.get("extends") {
        if !extends.is_null() && *extends != CompilerOptionsValue::String(String::new()) {
            let (path, err) = get_extends_config_path_or_array(extends, host, base_path, config_file_name, None, None, None);
            extended_config_path = path;
            errors.extend(err);
        }
    }
    let parsed_config = ParsedTsconfig {
        raw: CompilerOptionsValue::Object(json),
        options: Some(options),
        type_acquisition: Some(type_acquisition),
        extended_config_path: Some(extended_config_path),
    };
    (parsed_config, errors)
}

fn read_json_config_file(
    file_name: &str,
    path: Path,
    read_file: impl FnOnce(&str) -> Option<String>,
) -> (P<TsConfigSourceFile>, Vec<P<Diagnostic>>) {
    let (text, diagnostic) = try_read_file(file_name, read_file, Vec::new());
    if !text.is_empty() {
        (
            TsConfigSourceFile::new(tsrs_parser::parse_source_file(
                SourceFileParseOptions { file_name: file_name.to_string(), path, ..Default::default() },
                &text,
                ScriptKind::JSON,
            )),
            diagnostic,
        )
    } else {
        let mut factory = NodeFactory::default();
        let statements = factory.new_node_list(Vec::new());
        let end_of_file = factory.new_token(Kind::EndOfFile);
        let file = TsConfigSourceFile::new(
            factory
                .new_source_file(SourceFileParseOptions { file_name: file_name.to_string(), path, ..Default::default() }, "", statements, end_of_file)
                .as_source_file_p(),
        );
        file.source_file.set_diagnostics(&diagnostic);
        (file, diagnostic)
    }
}

fn get_extended_config(
    source_file: Option<P<TsConfigSourceFile>>,
    extended_config_file_name: &str,
    host: &'static dyn ParseConfigHost,
    resolution_stack: &[Path],
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
    result: &mut ExtendsResult,
) -> (Option<&'static ParsedTsconfig>, Vec<P<Diagnostic>>) {
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    let extended_config_path =
        tspath::to_path(extended_config_file_name, host.get_current_directory(), host.fs().use_case_sensitive_file_names());

    // Bypass the cache when we detect a cycle in the resolution stack.
    // The cache locks entries during parsing, and a cycle would cause the same goroutine
    // to re-lock the same entry, resulting in a deadlock. Let parseConfig handle the
    // circularity error via its own resolution stack check.
    let cache_entry = match extended_config_cache {
        Some(cache) if !resolution_stack.contains(&extended_config_path) => {
            cache.get_extended_config(extended_config_file_name, &extended_config_path, resolution_stack, host)
        }
        _ => parse_extended_config(extended_config_file_name, extended_config_path, resolution_stack, host, extended_config_cache),
    };

    if !cache_entry.errors.is_empty() {
        errors.extend(cache_entry.errors.iter().copied());
    }

    if let Some(extended_result) = cache_entry.extended_result {
        if source_file.is_some() {
            result.extended_source_files.insert(extended_result.source_file.file_name().to_string());
            for extended_source_file in extended_result.extended_source_files.borrow().iter() {
                result.extended_source_files.insert(extended_source_file.clone());
            }
        }
    }
    (cache_entry.get().extended_config.as_ref(), errors)
}

pub fn parse_extended_config(
    file_name: &str,
    path: Path,
    resolution_stack: &[Path],
    host: &'static dyn ParseConfigHost,
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> P<ExtendedConfigCacheEntry> {
    let (extended_result, read_errors) = read_json_config_file(file_name, path, |f| host.fs().read_file(f));
    let mut entry = ExtendedConfigCacheEntry { extended_result: Some(extended_result), extended_config: None, errors: Vec::new() };

    if !read_errors.is_empty() {
        entry.errors = read_errors;
        return P::new(entry);
    }

    let parse_diagnostics = extended_result.source_file.diagnostics();
    if !parse_diagnostics.is_empty() {
        entry.errors = parse_diagnostics.to_vec();
        return P::new(entry);
    }

    let (extended_config, parse_errors) = parse_config(
        None,
        Some(extended_result),
        host,
        &tspath::get_directory_path(file_name),
        &tspath::get_base_file_name(file_name),
        resolution_stack,
        extended_config_cache,
    );
    entry.extended_config = Some(extended_config);
    entry.errors = parse_errors;
    P::new(entry)
}

// parseConfig just extracts options/include/exclude/files out of a config file.
// It does not resolve the included files.
fn parse_config(
    json: Option<OrderedMap<String, CompilerOptionsValue>>,
    source_file: Option<P<TsConfigSourceFile>>,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    config_file_name: &str,
    resolution_stack: &[Path],
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> (ParsedTsconfig, Vec<P<Diagnostic>>) {
    let base_path = tspath::normalize_slashes(base_path);
    let resolved_path = tspath::to_path(config_file_name, &base_path, host.fs().use_case_sensitive_file_names());
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    if resolution_stack.contains(&resolved_path) {
        let result;
        errors.push(new_compiler_diagnostic(&diagnostics::Circularity_detected_while_resolving_configuration_Colon_0, &[]));
        if json.as_ref().map_or(0, |j| j.size()) == 0 {
            result = ParsedTsconfig {
                raw: match json {
                    Some(json) => CompilerOptionsValue::Object(json),
                    None => CompilerOptionsValue::Null,
                },
                options: None,
                type_acquisition: None,
                extended_config_path: None,
            };
        } else {
            let (raw_result, err) = convert_to_object(source_file.unwrap().source_file);
            errors.extend(err);
            result = ParsedTsconfig { raw: raw_result, options: None, type_acquisition: None, extended_config_path: None };
        }
        return (result, errors);
    }

    let (mut own_config, err) = match json {
        Some(json) => parse_own_config_of_json(json, host, &base_path, config_file_name),
        None => parse_own_config_of_json_source_file(tsconfig_to_source_file(source_file).unwrap(), host, &base_path, config_file_name),
    };
    errors.extend(err);
    if let Some(options) = &mut own_config.options {
        if options.paths.is_some() {
            // If we end up needing to resolve relative paths from 'paths' relative to
            // the config file location, we'll need to know where that config file was.
            // Since 'paths' can be inherited from an extended config in another directory,
            // we wouldn't know which directory to use unless we store it here.
            options.paths_base_path = base_path.clone();
        }
    }

    if let Some(extended_config_paths) = own_config.extended_config_path.clone() {
        // copy the resolution stack so it is never reused between branches in potential diamond-problem scenarios.
        let mut resolution_stack = resolution_stack.to_vec();
        resolution_stack.push(resolved_path);
        let mut result = ExtendsResult {
            options: CompilerOptions::default(),
            include: None,
            exclude: None,
            files: None,
            content_mappers: None,
            compile_on_save: false,
            extended_source_files: FxHashSet::default(),
        };
        for extended_config_path in &extended_config_paths {
            apply_extended_config(
                &mut result,
                extended_config_path,
                &own_config,
                source_file,
                host,
                &base_path,
                &resolution_stack,
                extended_config_cache,
                &mut errors,
            );
        }
        let raw = own_config.raw.as_object_mut().unwrap();
        if let Some(include) = result.include {
            raw.set("include".to_string(), CompilerOptionsValue::Array(include));
        }
        if let Some(exclude) = result.exclude {
            raw.set("exclude".to_string(), CompilerOptionsValue::Array(exclude));
        }
        if let Some(files) = result.files {
            raw.set("files".to_string(), CompilerOptionsValue::Array(files));
        }
        if let Some(content_mappers) = result.content_mappers {
            if !raw.has(&"contentMappers".to_string()) {
                raw.set("contentMappers".to_string(), CompilerOptionsValue::Array(content_mappers));
            }
        }
        if result.compile_on_save && !raw.has(&"compileOnSave".to_string()) {
            raw.set("compileOnSave".to_string(), CompilerOptionsValue::Bool(result.compile_on_save));
        }
        if let Some(source_file) = source_file {
            let mut extended_source_files = source_file.extended_source_files.borrow_mut();
            #[expect(clippy::iter_over_hash_type, reason = "each file is inserted at its sorted position")]
            for extended_source_file in result.extended_source_files {
                let i = extended_source_files.binary_search(&extended_source_file).unwrap_or_else(|i| i);
                extended_source_files.insert(i, extended_source_file);
            }
        }
        merge_compiler_options(&mut result.options, own_config.options.as_ref(), Some(&own_config.raw));
        own_config.options = Some(result.options);
    }
    (own_config, errors)
}

fn apply_extended_config(
    result: &mut ExtendsResult,
    extended_config_path: &str,
    own_config: &ParsedTsconfig,
    source_file: Option<P<TsConfigSourceFile>>,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    resolution_stack: &[Path],
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
    errors: &mut Vec<P<Diagnostic>>,
) {
    let (extended_config, extended_errors) =
        get_extended_config(source_file, extended_config_path, host, resolution_stack, extended_config_cache, result);
    errors.extend(extended_errors);
    let Some(extended_config) = extended_config else {
        return;
    };
    if extended_config.options.is_none() {
        return;
    }
    let extends_raw = &extended_config.raw;
    let mut relative_difference = String::new();
    let mut set_property_value = |property_name: &str, result: &mut ExtendsResult| {
        if let CompilerOptionsValue::Object(raw_map) = &own_config.raw {
            if raw_map.contains_key(property_name) {
                return;
            }
        }
        if property_name == "include" || property_name == "exclude" || property_name == "files" {
            if let CompilerOptionsValue::Object(raw_map) = extends_raw {
                if let Some(CompilerOptionsValue::Array(slice)) = raw_map.get(property_name) {
                    let value: Vec<CompilerOptionsValue> = slice
                        .iter()
                        .map(|path| {
                            let CompilerOptionsValue::String(path_str) = path else {
                                return path.clone();
                            };
                            if starts_with_config_dir_template(path_str) || tspath::is_rooted_disk_path(path_str) {
                                CompilerOptionsValue::String(path_str.clone())
                            } else {
                                if relative_difference.is_empty() {
                                    let t = ComparePathsOptions {
                                        use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(),
                                        current_directory: base_path.to_string(),
                                    };
                                    relative_difference =
                                        tspath::convert_to_relative_path(&tspath::get_directory_path(extended_config_path), &t);
                                }
                                CompilerOptionsValue::String(tspath::combine_paths(&relative_difference, &[path_str]))
                            }
                        })
                        .collect();
                    if property_name == "include" {
                        result.include = Some(value);
                    } else if property_name == "exclude" {
                        result.exclude = Some(value);
                    } else if property_name == "files" {
                        result.files = Some(value);
                    }
                }
            }
        }
    };

    set_property_value("include", result);
    set_property_value("exclude", result);
    set_property_value("files", result);
    if let CompilerOptionsValue::Object(extended_raw_map) = extends_raw {
        if extended_raw_map.contains_key("contentMappers") {
            result.content_mappers = match extended_raw_map.get("contentMappers") {
                Some(CompilerOptionsValue::Array(a)) => Some(a.clone()),
                Some(CompilerOptionsValue::NilArray) => None,
                _ => None,
            };
        }
    }
    if let CompilerOptionsValue::Object(extended_raw_map) = extends_raw {
        if extended_raw_map.contains_key("compileOnSave") {
            if let Some(CompilerOptionsValue::Bool(compile_on_save)) = extended_raw_map.get("compileOnSave") {
                result.compile_on_save = *compile_on_save;
            }
        }
    }
    merge_compiler_options(&mut result.options, extended_config.options.as_ref(), Some(extends_raw));
}

const DEFAULT_INCLUDE_SPEC: &str = "**/*";

struct PropOfRaw {
    slice_value: Option<Vec<CompilerOptionsValue>>,
    wrong_value: &'static str,
}

fn is_string_value(value: &CompilerOptionsValue) -> bool {
    value.is_string()
}

// parseJsonConfigFileContentWorker parses the contents of a config file from json or json source file (tsconfig.json).
// json: The contents of the config file to parse
// sourceFile: sourceFile corresponding to the Json
// host: Instance of ParseConfigHost used to enumerate files in folder.
// basePath: A root directory to resolve relative path entries in the config file to. e.g. outDir
// resolutionStack: Only present for backwards-compatibility. Should be empty.
fn parse_json_config_file_content_worker(
    json: Option<OrderedMap<String, CompilerOptionsValue>>,
    source_file: Option<P<TsConfigSourceFile>>,
    host: &'static dyn ParseConfigHost,
    base_path: &str,
    existing_options: Option<&CompilerOptions>,
    existing_options_raw: Option<&CompilerOptionsValue>,
    config_file_name: &str,
    resolution_stack: &[Path],
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> ParsedCommandLine {
    assert!((json.is_none() && source_file.is_some()) || (json.is_some() && source_file.is_none()));

    let base_path_for_file_names = if !config_file_name.is_empty() {
        tspath::normalize_path(&directory_of_combined_path(config_file_name, base_path))
    } else {
        tspath::normalize_path(base_path)
    };

    let (mut parsed_config, mut errors) =
        parse_config(json, source_file, host, base_path, config_file_name, resolution_stack, extended_config_cache);
    if let Some(options) = &mut parsed_config.options {
        merge_compiler_options(options, existing_options, existing_options_raw);
    }
    handle_option_config_dir_template_substitution(parsed_config.options.as_mut(), &base_path_for_file_names);
    let raw_config = parse_json_to_string_key(&parsed_config.raw);
    if !config_file_name.is_empty() {
        if let Some(options) = &mut parsed_config.options {
            options.config_file_path = tspath::normalize_slashes(config_file_name);
        }
    }
    let get_prop_from_raw = |prop: &str,
                             validate_element: &dyn Fn(&CompilerOptionsValue) -> bool,
                             element_type_name: &str,
                             errors: &mut Vec<P<Diagnostic>>|
     -> PropOfRaw {
        if let Some(value) = raw_config.get(prop) {
            if !value.is_null() {
                if value.is_slice() {
                    let result = match value {
                        CompilerOptionsValue::Array(a) => Some(a.clone()),
                        CompilerOptionsValue::StringArray(a) => {
                            Some(a.iter().map(|s| CompilerOptionsValue::String(s.clone())).collect())
                        }
                        _ => None,
                    };
                    if let CompilerOptionsValue::Array(a) = value {
                        if source_file.is_none() && !a.iter().all(|e| validate_element(e)) {
                            errors.push(new_compiler_diagnostic(
                                &diagnostics::Compiler_option_0_requires_a_value_of_type_1,
                                &[&prop, &element_type_name],
                            ));
                        }
                    }
                    return PropOfRaw { slice_value: result, wrong_value: "" };
                } else if source_file.is_none() {
                    errors.push(new_compiler_diagnostic(
                        &diagnostics::Compiler_option_0_requires_a_value_of_type_1,
                        &[&prop, &"Array"],
                    ));
                    return PropOfRaw { slice_value: None, wrong_value: "not-array" };
                }
            }
        }
        PropOfRaw { slice_value: None, wrong_value: "no-prop" }
    };
    let is_object = |element: &CompilerOptionsValue| matches!(element, CompilerOptionsValue::Object(_));
    let references_of_raw = get_prop_from_raw("references", &is_object, "object", &mut errors);
    let file_specs = get_prop_from_raw("files", &is_string_value, "string", &mut errors);
    if file_specs.slice_value.is_some() || file_specs.wrong_value.is_empty() {
        let mut has_zero_or_no_references = false;
        if references_of_raw.wrong_value == "no-prop"
            || references_of_raw.wrong_value == "not-array"
            || references_of_raw.slice_value.as_ref().map_or(0, |v| v.len()) == 0
        {
            has_zero_or_no_references = true;
        }
        let has_extends = raw_config.get("extends").filter(|v| !v.is_null());
        if file_specs.slice_value.as_ref().is_some_and(|v| v.is_empty()) && has_zero_or_no_references && has_extends.is_none() {
            if let Some(source_file) = source_file {
                let file_name = if !config_file_name.is_empty() { config_file_name } else { "tsconfig.json" };
                let diagnostic_message = &diagnostics::The_files_list_in_config_file_0_is_empty;
                let node_value = for_each_ts_config_prop_array(Some(source_file.source_file), "files", |property| property.initializer());
                errors.push(create_diagnostic_for_node_in_source_file(
                    source_file.source_file,
                    node_value.unwrap(),
                    diagnostic_message,
                    &[&file_name],
                ));
            } else {
                errors.push(new_compiler_diagnostic(&diagnostics::The_files_list_in_config_file_0_is_empty, &[&config_file_name]));
            }
        }
    }
    let mut include_specs = get_prop_from_raw("include", &is_string_value, "string", &mut errors);
    let mut exclude_specs = get_prop_from_raw("exclude", &is_string_value, "string", &mut errors);
    let mut is_default_include_spec = false;
    if exclude_specs.wrong_value == "no-prop" {
        if let Some(options) = &parsed_config.options {
            let out_dir = &options.out_dir;
            let declaration_dir = &options.declaration_dir;
            if !out_dir.is_empty() || !declaration_dir.is_empty() {
                let mut values: Vec<CompilerOptionsValue> = Vec::new();
                if !out_dir.is_empty() {
                    values.push(CompilerOptionsValue::String(out_dir.clone()));
                }
                if !declaration_dir.is_empty() {
                    values.push(CompilerOptionsValue::String(declaration_dir.clone()));
                }
                exclude_specs = PropOfRaw { slice_value: Some(values), wrong_value: "" };
            }
        }
    }
    if file_specs.slice_value.is_none() && include_specs.slice_value.is_none() {
        include_specs = PropOfRaw { slice_value: Some(vec![CompilerOptionsValue::String(DEFAULT_INCLUDE_SPEC.to_string())]), wrong_value: "" };
        is_default_include_spec = true;
    }
    let mut validated_include_specs: Vec<String> = Vec::new();
    let mut validated_include_specs_before_substitution: Vec<String> = Vec::new();
    let mut validated_exclude_specs: Vec<String> = Vec::new();
    let mut validated_files_spec: Vec<String> = Vec::new();
    let mut validated_files_spec_before_substitution: Vec<String> = Vec::new();
    // The exclude spec list is converted into a regular expression, which allows us to quickly
    // test whether a file or directory should be excluded before recursively traversing the
    // file system.
    if let Some(include_specs) = &include_specs.slice_value {
        let err;
        (validated_include_specs_before_substitution, err) =
            validate_specs(include_specs, true /*disallowTrailingRecursion*/, tsconfig_to_source_file(source_file), "include");
        errors.extend(err);
        validated_include_specs = match get_substituted_string_array_with_config_dir_template(
            &validated_include_specs_before_substitution,
            &base_path_for_file_names,
        ) {
            Some(v) => v,
            None => validated_include_specs_before_substitution.clone(),
        };
    }
    if let Some(exclude_specs) = &exclude_specs.slice_value {
        let err;
        (validated_exclude_specs, err) =
            validate_specs(exclude_specs, false /*disallowTrailingRecursion*/, tsconfig_to_source_file(source_file), "exclude");
        errors.extend(err);
        if let Some(validated_exclude_specs_with_substitution) =
            get_substituted_string_array_with_config_dir_template(&validated_exclude_specs, &base_path_for_file_names)
        {
            validated_exclude_specs = validated_exclude_specs_with_substitution;
        }
    }
    if let Some(file_specs) = &file_specs.slice_value {
        for spec in file_specs.iter().filter(|v| is_string_value(v)) {
            if let CompilerOptionsValue::String(spec) = spec {
                validated_files_spec_before_substitution.push(spec.clone());
            }
        }
        validated_files_spec = match get_substituted_string_array_with_config_dir_template(
            &validated_files_spec_before_substitution,
            &base_path_for_file_names,
        ) {
            Some(v) => v,
            None => validated_files_spec_before_substitution.clone(),
        };
    }
    let config_file_specs = ConfigFileSpecs {
        files_specs: file_specs.slice_value,
        include_specs: include_specs.slice_value,
        exclude_specs: exclude_specs.slice_value,
        validated_files_spec,
        validated_include_specs,
        validated_exclude_specs,
        validated_files_spec_before_substitution,
        validated_include_specs_before_substitution,
        is_default_include_spec,
    };

    if let Some(source_file) = source_file {
        *source_file.config_file_specs.borrow_mut() = Some(config_file_specs.clone());
    }

    let content_mapper_source_file = source_file.map(|f| f.source_file);
    let mut content_mappers: Vec<Mapper> = Vec::new();
    let mut content_mapper_indices: Vec<i32> = Vec::new();
    let content_mappers_of_raw = get_prop_from_raw("contentMappers", &is_object, "object", &mut errors);
    for (i, element) in content_mappers_of_raw.slice_value.iter().flatten().enumerate() {
        let (mapper, mapper_errors) = parse_content_mapper(element);
        for mapper_error in mapper_errors {
            errors.push(set_content_mapper_diagnostic_location(
                mapper_error,
                content_mapper_source_file,
                get_content_mapper_syntax(content_mapper_source_file, i as i32, ""),
            ));
        }
        if let Some(mapper) = mapper {
            content_mappers.push(mapper);
            content_mapper_indices.push(i as i32);
        }
    }
    let total_content_mapper_extensions: usize = content_mappers.iter().map(|m| m.definition.extensions.len()).sum();
    let mut seen_content_mapper_extensions: FxHashSet<String> =
        FxHashSet::with_capacity_and_hasher(total_content_mapper_extensions, Default::default());
    let mut content_mapper_extensions: Vec<String> = Vec::with_capacity(total_content_mapper_extensions);
    let native_extensions: Vec<&str> = tspath::ALL_SUPPORTED_EXTENSIONS_WITH_JSON.iter().flat_map(|g| g.iter().copied()).collect();
    let canonical_extension = |extension: &str| tspath::get_canonical_file_name(extension, host.fs().use_case_sensitive_file_names());
    for (j, mapper) in content_mappers.iter_mut().enumerate() {
        let mut valid_extensions: Vec<String> = Vec::with_capacity(mapper.definition.extensions.len());
        for ext in mapper.definition.extensions.iter() {
            let ext_node = get_content_mapper_extension_syntax(content_mapper_source_file, content_mapper_indices[j], ext);
            let canonical_ext = canonical_extension(ext);
            if !ext.starts_with('.') {
                errors.push(set_content_mapper_diagnostic_location(
                    new_compiler_diagnostic(&diagnostics::Content_mapper_file_extension_0_must_begin_with_a, &[ext]),
                    content_mapper_source_file,
                    ext_node,
                ));
            } else if native_extensions.iter().any(|native_extension| tsrs_core::stringutil::equal_fold(native_extension, ext)) {
                errors.push(set_content_mapper_diagnostic_location(
                    new_compiler_diagnostic(
                        &diagnostics::Content_mapper_file_extension_0_is_a_built_in_extension_and_cannot_be_registered_by_a_content_mapper,
                        &[ext],
                    ),
                    content_mapper_source_file,
                    ext_node,
                ));
            } else if seen_content_mapper_extensions.contains(canonical_ext.as_str()) {
                errors.push(set_content_mapper_diagnostic_location(
                    new_compiler_diagnostic(
                        &diagnostics::Content_mapper_file_extension_0_is_registered_by_more_than_one_content_mapper,
                        &[ext],
                    ),
                    content_mapper_source_file,
                    ext_node,
                ));
            } else {
                seen_content_mapper_extensions.insert(canonical_ext.clone());
                content_mapper_extensions.push(ext.clone());
                valid_extensions.push(ext.clone());
            }
        }
        mapper.definition.extensions = valid_extensions;
    }
    if !content_mappers.is_empty() && !parsed_config.options.as_ref().is_some_and(|o| o.run_external_code.is_true()) {
        errors.push(set_content_mapper_diagnostic_location(
            new_compiler_diagnostic(&diagnostics::Content_mappers_require_the_runExternalCode_command_line_flag_to_be_enabled, &[]),
            content_mapper_source_file,
            get_content_mappers_key_syntax(content_mapper_source_file),
        ));
        // Without the flag the mappers are not trusted to run, so drop them entirely: their extensions are
        // not registered and their files are not intercepted (they are treated as unknown foreign files).
        content_mappers = Vec::new();
        content_mapper_extensions = Vec::new();
    } else if !content_mappers.is_empty() {
        // Resolve each mapper's package.json now so its name, version, and run command are available to
        // everything downstream (diagnostics, build-info staleness) without executing anything.
        let containing_file = if config_file_name.is_empty() {
            tspath::combine_paths(&base_path_for_file_names, &["tsconfig.json"])
        } else {
            config_file_name.to_string()
        };
        let mut resolved_content_mappers: Vec<Mapper> = Vec::with_capacity(content_mappers.len());
        for (j, mut mapper) in content_mappers.into_iter().enumerate() {
            let (manifest, package_directory, diagnostic) = resolve_content_mapper_manifest(host, &containing_file, &mapper.definition.package);
            mapper.package_directory = package_directory;
            if let Some(diagnostic) = diagnostic {
                errors.push(set_content_mapper_diagnostic_location(
                    diagnostic,
                    content_mapper_source_file,
                    get_content_mapper_syntax(content_mapper_source_file, content_mapper_indices[j], "package"),
                ));
                continue;
            }
            mapper.manifest = manifest;
            resolved_content_mappers.push(mapper);
        }
        content_mappers = resolved_content_mappers;
        content_mapper_extensions = content_mappers.iter().flat_map(|m| m.definition.extensions.iter().cloned()).collect();
    }

    let mut get_file_names = |base_path: &str, errors: &mut Vec<P<Diagnostic>>| -> (Vec<String>, usize) {
        let parsed_config_options = parsed_config.options.as_ref();
        let (file_names, literal_file_names_len) =
            get_file_names_from_config_specs(&config_file_specs, base_path, parsed_config_options, host.fs(), &content_mapper_extensions);
        if should_report_no_input_files(&file_names, can_json_report_no_input_files(&raw_config), resolution_stack) {
            let include_specs = CompilerOptionsValue::Array(config_file_specs.include_specs.clone().unwrap_or_default());
            let exclude_specs = CompilerOptionsValue::Array(config_file_specs.exclude_specs.clone().unwrap_or_default());
            errors.push(new_compiler_diagnostic(
                &diagnostics::No_inputs_were_found_in_config_file_0_Specified_include_paths_were_1_and_exclude_paths_were_2,
                &[&config_file_name, &stringify_json(&include_specs), &stringify_json(&exclude_specs)],
            ));
        }
        (file_names, literal_file_names_len)
    };

    let get_project_references = |base_path: &str, errors: &mut Vec<P<Diagnostic>>| -> Option<Vec<ProjectReference>> {
        let mut project_references: Option<Vec<ProjectReference>> = None;
        let new_references_of_raw = get_prop_from_raw("references", &is_object, "object", errors);
        if let Some(slice_value) = &new_references_of_raw.slice_value {
            let project_references = project_references.insert(Vec::new());
            for (index, reference) in slice_value.iter().enumerate() {
                let Some(ref_) = parse_project_reference(reference) else {
                    continue;
                };
                if !ref_.has_path || !ref_.path_valid {
                    errors.push(create_diagnostic_at_project_reference_property(
                        source_file,
                        index,
                        "path",
                        &diagnostics::Compiler_option_0_requires_a_value_of_type_1,
                        &[&"reference.path", &"string"],
                    ));
                    continue;
                }
                if ref_.reference.path.is_empty() {
                    errors.push(create_diagnostic_at_project_reference_property(
                        source_file,
                        index,
                        "path",
                        &diagnostics::Compiler_option_0_cannot_be_given_an_empty_string,
                        &[&"reference.path"],
                    ));
                    continue;
                }
                if ref_.has_circular && !ref_.circular_valid {
                    errors.push(create_diagnostic_at_project_reference_property(
                        source_file,
                        index,
                        "circular",
                        &diagnostics::Compiler_option_0_requires_a_value_of_type_1,
                        &[&"reference.circular", &"boolean"],
                    ));
                }
                project_references.push(ProjectReference {
                    path: tspath::get_normalized_absolute_path(&ref_.reference.path, base_path),
                    original_path: ref_.reference.path.clone(),
                    circular: ref_.reference.circular,
                });
            }
        }
        project_references
    };

    let (file_names, literal_file_names_len) = get_file_names(&base_path_for_file_names, &mut errors);
    let mut compile_on_save = false;
    if let CompilerOptionsValue::Object(raw) = &parsed_config.raw {
        if let Some(CompilerOptionsValue::Bool(value)) = raw.get("compileOnSave") {
            compile_on_save = *value;
        }
    }
    let project_references = get_project_references(&base_path_for_file_names, &mut errors);
    ParsedCommandLine {
        parsed_config: ParsedOptions {
            compiler_options: parsed_config.options.map(P::new),
            type_acquisition: parsed_config.type_acquisition,
            file_names,
            project_references_is_nil: project_references.is_none(),
            project_references: project_references.unwrap_or_default(),
            content_mappers,
            watch_options: None,
        },
        config_file: source_file,
        raw: parsed_config.raw,
        errors,
        compile_on_save: Some(compile_on_save),

        compare_paths_options: ComparePathsOptions {
            use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(),
            current_directory: base_path_for_file_names,
        },
        literal_file_names_len,
        ..ParsedCommandLine::empty()
    }
}

fn can_json_report_no_input_files(raw_config: &OrderedMap<String, CompilerOptionsValue>) -> bool {
    let files_exists = raw_config.contains_key("files");
    let references_exists = raw_config.contains_key("references");
    !files_exists && !references_exists
}

fn should_report_no_input_files(file_names: &[String], can_json_report_no_input_files: bool, resolution_stack: &[Path]) -> bool {
    file_names.is_empty() && can_json_report_no_input_files && resolution_stack.is_empty()
}

fn validate_specs(
    specs: &[CompilerOptionsValue],
    disallow_trailing_recursion: bool,
    json_source_file: Option<P<SourceFile>>,
    spec_key: &str,
) -> (Vec<String>, Vec<P<Diagnostic>>) {
    let create_diagnostic = |message: &'static Message, spec: &str| -> P<Diagnostic> {
        let element = get_ts_config_prop_array_element_value(json_source_file, spec_key, spec);
        create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(json_source_file, element, message, &[&spec])
    };
    let mut errors: Vec<P<Diagnostic>> = Vec::new();
    let mut final_specs: Vec<String> = Vec::new();
    for value in specs {
        let CompilerOptionsValue::String(spec) = value else {
            continue;
        };
        let diag = spec_to_diagnostic(spec, disallow_trailing_recursion);
        if let Some(diag) = diag {
            errors.push(create_diagnostic(diag, spec));
        } else {
            final_specs.push(spec.clone());
        }
    }
    (final_specs, errors)
}

pub(crate) fn spec_to_diagnostic(spec: &str, disallow_trailing_recursion: bool) -> Option<&'static Message> {
    if disallow_trailing_recursion && invalid_trailing_recursion(spec) {
        return Some(&diagnostics::File_specification_cannot_end_in_a_recursive_directory_wildcard_Asterisk_Asterisk_Colon_0);
    }
    if invalid_dot_dot_after_recursive_wildcard(spec) {
        return Some(
            &diagnostics::File_specification_cannot_contain_a_parent_directory_that_appears_after_a_recursive_directory_wildcard_Asterisk_Asterisk_Colon_0,
        );
    }
    None
}

fn invalid_trailing_recursion(spec: &str) -> bool {
    // Matches **, /**, **/, and /**/, but not a**b.
    // Strip optional trailing slash, then check if it ends with /** or is just **
    let s = spec.strip_suffix('/').unwrap_or(spec);
    s == "**" || s.ends_with("/**")
}

fn invalid_dot_dot_after_recursive_wildcard(s: &str) -> bool {
    // We used to use the regex /(^|\/)\*\*\/(.*\/)?\.\.($|\/)/ to check for this case, but
    // in v8, that has polynomial performance because the recursive wildcard match - **/ -
    // can be matched in many arbitrary positions when multiple are present, resulting
    // in bad backtracking (and we don't care which is matched - just that some /.. segment
    // comes after some **/ segment).
    let wildcard_index: i64 = if s.starts_with("**/") {
        0
    } else {
        s.find("/**/").map_or(-1, |i| i as i64)
    };
    if wildcard_index == -1 {
        return false;
    }
    let last_dot_index: i64 = if s.ends_with("/..") { s.len() as i64 } else { s.rfind("/../").map_or(-1, |i| i as i64) };
    last_dot_index > wildcard_index
}

pub fn get_ts_config_prop_array_element_value(
    ts_config_source_file: Option<P<SourceFile>>,
    prop_key: &str,
    element_value: &str,
) -> Option<P<Node>> {
    let callback = get_callback_for_finding_property_assignment_by_value(element_value);
    for_each_ts_config_prop_array(ts_config_source_file, prop_key, |property| callback(property))
}

pub fn for_each_ts_config_prop_array<T>(
    ts_config_source_file: Option<P<SourceFile>>,
    prop_key: &str,
    callback: impl FnMut(P<Node>) -> Option<T>,
) -> Option<T> {
    if let Some(ts_config_source_file) = ts_config_source_file {
        return for_each_property_assignment(get_ts_config_object_literal_expression(Some(ts_config_source_file)), prop_key, callback, None);
    }
    None
}

pub fn create_diagnostic_at_reference_syntax(
    config: &ParsedCommandLine,
    index: usize,
    message: &'static Message,
    args: &[&dyn Display],
) -> Option<P<Diagnostic>> {
    // Programs created through the API have references but no config file: the caller reports the diagnostic
    // without a location (Go dereferences nil here).
    let source_file = config.config_file?.source_file;
    for_each_ts_config_prop_array(Some(source_file), "references", |property| {
        let initializer = property.initializer().unwrap();
        if tsrs_ast::is_array_literal_expression(initializer) {
            let value = initializer.elements();
            if value.len() > index {
                return Some(create_diagnostic_for_node_in_source_file(source_file, value[index], message, args));
            }
        }
        None
    })
}

fn create_diagnostic_at_project_reference_property(
    source_file: Option<P<TsConfigSourceFile>>,
    index: usize,
    property_name: &str,
    message: &'static Message,
    args: &[&dyn Display],
) -> P<Diagnostic> {
    let mut node: Option<P<Node>> = None;
    if let Some(source_file) = source_file {
        node = for_each_ts_config_prop_array(Some(source_file.source_file), "references", |property| {
            let initializer = property.initializer().unwrap();
            if tsrs_ast::is_array_literal_expression(initializer) {
                let elements = initializer.elements();
                if elements.len() > index && tsrs_ast::is_object_literal_expression(elements[index]) {
                    if let Some(property_node) =
                        for_each_property_assignment(Some(elements[index]), property_name, |property| property.initializer(), None)
                    {
                        return Some(property_node);
                    }
                    return Some(elements[index]);
                }
            }
            None
        });
    }
    create_diagnostic_for_node_in_source_file_or_compiler_diagnostic(tsconfig_to_source_file(source_file), node, message, args)
}

pub fn get_callback_for_finding_property_assignment_by_value(value: &str) -> impl Fn(P<Node>) -> Option<P<Node>> + '_ {
    move |property: P<Node>| {
        let initializer = property.initializer().unwrap();
        if tsrs_ast::is_array_literal_expression(initializer) {
            return initializer.elements().iter().copied().find(|element| tsrs_ast::is_string_literal(*element) && element.text() == value);
        }
        None
    }
}

pub fn get_options_syntax_by_array_element_value(object_literal: Option<P<Node>>, prop_key: &str, element_value: &str) -> Option<P<Node>> {
    for_each_property_assignment(object_literal, prop_key, get_callback_for_finding_property_assignment_by_value(element_value), None)
}

// getContentMapperSyntax returns the tsconfig JSON node to attribute a diagnostic about the content
// mapper at index to: the value of subKey within that mapper's object (when subKey is non-empty),
// falling back to the mapper element, then to the "contentMappers" array. An index outside the array
// (e.g. -1) yields the array itself. Returns nil when there is no source file (JSON API).
fn get_content_mapper_syntax(source_file: Option<P<SourceFile>>, index: i32, sub_key: &str) -> Option<P<Node>> {
    source_file?;
    for_each_ts_config_prop_array(source_file, "contentMappers", |property| {
        let initializer = property.initializer().unwrap();
        if !tsrs_ast::is_array_literal_expression(initializer) {
            return Some(initializer);
        }
        let elements = initializer.elements();
        if index < 0 || index as usize >= elements.len() {
            return Some(initializer);
        }
        let element = elements[index as usize];
        if !sub_key.is_empty() && tsrs_ast::is_object_literal_expression(element) {
            if let Some(node) = for_each_property_assignment(Some(element), sub_key, |property| property.initializer(), None) {
                return Some(node);
            }
        }
        Some(element)
    })
}

// getContentMappersKeySyntax returns the "contentMappers" property key node, used to attribute a
// diagnostic about the setting as a whole rather than a specific mapper.
fn get_content_mappers_key_syntax(source_file: Option<P<SourceFile>>) -> Option<P<Node>> {
    source_file?;
    for_each_ts_config_prop_array(source_file, "contentMappers", |property| property.name())
}

// getContentMapperExtensionSyntax returns the node for a specific extension string within the content
// mapper at index, falling back to the "extensions" array or the mapper element.
fn get_content_mapper_extension_syntax(source_file: Option<P<SourceFile>>, index: i32, ext: &str) -> Option<P<Node>> {
    let node = get_content_mapper_syntax(source_file, index, "extensions");
    if let Some(node) = node {
        if tsrs_ast::is_array_literal_expression(node) {
            if let Some(element) =
                node.elements().iter().copied().find(|element| tsrs_ast::is_string_literal(*element) && element.text() == ext)
            {
                return Some(element);
            }
        }
    }
    node
}

// setContentMapperDiagnosticLocation attaches a source location to a content mapper diagnostic when a
// tsconfig source file and node are available (the jsonSourceFile API), leaving it as a location-less
// compiler diagnostic otherwise (the JSON API).
fn set_content_mapper_diagnostic_location(
    diagnostic: P<Diagnostic>,
    source_file: Option<P<SourceFile>>,
    node: Option<P<Node>>,
) -> P<Diagnostic> {
    if let (Some(source_file), Some(node)) = (source_file, node) {
        diagnostic.set_file(Some(source_file));
        diagnostic.set_location(TextRange::new(tsrs_scanner::skip_trivia(source_file.text(), node.pos()), node.end()));
    }
    diagnostic
}

pub fn for_each_property_assignment<T>(
    object_literal: Option<P<Node>>,
    key: &str,
    mut callback: impl FnMut(P<Node>) -> Option<T>,
    key2: Option<&str>,
) -> Option<T> {
    if let Some(object_literal) = object_literal {
        for property in object_literal.as_object_literal_expression().properties.nodes().iter().copied() {
            if !tsrs_ast::is_property_assignment(property) {
                continue;
            }
            if let Some(prop_name) = tsrs_ast::try_get_text_of_property_name(property.name().unwrap()) {
                if prop_name == key || key2.is_some_and(|key2| key2 == prop_name) {
                    return callback(property);
                }
            }
        }
    }
    None
}

fn get_ts_config_object_literal_expression(ts_config_source_file: Option<P<SourceFile>>) -> Option<P<Node>> {
    if let Some(ts_config_source_file) = ts_config_source_file {
        let statements = ts_config_source_file.as_node().statements();
        if !statements.is_empty() {
            let expression = statements[0].expression();
            if let Some(expression) = expression {
                if tsrs_ast::is_object_literal_expression(expression) {
                    return Some(expression);
                }
            }
        }
    }
    None
}

fn get_substituted_path_with_config_dir_template(value: &str, base_path: &str) -> String {
    tspath::get_normalized_absolute_path(&value.replacen(CONFIG_DIR_TEMPLATE, "./", 1), base_path)
}

fn get_substituted_string_array_with_config_dir_template(list: &[String], base_path: &str) -> Option<Vec<String>> {
    let mut result: Option<Vec<String>> = None;
    for (i, element) in list.iter().enumerate() {
        if starts_with_config_dir_template(element) {
            let result = result.get_or_insert_with(|| list.to_vec());
            result[i] = get_substituted_path_with_config_dir_template(element, base_path);
        }
    }
    result
}

fn handle_option_config_dir_template_substitution(compiler_options: Option<&mut CompilerOptions>, base_path: &str) {
    let Some(compiler_options) = compiler_options else {
        return;
    };

    // !!! don't hardcode this; use options declarations?

    if let Some(paths) = &mut compiler_options.paths {
        for (_, v) in paths.iter_mut() {
            if let Some(substitution) = get_substituted_string_array_with_config_dir_template(v, base_path) {
                *v = substitution;
            }
        }
    }

    if let Some(root_dirs) =
        compiler_options.root_dirs.as_deref().and_then(|v| get_substituted_string_array_with_config_dir_template(v, base_path))
    {
        compiler_options.root_dirs = Some(root_dirs);
    }
    if let Some(type_roots) =
        compiler_options.type_roots.as_deref().and_then(|v| get_substituted_string_array_with_config_dir_template(v, base_path))
    {
        compiler_options.type_roots = Some(type_roots);
    }
    macro_rules! substitute_string {
        ($($field:ident),*) => {
            $(
                if starts_with_config_dir_template(&compiler_options.$field) {
                    compiler_options.$field = get_substituted_path_with_config_dir_template(&compiler_options.$field, base_path);
                }
            )*
        };
    }
    substitute_string!(
        generate_cpu_profile,
        generate_trace,
        out_file,
        out_dir,
        root_dir,
        ts_build_info_file,
        base_url,
        declaration_dir
    );
}

// hasFileWithHigherPriorityExtension determines whether a literal or wildcard file has already been included that has a higher extension priority.
// file is the path to the file.
fn has_file_with_higher_priority_extension(file: &str, extensions: &[Vec<String>], has_file: impl Fn(&str) -> bool) -> bool {
    let mut extension_group: Vec<&str> = Vec::new();
    for group in extensions {
        let group: Vec<&str> = group.iter().map(|s| s.as_str()).collect();
        if tspath::file_extension_is_one_of(file, &group) {
            extension_group.extend(group);
        }
    }
    if extension_group.is_empty() {
        return false;
    }
    for ext in extension_group {
        // d.ts files match with .ts extension and with case sensitive sorting the file order for same files with ts tsx and dts extension is
        // d.ts, .ts, .tsx in that order so we need to handle tsx and dts of same same name case here and in remove files with same extensions
        // So dont match .d.ts files with .ts extension
        if tspath::file_extension_is(file, ext) && (ext != tspath::EXTENSION_TS || !tspath::file_extension_is(file, tspath::EXTENSION_DTS)) {
            return false;
        }
        if has_file(&tspath::change_extension(file, ext)) {
            if ext == tspath::EXTENSION_DTS
                && (tspath::file_extension_is(file, tspath::EXTENSION_JS) || tspath::file_extension_is(file, tspath::EXTENSION_JSX))
            {
                // LEGACY BEHAVIOR: An off-by-one bug somewhere in the extension priority system for wildcard module loading allowed declaration
                // files to be loaded alongside their js(x) counterparts. We regard this as generally undesirable, but retain the behavior to
                // prevent breakage.
                continue;
            }
            return true;
        }
    }
    false
}

// Removes files included via wildcard expansion with a lower extension priority that have already been included.
// file is the path to the file.
fn remove_wildcard_files_with_lower_priority_extension(
    file: &str,
    wildcard_files: &mut OrderedMap<String, String>,
    extensions: &[Vec<String>],
    key_mapper: impl Fn(&str) -> String,
) {
    let mut extension_group: Option<Vec<&str>> = None;
    for group in extensions {
        let group: Vec<&str> = group.iter().map(|s| s.as_str()).collect();
        if tspath::file_extension_is_one_of(file, &group) {
            extension_group.get_or_insert_with(Vec::new).extend(group);
        }
    }
    let Some(extension_group) = extension_group else {
        return;
    };
    for ext in extension_group.iter().rev() {
        if tspath::file_extension_is(file, ext) {
            return;
        }
        let lower_priority_path = key_mapper(&tspath::change_extension(file, ext));
        wildcard_files.delete(&lower_priority_path);
    }
}

// getFileNamesFromConfigSpecs gets the file names from the provided config file specs that contain, files, include, exclude and
// other properties needed to resolve the file names
// configFileSpecs is the config file specs extracted with file names to include, wildcards to include/exclude and other details
// basePath is the base path for any relative file specifications.
// options is the Compiler options.
// host is the host used to resolve files and directories.
// extraExtensions are additional file extensions (e.g. from content mappers) to treat as supported.
pub(crate) fn get_file_names_from_config_specs(
    config_file_specs: &ConfigFileSpecs,
    base_path: &str, // considering this is the current directory
    options: Option<&CompilerOptions>,
    host: &dyn FS,
    extra_extensions: &[String],
) -> (Vec<String>, usize) {
    let base_path = tspath::normalize_path(base_path);
    let key_mappper = |value: &str| tspath::get_canonical_file_name(value, host.use_case_sensitive_file_names());
    // Literal file names (provided via the "files" array in tsconfig.json) are stored in a
    // file map with a possibly case insensitive key. We use this map later when when including
    // wildcard paths.
    let mut literal_file_map: OrderedMap<String, String> = OrderedMap::default();
    // Wildcard paths (provided via the "includes" array in tsconfig.json) are stored in a
    // file map with a possibly case insensitive key. We use this map to store paths matched
    // via wildcard, and to handle extension priority.
    let mut wildcard_file_map: OrderedMap<String, String> = OrderedMap::default();
    // Wildcard paths of json files (provided via the "includes" array in tsconfig.json) are stored in a
    // file map with a possibly case insensitive key. We use this map to store paths matched
    // via wildcard of *.json kind
    let mut wild_card_json_file_map: OrderedMap<String, String> = OrderedMap::default();
    let validated_files_spec = &config_file_specs.validated_files_spec;
    let validated_include_specs = &config_file_specs.validated_include_specs;
    let validated_exclude_specs = &config_file_specs.validated_exclude_specs;
    // Rather than re-query this for each file and filespec, we query the supported extensions
    // once and store it on the expansion context.
    let supported_extensions = get_supported_extensions(options, extra_extensions);
    let supported_extensions_with_json_if_resolve_json_module =
        get_supported_extensions_with_json_if_resolve_json_module(options, &supported_extensions);
    // Literal files are always included verbatim. An "include" or "exclude" specification cannot
    // remove a literal file.
    for file_name in validated_files_spec {
        let file = tspath::get_normalized_absolute_path(file_name, &base_path);
        literal_file_map.set(key_mappper(file_name), file);
    }

    let mut json_only_include_matchers: Option<Option<vfsmatch::SpecMatcher>> = None;
    if !validated_include_specs.is_empty() {
        let flat_extensions: Vec<String> = supported_extensions_with_json_if_resolve_json_module.iter().flatten().cloned().collect();
        let files = tsrs_core::phases::time("Config: include glob", || {
            vfsmatch::read_directory(
                host,
                &base_path,
                &base_path,
                &flat_extensions,
                validated_exclude_specs,
                validated_include_specs,
                vfsmatch::UNLIMITED_DEPTH,
            )
        });
        for file in files {
            if tspath::file_extension_is(&file, tspath::EXTENSION_JSON) {
                let matchers = json_only_include_matchers.get_or_insert_with(|| {
                    let includes: Vec<String> =
                        validated_include_specs.iter().filter(|include| include.ends_with(tspath::EXTENSION_JSON)).cloned().collect();
                    vfsmatch::new_spec_matcher(&includes, &base_path, vfsmatch::Usage::Files, host.use_case_sensitive_file_names())
                });
                let mut include_index: i32 = -1;
                if let Some(matchers) = matchers {
                    include_index = matchers.match_index(&file);
                }
                if include_index != -1 {
                    let key = key_mappper(&file);
                    if !literal_file_map.contains_key(&key) && !wild_card_json_file_map.contains_key(&key) {
                        wild_card_json_file_map.set(key, file.clone());
                    }
                }
                continue;
            }
            // If we have already included a literal or wildcard path with a
            // higher priority extension, we should skip this file.
            //
            // This handles cases where we may encounter both <file>.ts and
            // <file>.d.ts (or <file>.js if "allowJs" is enabled) in the same
            // directory when they are compilation outputs.
            if has_file_with_higher_priority_extension(&file, &supported_extensions, |file_name| {
                let canonical_file_name = key_mappper(file_name);
                literal_file_map.contains_key(&canonical_file_name) || wildcard_file_map.contains_key(&canonical_file_name)
            }) {
                continue;
            }
            // We may have included a wildcard path with a lower priority
            // extension due to the user-defined order of entries in the
            // "include" array. If there is a lower priority extension in the
            // same directory, we should remove it.
            remove_wildcard_files_with_lower_priority_extension(&file, &mut wildcard_file_map, &supported_extensions, key_mappper);
            let key = key_mappper(&file);
            if !literal_file_map.contains_key(&key) && !wildcard_file_map.contains_key(&key) {
                wildcard_file_map.set(key, file.clone());
            }
        }
    }
    let mut files: Vec<String> = Vec::with_capacity(literal_file_map.len() + wildcard_file_map.len() + wild_card_json_file_map.len());
    for file in literal_file_map.values() {
        files.push(file.clone());
    }
    for file in wildcard_file_map.values() {
        files.push(file.clone());
    }
    for file in wild_card_json_file_map.values() {
        files.push(file.clone());
    }
    (files, literal_file_map.len())
}

fn to_owned_groups(groups: &[&[&str]]) -> Vec<Vec<String>> {
    groups.iter().map(|g| g.iter().map(|s| s.to_string()).collect()).collect()
}

pub fn get_supported_extensions(compiler_options: Option<&CompilerOptions>, extra_extensions: &[String]) -> Vec<Vec<String>> {
    let need_js_extensions = compiler_options.is_some_and(|o| o.get_allow_js());
    let builtins = if need_js_extensions {
        to_owned_groups(&tspath::ALL_SUPPORTED_EXTENSIONS)
    } else {
        to_owned_groups(&tspath::SUPPORTED_TS_EXTENSIONS)
    };
    if extra_extensions.is_empty() {
        return builtins;
    }
    let flat_builtins: Vec<&String> = builtins.iter().flatten().collect();
    let mut result: Vec<Vec<String>> = Vec::new();
    for ext in extra_extensions {
        if !flat_builtins.contains(&ext) {
            result.push(vec![ext.clone()]);
        }
    }
    if result.is_empty() {
        return builtins;
    }
    let mut builtins = builtins;
    builtins.extend(result);
    builtins
}

pub fn get_supported_extensions_with_json_if_resolve_json_module(
    compiler_options: Option<&CompilerOptions>,
    supported_extensions: &[Vec<String>],
) -> Vec<Vec<String>> {
    let Some(compiler_options) = compiler_options else {
        return supported_extensions.to_vec();
    };
    if !compiler_options.get_resolve_json_module() {
        return supported_extensions.to_vec();
    }
    if supported_extensions == to_owned_groups(&tspath::ALL_SUPPORTED_EXTENSIONS).as_slice() {
        return to_owned_groups(&tspath::ALL_SUPPORTED_EXTENSIONS_WITH_JSON);
    }
    if supported_extensions == to_owned_groups(&tspath::SUPPORTED_TS_EXTENSIONS).as_slice() {
        return to_owned_groups(&tspath::SUPPORTED_TS_EXTENSIONS_WITH_JSON);
    }
    let mut result = supported_extensions.to_vec();
    result.push(vec![tspath::EXTENSION_JSON.to_string()]);
    result
}

// Reads the config file and reports errors.
pub fn get_parsed_command_line_of_config_file(
    config_file_name: &str,
    options: Option<&CompilerOptions>,
    options_raw: Option<&CompilerOptionsValue>,
    sys: &'static dyn ParseConfigHost,
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> (Option<ParsedCommandLine>, Vec<P<Diagnostic>>) {
    let config_file_name = tspath::get_normalized_absolute_path(config_file_name, sys.get_current_directory());
    let path = tspath::to_path(&config_file_name, sys.get_current_directory(), sys.fs().use_case_sensitive_file_names());
    get_parsed_command_line_of_config_file_path(&config_file_name, path, options, options_raw, sys, extended_config_cache)
}

pub fn get_parsed_command_line_of_config_file_path(
    config_file_name: &str,
    path: Path,
    options: Option<&CompilerOptions>,
    options_raw: Option<&CompilerOptionsValue>,
    sys: &'static dyn ParseConfigHost,
    extended_config_cache: Option<&dyn ExtendedConfigCache>,
) -> (Option<ParsedCommandLine>, Vec<P<Diagnostic>>) {
    let (config_file_text, errors) = try_read_file(config_file_name, |f| sys.fs().read_file(f), Vec::new());
    if !errors.is_empty() {
        // these are unrecoverable errors--exit to report them as diagnostics
        return (None, errors);
    }

    let ts_config_source_file = new_tsconfig_source_file_from_file_path(config_file_name, path, &config_file_text);
    // tsConfigSourceFile.resolvedPath = tsConfigSourceFile.FileName()
    // tsConfigSourceFile.originalFileName = tsConfigSourceFile.FileName()
    (
        Some(parse_json_source_file_config_file_content(
            ts_config_source_file,
            sys,
            &tspath::get_directory_path(config_file_name),
            options,
            options_raw,
            config_file_name,
            &[],
            extended_config_cache,
        )),
        Vec::new(),
    )
}

// Go json.Marshal of the `any` option values (compact form).
pub fn stringify_json(value: &CompilerOptionsValue) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => { let _ = write!(out, "\\u{:04x}", c as u32); },
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_json(out: &mut String, value: &CompilerOptionsValue) {
    match value {
        CompilerOptionsValue::Null | CompilerOptionsValue::NilArray => out.push_str("null"),
        CompilerOptionsValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        CompilerOptionsValue::Float(f) => out.push_str(&tsrs_core::jsnum::Number(*f).to_string()),
        CompilerOptionsValue::Int(i) => out.push_str(&i.to_string()),
        CompilerOptionsValue::String(s) => write_json_string(out, s),
        CompilerOptionsValue::Array(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(out, v);
            }
            out.push(']');
        }
        CompilerOptionsValue::StringArray(a) => {
            out.push('[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(out, v);
            }
            out.push(']');
        }
        CompilerOptionsValue::Object(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(out, k);
                out.push(':');
                write_json(out, v);
            }
            out.push('}');
        }
        CompilerOptionsValue::EmptyStruct => out.push_str("{}"),
        CompilerOptionsValue::Tristate(t) => out.push_str(match t {
            Tristate::Unknown => "null",
            Tristate::False => "false",
            Tristate::True => "true",
        }),
        CompilerOptionsValue::ModuleKind(v) => out.push_str(&v.value().to_string()),
        CompilerOptionsValue::ModuleResolutionKind(v) => out.push_str(&v.value().to_string()),
        CompilerOptionsValue::ModuleDetectionKind(v) => out.push_str(&v.value().to_string()),
        CompilerOptionsValue::ScriptTarget(v) => out.push_str(&v.value().to_string()),
        CompilerOptionsValue::JsxEmit(v) => out.push_str(&v.value().to_string()),
        CompilerOptionsValue::NewLineKind(v) => out.push_str(&v.value().to_string()),
        CompilerOptionsValue::WatchFileKind(v) => out.push_str(&(*v as i32).to_string()),
        CompilerOptionsValue::WatchDirectoryKind(v) => out.push_str(&(*v as i32).to_string()),
        CompilerOptionsValue::PollingKind(v) => out.push_str(&(*v as i32).to_string()),
    }
}

// Go `locale.Parse` (golang.org/x/text/language.Parse) reports whether the string is a BCP 47 language
// tag. This checks BCP 47 well-formedness (language, script, region, variants, extensions, private use).
pub(crate) fn locale_parse(locale_str: &str) -> bool {
    let subtags: Vec<&str> = locale_str.split(['-', '_']).collect();
    let is_alpha = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic());
    let is_digit = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let is_alnum = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    let mut i = 0;
    let first = subtags[0];
    if first.eq_ignore_ascii_case("x") {
        return subtags.len() > 1 && subtags[1..].iter().all(|s| is_alnum(s) && s.len() <= 8);
    }
    if !(is_alpha(first) && (2..=8).contains(&first.len()) && first.len() != 4) {
        return false;
    }
    i += 1;
    // extlang
    let mut extlangs = 0;
    while i < subtags.len() && first.len() <= 3 && subtags[i].len() == 3 && is_alpha(subtags[i]) && extlangs < 3 {
        i += 1;
        extlangs += 1;
    }
    // script
    if i < subtags.len() && subtags[i].len() == 4 && is_alpha(subtags[i]) {
        i += 1;
    }
    // region
    if i < subtags.len() && ((subtags[i].len() == 2 && is_alpha(subtags[i])) || (subtags[i].len() == 3 && is_digit(subtags[i]))) {
        i += 1;
    }
    // variants
    while i < subtags.len()
        && is_alnum(subtags[i])
        && ((5..=8).contains(&subtags[i].len()) || (subtags[i].len() == 4 && subtags[i].as_bytes()[0].is_ascii_digit()))
    {
        i += 1;
    }
    // extensions and private use
    while i < subtags.len() {
        let singleton = subtags[i];
        if singleton.len() != 1 || !is_alnum(singleton) {
            return false;
        }
        let private_use = singleton.eq_ignore_ascii_case("x");
        i += 1;
        let start = i;
        while i < subtags.len() && is_alnum(subtags[i]) && subtags[i].len() <= 8 && (private_use || subtags[i].len() >= 2) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    true
}
