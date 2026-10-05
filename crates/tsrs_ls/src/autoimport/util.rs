use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, Kind, Symbol, SymbolFlags};
use tsrs_checker::{self as checker, Checker};
use tsrs_compiler::Program;
use tsrs_core::collections::{new_set_with_size_hint, Set};
use tsrs_core::context::Context;
use tsrs_core::stringutil::{self, Rune};
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_module::packagejson::PackageJson;
use tsrs_module::{self as module, DefaultResolver, ResolutionHost, ResolverOptions};
use tsrs_vfs::{wrapvfs, FS};

use super::export::ModuleID;
use super::registry::RegistryCloneHost;

// util.go:23
pub(crate) fn try_get_module_id_and_file_name_of_module_symbol(symbol: P<Symbol>) -> Option<(ModuleID, String)> {
    if !symbol.is_external_module() {
        return None;
    }
    let decl = ast::get_non_augmentation_declaration(symbol)?;
    if decl.kind() == Kind::SourceFile {
        let file = decl.as_source_file();
        return Some((ModuleID(file.path().to_string()), file.file_name().to_string()));
    }
    if ast::is_module_with_string_literal_name(decl) {
        return Some((ModuleID(decl.name().unwrap().text().to_string()), String::new()));
    }
    None
}

// Go `unicode.IsLower`. Go's table is the Ll category; Rust's `char::is_lowercase` is the Lowercase property
// (Ll plus Other_Lowercase), which differs only for a few modifier letters and symbols.
pub(crate) fn go_unicode_is_lower(r: Rune) -> bool {
    if (0..0x80).contains(&r) {
        return (b'a' as Rune..=b'z' as Rune).contains(&r);
    }
    char::from_u32(r as u32).is_some_and(|c| c.is_lowercase())
}

// util.go:66
// wordIndices splits an identifier into its constituent words based on camelCase and snake_case conventions
// by returning the starting byte indices of each word. The first index is always 0.
//   - CamelCase
//     ^    ^
//   - snake_case
//     ^     ^
//   - ParseURL
//     ^    ^
//   - __proto__
//     ^
pub(crate) fn word_indices(s: &str) -> Vec<usize> {
    let mut indices = Vec::new();
    let bytes = s.as_bytes();
    for (byte_index, rune_value) in s.char_indices() {
        if byte_index == 0 {
            indices.push(byte_index);
            continue;
        }
        if rune_value == '_' {
            if byte_index + 1 < s.len() && bytes[byte_index + 1] != b'_' {
                indices.push(byte_index + 1);
            }
            continue;
        }
        let r = rune_value as Rune;
        if stringutil::unicode_is_upper(r) {
            let (prev, _) = stringutil::decode_last_rune(&bytes[..byte_index]);
            let next_is_lower = byte_index + 1 < s.len() && {
                let (next, _) = stringutil::decode_rune(&bytes[byte_index + 1..]);
                go_unicode_is_lower(next)
            };
            if go_unicode_is_lower(prev) || next_is_lower {
                indices.push(byte_index);
            }
        }
    }
    indices
}

// util.go:87
pub(crate) fn get_package_names_in_node_modules(node_modules_dir: &str, fs: &dyn FS) -> Set<String> {
    let mut package_names = Set::new();
    if tspath::get_base_file_name(node_modules_dir) != "node_modules" {
        panic!("nodeModulesDir is not a node_modules directory");
    }
    // A missing node_modules directory yields no entries (GetAccessibleEntries returns
    // empty), so there's no need to check existence first: a deleted node_modules is
    // handled upstream in updateBucketAndDirectoryExistence, which drops the bucket.
    let entries = fs.get_accessible_entries(node_modules_dir);
    for base_name in &entries.directories {
        if base_name.as_bytes()[0] == b'.' {
            continue;
        }
        if base_name.as_bytes()[0] == b'@' {
            let scoped_dir_path = tspath::combine_paths(node_modules_dir, &[base_name]);
            for scoped_package_dir_name in &fs.get_accessible_entries(&scoped_dir_path).directories {
                let scoped_base_name = tspath::get_base_file_name(scoped_package_dir_name);
                if base_name == "@types" {
                    package_names.add(module::get_package_name_from_types_package_name(&tspath::combine_paths("@types", &[&scoped_base_name])));
                } else {
                    package_names.add(tspath::combine_paths(base_name, &[&scoped_base_name]));
                }
            }
            continue;
        }
        package_names.add(base_name.clone());
    }
    package_names
}

