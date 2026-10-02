// statebaseline.go: the project-state baseline of `// @stateBaseline: true` tests (requests and notifications,
// the test FS and its changes, and diffs of the session's projects, open files and config file registry).
//
// Go's diff tables hold closures that print lazily; the closures only read values captured when they are added,
// so rendering each entry to a string when it is added prints the same text.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use tsrs_compiler::Program;
use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;
use tsrs_core::tspath;
use tsrs_ls::lsconv;
use tsrs_lsproto as lsproto;
use tsrs_project::{ConfigFileRegistry, Snapshot};
use tsrs_vfs::iovfs::IoVFS;
use tsrs_vfs::vfstest::MapFS;
use tsrs_vfs::FS;

use crate::baselineutil::is_lib_file;
use crate::fourslash::FourslashTest;
use crate::fsbaselineutil::FSDiffer;
use crate::testing::T;

// statebaseline.go:25
pub struct StateBaseline {
    pub(crate) baseline: String,
    fs_differ: FSDiffer,
    pub(crate) is_initialized: bool,

    serialized_projects: HashMap<String, projectInfo>,
    // Go's `*compiler.Program` keeps the program alive (GC); the project values that own the serialized programs
    // (docs/LSP.md memory regions) are kept until the next diff.
    serialized_programs_owner: Option<Box<dyn std::any::Any>>,
    serialized_open_files: HashMap<String, openFileInfo>,
    serialized_config_file_registry: Option<Arc<ConfigFileRegistry>>,
}

// statebaseline.go:35
pub(crate) fn new_state_baseline(fs_from_map: Arc<IoVFS<MapFS>>) -> StateBaseline {
    let use_case_sensitive_file_names = fs_from_map.use_case_sensitive_file_names();
    let mut state_baseline = StateBaseline {
        baseline: String::new(),
        fs_differ: FSDiffer::new(fs_from_map),
        is_initialized: false,
        serialized_projects: HashMap::new(),
        serialized_programs_owner: None,
        serialized_open_files: HashMap::new(),
        serialized_config_file_registry: None,
    };
    state_baseline.baseline.push_str(&format!("UseCaseSensitiveFileNames: {}\n", use_case_sensitive_file_names));
    let mut b = std::mem::take(&mut state_baseline.baseline);
    state_baseline.fs_differ.baseline_fs_with_diff(&mut b);
    state_baseline.baseline = b;
    state_baseline
}

// Go `projectInfo = *compiler.Program` (nil = no program).
type projectInfo = Option<&'static Program>;

fn same_program(a: projectInfo, b: projectInfo) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

#[derive(Clone, Default)]
struct openFileInfo {
    default_project_name: String,
    all_projects: Vec<String>,
}

#[derive(Clone, Copy, Default)]
struct diffTableOptions {
    indent: &'static str,
    sort_keys: bool,
}

// statebaseline.go:120
#[derive(Default)]
struct diffTable {
    diff: OrderedMap<String, String>,
    options: diffTableOptions,
}

impl diffTable {
    fn new(options: diffTableOptions) -> diffTable {
        diffTable { diff: OrderedMap::default(), options }
    }

    // statebaseline.go:125
    fn add(&mut self, key: &str, value: &str) {
        self.diff.insert(key.to_string(), value.to_string());
    }

    // statebaseline.go:129
    fn print(&self, w: &mut String, header: &str) {
        let count = self.diff.len();
        if count == 0 {
            return;
        }
        if !header.is_empty() {
            w.push_str(&format!("{}{}\n", self.options.indent, header));
        }
        let mut diff_keys: Vec<&String> = Vec::with_capacity(count);
        let mut key_width = 0;
        let indent = format!("{}  ", self.options.indent);
        for key in self.diff.keys() {
            key_width = key_width.max(key.len());
            diff_keys.push(key);
        }
        if self.options.sort_keys {
            diff_keys.sort();
        }

        for key in diff_keys {
            let value = self.diff.get(key).cloned().unwrap_or_default();
            // Go `%-*s`: pads to the width in runes.
            let pad = (key_width + 1).saturating_sub(key.chars().count());
            w.push_str(&format!("{}{}{} {}\n", indent, key, " ".repeat(pad), value));
        }
    }
}

// statebaseline.go:157
struct diffTableWriter {
    has_change: bool,
    header: &'static str,
    diffs: BTreeMap<String, String>,
}

