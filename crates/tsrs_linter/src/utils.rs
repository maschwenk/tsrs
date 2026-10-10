use serde::Deserialize;
use tsrs_ast::{self as ast, Kind, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::{Checker, Type, TypeFlags};
use tsrs_compiler::Program;
use tsrs_core::P;
use tsrs_core::tspath;

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum TypeOrValueSpecifier {
    Name(String),
    Source(SourceSpecifier),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpecifier {
    #[serde(default = "default_from")]
    pub from: String,
    #[serde(default, deserialize_with = "one_or_many")]
    pub name: Vec<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub package: String,
}

fn default_from() -> String {
    "file".to_string()
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    })
}

fn names(specifier: &TypeOrValueSpecifier) -> &[String] {
    match specifier {
        TypeOrValueSpecifier::Name(name) => std::slice::from_ref(name),
        TypeOrValueSpecifier::Source(source) => &source.name,
    }
}

fn declarations_of_type(t: P<Type>) -> &'static [P<Node>] {
    t.symbol()
        .or_else(|| t.alias().and_then(|a| a.symbol()))
        .map_or(&[], |s| s.declarations())
}

fn declaration_files(declarations: &[P<Node>]) -> Vec<P<SourceFile>> {
    declarations
        .iter()
        .filter_map(|&d| ast::get_source_file_of_node(d))
        .collect()
}

fn package_name_from_node_modules_path(file_name: &str) -> Option<String> {
    let components: Vec<_> = tspath::normalize_slashes(file_name)
        .split('/')
        .map(str::to_string)
        .collect();
    for index in (0..components.len()).rev() {
        if components[index] != "node_modules" || index + 1 >= components.len() {
            continue;
        }
        if components[index + 1].starts_with('@') && index + 2 < components.len() {
            return Some(format!(
                "{}/{}",
                components[index + 1],
                components[index + 2]
            ));
        }
        return Some(components[index + 1].clone());
    }
    None
}

fn contains_node_modules_segment(path: &str) -> bool {
    format!("/{}/", tspath::normalize_slashes(path)).contains("/node_modules/")
}

fn package_name_from_nearest_package_json(
    program: &'static Program,
    file_name: &str,
) -> Option<String> {
    if !contains_node_modules_segment(file_name) {
        return None;
    }
    let mut directory = tspath::get_directory_path(file_name);
    while !directory.is_empty() && contains_node_modules_segment(&directory) {
        let path = tspath::combine_paths(&directory, &["package.json"]);
        if let Some(contents) = program.host().fs().read_file(&path) {
            if let Some(name) = serde_json::from_str::<serde_json::Value>(&contents)
                .ok()
                .and_then(|value| value.get("name")?.as_str().map(str::to_string))
                .filter(|name| !name.is_empty())
            {
                return Some(name);
            }
        }
        let parent = tspath::get_directory_path(&directory);
        if parent == directory {
            break;
        }
        directory = parent;
    }
    None
}

fn package_name_for_file(program: &'static Program, file: P<SourceFile>) -> Option<String> {
    package_name_from_nearest_package_json(program, file.file_name())
        .or_else(|| package_name_from_node_modules_path(file.file_name()))
}

fn parent_ambient_module_name(mut node: P<Node>) -> Option<&'static str> {
    loop {
        match node.kind() {
            Kind::ModuleDeclaration => {
                let declaration = node.as_module_declaration();
                if declaration.keyword() != Kind::NamespaceKeyword {
                    return ast::is_string_literal(declaration.name())
                        .then(|| declaration.name().text());
                }
            }
            Kind::SourceFile => return None,
            _ => {}
        }
        node = node.parent()?;
    }
}

fn path_is_in_package(package_path: &str, package_name: &str) -> bool {
    package_path == package_name
        || package_path
            .strip_prefix(package_name)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn types_package_name(package_name: &str) -> String {
    let Some(scoped) = package_name.strip_prefix('@') else {
        return package_name.to_string();
    };
    let Some((scope, name)) = scoped.split_once('/') else {
        return package_name.to_string();
    };
    if scope.is_empty() || name.is_empty() {
        package_name.to_string()
    } else {
        format!("{scope}__{name}")
    }
}

fn source_matches(
    specifier: &TypeOrValueSpecifier,
    declarations: &[P<Node>],
    program: &'static Program,
) -> bool {
    let TypeOrValueSpecifier::Source(source) = specifier else {
        return true;
    };
    let files = declaration_files(declarations);
    match source.from.as_str() {
        "file" | "" => {
            let cwd = program.host().get_current_directory();
            if source.path.is_empty() {
                files.iter().any(|file| file.file_name().starts_with(cwd))
            } else {
                let absolute = tspath::get_normalized_absolute_path(&source.path, cwd);
                files.iter().any(|file| file.file_name() == absolute)
            }
        }
        "lib" => {
            files.is_empty()
                || files
                    .iter()
                    .any(|file| program.is_source_file_default_library(file.path()))
        }
        "package" => {
            declarations.iter().copied().any(|declaration| {
                parent_ambient_module_name(declaration) == Some(source.package.as_str())
            }) || if source.package.is_empty() {
                false
            } else {
                let types_name = types_package_name(&source.package);
                files.iter().copied().any(|file| {
                    program.is_source_file_from_external_library(file)
                        && package_name_for_file(program, file).is_some_and(|name| {
                            path_is_in_package(&name, &source.package)
                                || name
                                    .strip_prefix("@types/")
                                    .is_some_and(|name| path_is_in_package(name, &types_name))
                        })
                })
            }
        }
        _ => false,
    }
}

fn type_name_matches(t: P<Type>, specifier: &TypeOrValueSpecifier) -> bool {
    let wanted = names(specifier);
    if let Some(symbol) = t.alias().and_then(|a| a.symbol()).or_else(|| t.symbol()) {
        if wanted.iter().any(|name| name == symbol.name()) {
            return true;
        }
    }
    t.flags().intersects(TypeFlags::Intrinsic)
        && wanted
            .iter()
            .any(|name| name == t.as_intrinsic_type().intrinsic_name())
}

pub fn type_matches_some_specifier(
    t: P<Type>,
    specifiers: &[TypeOrValueSpecifier],
    program: &'static Program,
) -> bool {
    let matches = |part: P<Type>| {
        !part.flags().intersects(TypeFlags::Intrinsic)
            || part.as_intrinsic_type().intrinsic_name() != "error"
    };
    let test = |part: P<Type>| {
        matches(part)
            && specifiers.iter().any(|specifier| {
                type_name_matches(part, specifier)
                    && source_matches(specifier, declarations_of_type(part), program)
            })
    };
    test(t) || t.flags().intersects(TypeFlags::Intersection) && t.types().iter().copied().any(test)
}

fn static_name(node: P<Node>) -> Option<&'static str> {
    if ast::is_identifier(node) {
        Some(node.as_identifier().text())
    } else if ast::is_private_identifier(node) {
        Some(node.as_private_identifier().text().trim_start_matches('#'))
    } else if ast::is_string_literal(node) {
        Some(node.text())
    } else {
        None
    }
}