// util.go:118
pub(crate) fn get_default_like_export_name_from_declaration(symbol: P<Symbol>) -> String {
    for &d in symbol.declarations() {
        // "export default" in this case. See `ExportAssignment`for more details.
        if ast::is_export_assignment(d) {
            let inner_expression = ast::skip_outer_expressions(d.expression().unwrap(), ast::OEK::All);
            if ast::is_identifier(inner_expression) {
                return inner_expression.text().to_string();
            }
            continue;
        }
        // "export { ~ as default }"
        if ast::is_export_specifier(d) && d.symbol().unwrap().flags() == SymbolFlags::Alias && d.property_name().is_some() {
            if d.property_name().unwrap().kind() == Kind::Identifier {
                return d.property_name().unwrap().text().to_string();
            }
            continue;
        }
        // GH#52694
        if let Some(name) = ast::get_name_of_declaration(Some(d)) {
            if name.kind() == Kind::Identifier {
                return name.text().to_string();
            }
        }
        if let Some(parent) = symbol.parent() {
            if !checker::is_external_module_symbol(parent) {
                return parent.name().to_string();
            }
        }
    }
    String::new()
}

// util.go:146
pub(crate) fn get_resolved_package_names(ctx: &Context, program: &'static Program) -> Set<String> {
    let raw_names = program.resolved_package_names();
    let unresolved_package_names = program.unresolved_package_names();

    // Normalize @types/ package names to their actual package names
    // (e.g., "@types/react" → "react"). ResolvedPackageNames can contain
    // @types names when the program resolves an import like "react" to
    // "@types/react/index.d.ts" via the PackageId.Name field.
    let mut resolved_package_names = new_set_with_size_hint(raw_names.len());
    #[expect(clippy::iter_over_hash_type, reason = "only adds to a set; Go ranges the set too")]
    for name in raw_names.keys() {
        resolved_package_names.add(module::get_package_name_from_types_package_name(name));
    }

    if let Some(types) = &program.options().types {
        for name in types {
            if name != "*" {
                resolved_package_names.add(module::get_package_name_from_types_package_name(name));
            }
        }
    }

    if unresolved_package_names.len() > 0 {
        let mut checker = program.get_type_checker(ctx);
        #[expect(clippy::iter_over_hash_type, reason = "read-only globals lookup, then adds to a set; Go ranges the set too")]
        for name in unresolved_package_names.keys() {
            if let Some(symbol) = checker.try_find_ambient_module_exported(name) {
                let declaring_file = ast::get_source_file_of_module(symbol).unwrap();
                let package_name = tsrs_modulespecifiers::get_package_name_from_directory(declaring_file.file_name());
                if !package_name.is_empty() {
                    resolved_package_names.add(module::get_package_name_from_types_package_name(&package_name));
                }
            }
        }
    }
    resolved_package_names
}

// util.go:186
// addProjectReferenceOutputMappings adds output .d.ts to source file mappings
// from a program's project references to the provided map.
// This is used during node_modules bucket building to redirect extraction
// from output files to source files when the output is from a project reference.
pub(crate) fn add_project_reference_output_mappings(program: &'static Program, result: &mut FxHashMap<Path, String>) {
    let refs = program.get_resolved_project_references();
    for r in refs {
        let Some(r) = r else {
            continue;
        };
        tsrs_tsoptions::ParsedCommandLine::parse_input_output_names(r);
        if let Some(mappings) = r.output_dts_to_project_reference() {
            #[expect(clippy::iter_over_hash_type, reason = "keys are distinct within one reference's map; first-wins runs across the refs slice; Go ranges the map too")]
            for (output_dts_path, mapping) in mappings {
                // Only add if not already present (first program wins)
                result.entry(output_dts_path.clone()).or_insert_with(|| mapping.source.clone());
            }
        }
    }
}

// util.go:203
// Go's pool hands out up to GOMAXPROCS checkers to the extraction goroutines; extraction runs sequentially here,
// so the pool holds the one checker the first request creates.
pub(crate) struct checkerPool {
    program: &'static dyn checker::Program,
    checker: Option<Box<Checker>>,
    created: i32,
}

pub(crate) fn create_checker_pool(program: &'static dyn checker::Program) -> checkerPool {
    checkerPool { program, checker: None, created: 0 }
}

impl checkerPool {
    pub(crate) fn get_checker(&mut self) -> &mut Checker {
        if self.checker.is_none() {
            self.checker = Some(checker::new_checker(self.program));
            self.created += 1;
        }
        self.checker.as_mut().unwrap()
    }

    pub(crate) fn created_count(&self) -> i32 {
        self.created
    }
}