impl diffTableWriter {
    // statebaseline.go:163
    fn new(header: &'static str) -> diffTableWriter {
        diffTableWriter { has_change: false, header, diffs: BTreeMap::new() }
    }

    // statebaseline.go:167
    fn set_has_change(&mut self) {
        self.has_change = true;
    }

    // statebaseline.go:171
    fn add(&mut self, key: &str, rendered: String) {
        self.diffs.insert(key.to_string(), rendered);
    }

    // statebaseline.go:175
    fn print(&self, w: &mut String) {
        if self.has_change {
            w.push_str(&format!("{}::\n", self.header));
            for rendered in self.diffs.values() {
                w.push_str(rendered);
            }
        }
    }
}

// statebaseline.go:186
fn are_iter_seq_equal(a: &[String], b: &[String]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort();
    b.sort();
    a == b
}

// statebaseline.go:194
fn print_slices_with_diff_table(
    w: &mut String,
    header: &str,
    new_slice: &[String],
    get_old_slice: impl FnOnce() -> Vec<String>,
    options: diffTableOptions,
    top_change: &str,
    is_default: Option<&dyn Fn(&str) -> bool>,
) {
    let old_slice = if top_change == "*modified*" { get_old_slice() } else { Vec::new() };
    let mut table = diffTable::new(options);
    for entry in new_slice {
        let mut entry_change = "";
        if is_default.is_some_and(|f| f(entry)) {
            entry_change = "(default) ";
        }
        if top_change == "*modified*" && !old_slice.contains(entry) {
            entry_change = "*new*";
        }
        table.add(entry, entry_change);
    }
    if top_change == "*modified*" {
        for entry in &old_slice {
            if !new_slice.contains(entry) {
                table.add(entry, "*deleted*");
            }
        }
    }
    table.print(w, header);
}

// statebaseline.go:222
fn slice_from_iter_seq_string(seq: &[String]) -> Vec<String> {
    let mut result = seq.to_vec();
    result.sort();
    result
}

// statebaseline.go:231
fn print_string_iter_seq_with_diff_table(w: &mut String, header: &str, new_iter_seq: &[String], get_old_iter_seq: impl FnOnce() -> Vec<String>, options: diffTableOptions, top_change: &str) {
    print_slices_with_diff_table(w, header, &slice_from_iter_seq_string(new_iter_seq), || slice_from_iter_seq_string(&get_old_iter_seq()), options, top_change, None);
}

impl FourslashTest {
    // statebaseline.go:51 (`params` is evaluated only when state baselining is enabled)
    pub(crate) fn baseline_request_or_notification(&mut self, t: &T, method: lsproto::Method, params: impl FnOnce() -> Value) {
        t.helper();

        if !self.test_data.is_state_baselining_enabled() {
            return;
        }

        // requestOrMessage{Method, Params} with `params,omitzero`
        let mut o = tsrs_core::collections::OrderedMap::default();
        o.insert("method".to_string(), Value::String(method.0.to_string()));
        let params = params();
        if params != Value::Null {
            o.insert("params".to_string(), params);
        }
        let res = tsrs_core::json::marshal_indent(&Value::Object(o), "", "  ").unwrap_or_default();
        let state_baseline = self.state_baseline.as_mut().unwrap();
        state_baseline.baseline.push('\n');
        state_baseline.baseline.push_str(&res);
        state_baseline.baseline.push('\n');
        state_baseline.is_initialized = true;
    }