fn symbol_matches(
    symbol: P<Symbol>,
    name: &str,
    specifier: &TypeOrValueSpecifier,
    program: &'static Program,
) -> bool {
    names(specifier).iter().any(|candidate| candidate == name)
        && source_matches(specifier, symbol.declarations(), program)
}

pub fn value_matches_some_specifier(
    node: P<Node>,
    specifiers: &[TypeOrValueSpecifier],
    program: &'static Program,
    checker: &mut Checker,
) -> bool {
    let Some(name) = static_name(node) else {
        return false;
    };
    for specifier in specifiers {
        if !names(specifier).iter().any(|candidate| candidate == name) {
            continue;
        }
        if matches!(specifier, TypeOrValueSpecifier::Name(_)) {
            return true;
        }
        let Some(mut symbol) = checker.get_symbol_at_location_exported(node) else {
            continue;
        };
        if symbol
            .value_declaration()
            .is_some_and(ast::is_shorthand_property_assignment)
        {
            if let Some(value_symbol) =
                checker.get_shorthand_assignment_value_symbol(symbol.value_declaration())
            {
                symbol = value_symbol;
            }
        }
        if symbol
            .value_declaration()
            .is_some_and(ast::is_binding_element)
        {
            let declaration = symbol.value_declaration().unwrap();
            if declaration
                .parent()
                .is_some_and(ast::is_object_binding_pattern)
                && declaration
                    .as_binding_element()
                    .dot_dot_dot_token()
                    .is_none()
            {
                let Some(property_name) = declaration.property_name_or_name().and_then(static_name)
                else {
                    continue;
                };
                if property_name.is_empty() {
                    continue;
                }
                let source_type = checker.get_type_at_location(declaration.parent().unwrap());
                let Some(property_symbol) =
                    checker.get_property_of_type(source_type, property_name)
                else {
                    continue;
                };
                symbol = property_symbol;
            }
        }
        if symbol.flags().intersects(SymbolFlags::Alias) {
            symbol = checker.get_aliased_symbol(symbol);
        }
        if symbol_matches(symbol, name, specifier, program) {
            return true;
        }
    }
    false
}

pub fn union_parts(t: P<Type>) -> Vec<P<Type>> {
    if t.flags().intersects(TypeFlags::Union) {
        t.types().to_vec()
    } else {
        vec![t]
    }
}

pub fn is_builtin_promise_like(
    program: &'static Program,
    checker: &mut Checker,
    t: P<Type>,
) -> bool {
    fn recur(program: &'static Program, checker: &mut Checker, t: P<Type>) -> bool {
        if t.flags().intersects(TypeFlags::Intersection) {
            return t
                .types()
                .iter()
                .copied()
                .any(|part| recur(program, checker, part));
        }
        if t.flags().intersects(TypeFlags::Union) {
            return t
                .types()
                .iter()
                .copied()
                .all(|part| recur(program, checker, part));
        }
        if t.flags().intersects(TypeFlags::TypeParameter) {
            return checker
                .get_base_constraint_of_type(t)
                .is_some_and(|constraint| recur(program, checker, constraint));
        }
        if let Some(symbol) = t.symbol() {
            if symbol.name() == "Promise"
                && symbol
                    .declarations()
                    .iter()
                    .filter_map(|&d| ast::get_source_file_of_node(d))
                    .any(|f| program.is_source_file_default_library(f.path()))
            {
                return true;
            }
            if symbol
                .flags()
                .intersects(SymbolFlags::Class | SymbolFlags::Interface)
            {
                let declared = checker.get_declared_type_of_symbol(symbol);
                return checker
                    .get_base_types(declared)
                    .iter()
                    .copied()
                    .any(|base| recur(program, checker, base));
            }
        }
        false
    }
    recur(program, checker, t)
}