// util.go:240
// addPackageJsonDependencies adds all dependencies and peerDependencies from a package.json
// to the given set, canonicalizing @types package names to their base names.
pub(crate) fn add_package_json_dependencies(contents: &PackageJson, deps: &mut Set<String>) {
    contents.range_dependencies(|name, _, field| {
        if name.is_empty() || name == "@types/" || name.as_bytes()[0] == b'.' {
            // Edge cases that could make us blow up probably
            return true;
        }
        if field == "dependencies" || field == "peerDependencies" {
            deps.add(module::get_package_name_from_types_package_name(name));
        }
        true
    });
}

pub(crate) type PathFunc = Arc<dyn Fn(&str) -> String + Send + Sync>;

// util.go:261
// getPackageRealpathFuncs returns functions to transform between symlink and realpath for files within a package.
// It calls FS.Realpath once per package directory and uses prefix substitution for files within that directory,
// avoiding expensive realpath syscalls for each file. For files outside the package (e.g. re-exported
// dependencies reached through node_modules symlinks), it resolves the file's directory realpath once,
// finds the symlink boundary (the package root where the symlink lives), and caches that prefix mapping.
// All subsequent files under the same symlinked package directory use prefix substitution with no syscalls.
pub(crate) fn get_package_realpath_funcs(fs: &'static dyn FS, package_dir: &str) -> (PathFunc, PathFunc) {
    let real_package_dir = fs.realpath(package_dir);
    let is_symlinked = real_package_dir != package_dir;
    // Cache of package-directory-level symlink→realpath prefix mappings for
    // external packages encountered via re-exports. Keyed by the node_modules
    // package directory (e.g. "/app/node_modules/dep"), so all files under
    // that package reuse a single realpath lookup.
    let dir_cache: Mutex<FxHashMap<String, String>> = Mutex::new(FxHashMap::default());
    let package_dir_owned = package_dir.to_string();
    let real_package_dir_owned = real_package_dir.clone();
    let to_realpath: PathFunc = Arc::new(move |file_name: &str| -> String {
        // Fast path: files within the package use prefix substitution.
        if is_symlinked {
            if let Some(after) = file_name.strip_prefix(package_dir_owned.as_str()) {
                return format!("{}{}", real_package_dir_owned, after);
            }
        }
        // Files outside the package (e.g. re-exports into symlinked deps):
        // find the node_modules package directory, resolve it once, and cache.
        let pkg_dir = module::parse_node_module_from_path(file_name, false /*isFolder*/);
        if pkg_dir.is_empty() {
            return file_name.to_string();
        }
        let cached = dir_cache.lock().unwrap().get(&pkg_dir).cloned();
        if let Some(real_dir) = cached {
            if real_dir == pkg_dir {
                return file_name.to_string();
            }
            return format!("{}{}", real_dir, &file_name[pkg_dir.len()..]);
        }
        let real_dir = fs.realpath(&pkg_dir);
        dir_cache.lock().unwrap().insert(pkg_dir.clone(), real_dir.clone());
        if real_dir == pkg_dir {
            return file_name.to_string();
        }
        format!("{}{}", real_dir, &file_name[pkg_dir.len()..])
    });
    if !is_symlinked {
        return (to_realpath, Arc::new(|s: &str| s.to_string()));
    }
    // toSymlink only handles files within the package directory (reversing the
    // packageDir→realPackageDir substitution). It does not handle arbitrary external
    // paths; callers should only use it for files known to be within the package.
    let package_dir_owned = package_dir.to_string();
    let to_symlink: PathFunc = Arc::new(move |file_name: &str| -> String {
        if let Some(after) = file_name.strip_prefix(real_package_dir.as_str()) {
            return format!("{}{}", package_dir_owned, after);
        }
        file_name.to_string()
    });
    (to_realpath, to_symlink)
}

// util.go:318
struct resolutionHost {
    fs: wrapvfs::wrappedFS,
    current_directory: String,
}

impl ResolutionHost for resolutionHost {
    // util.go:325
    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }

    // util.go:329
    fn fs(&self) -> &dyn FS {
        &self.fs
    }
}

// util.go:333
// The resolver keeps its host for its whole life (module.ResolverOptions.Host is 'static); the host lives in the
// current arena, which is the registry update's scratch region (registry.go `Clone`), like the resolver.
pub(crate) fn get_module_resolver(host: &'static dyn RegistryCloneHost, realpath: PathFunc) -> DefaultResolver {
    let rh: &'static resolutionHost = tsrs_core::alloc(resolutionHost {
        fs: wrapvfs::wrap(host.fs(), wrapvfs::Replacements { realpath: Some(Box::new(move |s: &str| realpath(s))), ..Default::default() }),
        current_directory: host.get_current_directory().to_string(),
    });
    let opts = ResolverOptions::new(rh, tsrs_core::empty_compiler_options());
    module::new_resolver(opts)
}