    // statebaseline.go:66
    pub(crate) fn baseline_projects_after_notification(&mut self, t: &T, file_name: &str) {
        t.helper();
        if !self.test_data.is_state_baselining_enabled() {
            return;
        }
        // Do hover so we have snapshot to check things on!!
        let (_, result) = self.client().send_request(
            lsproto::TEXT_DOCUMENT_HOVER_INFO,
            lsproto::HoverParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(file_name) },
                position: lsproto::Position { line: 0, character: 0 },
                ..Default::default()
            },
        );
        crate::go::assert::assert(t, result.is_some(), "");
        self.baseline_state(t);
    }

    // statebaseline.go:85
    pub(crate) fn baseline_state(&mut self, t: &T) {
        t.helper();

        if !self.test_data.is_state_baselining_enabled() {
            return;
        }

        let serialized = self.serialized_state(t);
        if !serialized.is_empty() {
            let state_baseline = self.state_baseline.as_mut().unwrap();
            state_baseline.baseline.push('\n');
            state_baseline.baseline.push_str(&serialized);
        }
    }

    // statebaseline.go:99
    fn serialized_state(&mut self, t: &T) -> String {
        t.helper();

        let mut builder = String::new();
        self.state_baseline.as_mut().unwrap().fs_differ.baseline_fs_with_diff(&mut builder);
        if builder.trim().is_empty() {
            builder.clear();
        }

        self.print_state_diff(t, &mut builder);
        builder
    }

    // statebaseline.go:249
    fn print_state_diff(&mut self, t: &T, w: &mut String) {
        if !self.state_baseline.as_ref().unwrap().is_initialized {
            return;
        }
        let session = self.client().server.session().clone();
        let snapshot = session.snapshot();

        self.print_projects_diff(t, &snapshot, w);
        self.print_open_files_diff(t, &snapshot, w);
        self.print_config_file_registry_diff(t, &snapshot, w);
    }

    // statebaseline.go:261
    fn print_projects_diff(&mut self, t: &T, snapshot: &Snapshot, w: &mut String) {
        t.helper();

        let mut current_projects: HashMap<String, projectInfo> = HashMap::new();
        let options = diffTableOptions { indent: "  ", sort_keys: false };
        let mut projects_diff_table = diffTableWriter::new("Projects");
        let state_baseline = self.state_baseline.as_mut().unwrap();

        for project in snapshot.project_collection.projects() {
            let program = project.get_program();
            let mut old_program: projectInfo = None;
            let id = project.id().0;
            current_projects.insert(id.clone(), program);
            let project_change;
            if let Some(existing) = state_baseline.serialized_projects.get(&id) {
                old_program = *existing;
                if !same_program(old_program, program) {
                    project_change = "*modified*";
                    projects_diff_table.set_has_change();
                } else {
                    project_change = "";
                }
            } else {
                project_change = "*new*";
                projects_diff_table.set_has_change();
            }

            let mut out = format!("  [{}] {}\n", id, project_change);
            let mut sub_diff = diffTable::new(options);
            if let Some(program) = program {
                for &file in program.get_source_files() {
                    let mut file_diff = "";
                    // No need to write "*new*" for files as its obvious
                    let file_name = file.file_name();
                    if project_change == "*modified*" {
                        match old_program {
                            None => {
                                if !is_lib_file(&file_name) {
                                    file_diff = "*new*";
                                }
                            }
                            Some(old_program) => match old_program.get_source_file_by_path(&file.path()) {
                                None => file_diff = "*new*",
                                Some(old_file) if old_file != file => file_diff = "*modified*",
                                Some(_) => {}
                            },
                        }
                    }
                    if !file_diff.is_empty() || !is_lib_file(&file_name) {
                        sub_diff.add(&file_name, file_diff);
                    }
                }
            }
            if !same_program(old_program, program) {
                if let Some(old_program) = old_program {
                    for &file in old_program.get_source_files() {
                        if program.is_none_or(|p| p.get_source_file_by_path(&file.path()).is_none()) {
                            sub_diff.add(&file.file_name(), "*deleted*");
                        }
                    }
                }
            }
            sub_diff.print(&mut out, "");
            projects_diff_table.add(&id, out);
        }

        for (project_name, info) in &state_baseline.serialized_projects {
            if !current_projects.contains_key(project_name) {
                projects_diff_table.set_has_change();
                let mut out = format!("  [{}] *deleted*\n", project_name);
                let mut sub_diff = diffTable::new(options);
                if let Some(info) = info {
                    for &file in info.get_source_files() {
                        let file_name = file.file_name();
                        if !is_lib_file(&file_name) {
                            sub_diff.add(&file_name, "");
                        }
                    }
                }
                sub_diff.print(&mut out, "");
                projects_diff_table.add(project_name, out);
            }
        }
        state_baseline.serialized_projects = current_projects;
        state_baseline.serialized_programs_owner = Some(Box::new(snapshot.project_collection.projects()));
        projects_diff_table.print(w);
    }

    // statebaseline.go:348
    fn print_open_files_diff(&mut self, t: &T, snapshot: &Snapshot, w: &mut String) {
        t.helper();

        let mut current_open_files: HashMap<String, openFileInfo> = HashMap::new();
        let mut files_diff_table = diffTableWriter::new("Open Files");
        let options = diffTableOptions { indent: "  ", sort_keys: true };
        let use_case_sensitive_file_names = self.vfs.use_case_sensitive_file_names();
        let open_files: Vec<String> = self.open_files.keys().cloned().collect();
        let state_baseline = self.state_baseline.as_mut().unwrap();
        for file_name in &open_files {
            let path = tspath::to_path(file_name, "/", use_case_sensitive_file_names);
            let default_project = snapshot.project_collection.get_default_project(&path);
            let mut new_file_info = openFileInfo::default();
            if let Some(default_project) = &default_project {
                new_file_info.default_project_name = default_project.id().0;
            }
            for project in snapshot.project_collection.projects() {
                if let Some(program) = project.get_program() {
                    if program.get_source_file_by_path(&path).is_some() {
                        new_file_info.all_projects.push(project.id().0);
                    }
                }
            }
            new_file_info.all_projects.sort();
            current_open_files.insert(file_name.clone(), new_file_info.clone());
            let open_file_change;
            let mut old_file_info = openFileInfo::default();
            if let Some(existing) = state_baseline.serialized_open_files.get(file_name) {
                old_file_info = existing.clone();
                if existing.default_project_name != new_file_info.default_project_name || existing.all_projects != new_file_info.all_projects {
                    open_file_change = "*modified*";
                    files_diff_table.set_has_change();
                } else {
                    open_file_change = "";
                }
            } else {
                open_file_change = "*new*";
                files_diff_table.set_has_change();
            }

            let mut out = format!("  [{}] {}\n", file_name, open_file_change);
            let default_name = new_file_info.default_project_name.clone();
            print_slices_with_diff_table(
                &mut out,
                "",
                &new_file_info.all_projects,
                || old_file_info.all_projects.clone(),
                options,
                open_file_change,
                Some(&|project_name: &str| project_name == default_name),
            );
            files_diff_table.add(file_name, out);
        }
        for file_name in state_baseline.serialized_open_files.keys() {
            if !current_open_files.contains_key(file_name) {
                files_diff_table.set_has_change();
                files_diff_table.add(file_name, format!("  [{}] *closed*\n", file_name));
            }
        }
        state_baseline.serialized_open_files = current_open_files;
        files_diff_table.print(w);
    }

    // statebaseline.go:404
    fn print_config_file_registry_diff(&mut self, t: &T, snapshot: &Snapshot, w: &mut String) {
        t.helper();
        let config_file_registry = snapshot.project_collection.config_file_registry().clone();
        let state_baseline = self.state_baseline.as_mut().unwrap();

        let mut config_diffs_table = diffTableWriter::new("Config");
        let mut config_file_names_diffs_table = diffTableWriter::new("Config File Names");

        if state_baseline.serialized_config_file_registry.as_ref().is_some_and(|r| Arc::ptr_eq(r, &config_file_registry)) {
            return;
        }
        let old_registry = state_baseline.serialized_config_file_registry.clone();
        let options = diffTableOptions { indent: "    ", sort_keys: true };
        let project_ids = |ids: &[tsrs_project::ID]| ids.iter().map(|id| id.0.clone()).collect::<Vec<String>>();
        let paths = |ps: &[tspath::Path]| ps.iter().map(|p| p.to_string()).collect::<Vec<String>>();
        config_file_registry.for_each_test_config_entry(|path, entry| {
            let mut config_change = "";
            let old_entry = old_registry.as_ref().and_then(|r| r.get_test_config_entry(path));
            let (new_projects, new_open_files, new_configs) = (project_ids(&entry.retaining_projects), paths(&entry.retaining_open_files), paths(&entry.retaining_configs));
            let (old_projects, old_open_files, old_configs) = match &old_entry {
                Some(old) => (project_ids(&old.retaining_projects), paths(&old.retaining_open_files), paths(&old.retaining_configs)),
                None => (Vec::new(), Vec::new(), Vec::new()),
            };
            match &old_entry {
                None => {
                    config_change = "*new*";
                    config_diffs_table.set_has_change();
                }
                // Go `oldEntry != entry` compares two freshly built entries, so it is always true.
                Some(_) => {
                    if !are_iter_seq_equal(&old_projects, &new_projects) || !are_iter_seq_equal(&old_open_files, &new_open_files) || !are_iter_seq_equal(&old_configs, &new_configs) {
                        config_change = "*modified*";
                        config_diffs_table.set_has_change();
                    }
                }
            }
            let mut out = format!("  [{}] {}\n", entry.file_name, config_change);
            // Print the details of the config entry
            let mut retaining_projects_modified = "";
            let mut retaining_open_files_modified = "";
            let mut retaining_configs_modified = "";
            if config_change == "*modified*" {
                if !are_iter_seq_equal(&new_projects, &old_projects) {
                    retaining_projects_modified = " *modified*";
                }
                if !are_iter_seq_equal(&new_open_files, &old_open_files) {
                    retaining_open_files_modified = " *modified*";
                }
                if !are_iter_seq_equal(&new_configs, &old_configs) {
                    retaining_configs_modified = " *modified*";
                }
            }
            print_string_iter_seq_with_diff_table(&mut out, &format!("RetainingProjects:{}", retaining_projects_modified), &new_projects, || old_projects.clone(), options, config_change);
            print_string_iter_seq_with_diff_table(&mut out, &format!("RetainingOpenFiles:{}", retaining_open_files_modified), &new_open_files, || old_open_files.clone(), options, config_change);
            print_string_iter_seq_with_diff_table(&mut out, &format!("RetainingConfigs:{}", retaining_configs_modified), &new_configs, || old_configs.clone(), options, config_change);
            config_diffs_table.add(&path.0, out);
        });
        config_file_registry.for_each_test_config_file_names_entry(|path, entry| {
            let mut config_file_names_change = "";
            let old_entry = old_registry.as_ref().and_then(|r| r.get_test_config_file_names_entry(path));
            let empty = Default::default();
            let new_ancestors = entry.ancestors.as_ref().unwrap_or(&empty);
            match &old_entry {
                None => {
                    config_file_names_change = "*new*";
                    config_file_names_diffs_table.set_has_change();
                }
                Some(old) => {
                    if old.nearest_config_file_name != entry.nearest_config_file_name || old.ancestors.as_ref().unwrap_or(&empty) != new_ancestors {
                        config_file_names_change = "*modified*";
                        config_file_names_diffs_table.set_has_change();
                    }
                }
            }
            let mut out = format!("  [{}] {}\n", path.0, config_file_names_change);
            let mut nearest_config_file_name_modified = "";
            let mut ancestor_diff_modified = "";
            let old_ancestors = old_entry.as_ref().and_then(|o| o.ancestors.clone()).unwrap_or_default();
            if config_file_names_change == "*modified*" {
                let old = old_entry.as_ref().unwrap();
                if old.nearest_config_file_name != entry.nearest_config_file_name {
                    nearest_config_file_name_modified = " *modified*";
                }
                if &old_ancestors != new_ancestors {
                    ancestor_diff_modified = " *modified*";
                }
            }
            out.push_str(&format!("    NearestConfigFileName: {}{}\n", entry.nearest_config_file_name, nearest_config_file_name_modified));
            let mut ancestor_diff = diffTable::new(options);
            for (config, ancestor_of_config) in new_ancestors {
                let mut ancestor_change = "";
                if config_file_names_change == "*modified*" {
                    match old_ancestors.get(config) {
                        Some(old_config_file_name) => {
                            if old_config_file_name != ancestor_of_config {
                                ancestor_change = "*modified*";
                            }
                        }
                        None => ancestor_change = "*new*",
                    }
                }
                ancestor_diff.add(config, &format!("{} {}", ancestor_of_config, ancestor_change));
            }
            if config_file_names_change == "*modified*" {
                for (ancestor_path, old_config_file_name) in &old_ancestors {
                    if !new_ancestors.contains_key(ancestor_path) {
                        ancestor_diff.add(ancestor_path, &format!("{} *deleted*", old_config_file_name));
                    }
                }
            }
            ancestor_diff.print(&mut out, &format!("Ancestors:{}", ancestor_diff_modified));
            config_file_names_diffs_table.add(&path.0, out);
        });

        if let Some(old_registry) = &old_registry {
            old_registry.for_each_test_config_entry(|path, entry| {
                if config_file_registry.get_test_config_entry(path).is_none() {
                    config_diffs_table.set_has_change();
                    config_diffs_table.add(&path.0, format!("  [{}] *deleted*\n", entry.file_name));
                }
            });
            old_registry.for_each_test_config_file_names_entry(|path, _entry| {
                if config_file_registry.get_test_config_file_names_entry(path).is_none() {
                    config_file_names_diffs_table.set_has_change();
                    config_file_names_diffs_table.add(&path.0, format!("  [{}] *deleted*\n", path.0));
                }
            });
        }
        state_baseline.serialized_config_file_registry = Some(config_file_registry);
        config_diffs_table.print(w);
        config_file_names_diffs_table.print(w);
    }
}
