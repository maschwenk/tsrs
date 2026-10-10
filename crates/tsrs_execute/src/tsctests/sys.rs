// Port of execute/tsctests/sys.go and fs.go (non-watch), plus testutil/fsbaselineutil (FSDiffer) and
// harnessutil.TracerForBaselining.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_vfs::iovfs::IoVFS;
use tsrs_vfs::vfstest::MapFS;
use tsrs_vfs::{Entries, FileInfo, FileMode, FS};

use super::readablebuildinfo::to_readable_build_info;
use crate::tsc::{CommandLineTesting, MTimesCache, SyncWriter, System};

pub(crate) const FAKE_TS_VERSION: &str = "FakeTSVersion";
pub(crate) const FAKE_TIME_STAMP: &str = "HH:MM:SS AM";

const LIST_FILE_START: &str = "!!! List files start";
const LIST_FILE_END: &str = "!!! List files end";
const STATISTICS_START: &str = "!!! Statistics start";
const STATISTICS_END: &str = "!!! Statistics end";
const BUILD_STATUS_REPORT_START: &str = "!!! Build Status Report Start";
const BUILD_STATUS_REPORT_END: &str = "!!! Build Status Report End";
const TRACE_START: &str = "!!! Trace start";
const TRACE_END: &str = "!!! Trace end";

// sys.go TestClock
pub(crate) struct TestClock {
    start: SystemTime,
    now: Mutex<Option<SystemTime>>,
}

impl TestClock {
    pub(crate) fn new() -> Arc<TestClock> {
        Arc::new(TestClock { start: SystemTime::now(), now: Mutex::new(None) })
    }

    pub(crate) fn now(&self) -> SystemTime {
        let mut now = self.now.lock().unwrap();
        let t = now.unwrap_or(self.start) + Duration::from_secs(1); // Simulate some time passing
        *now = Some(t);
        t
    }
}

// fs.go testFs
pub(crate) struct testFs {
    pub(crate) fs: Arc<IoVFS<MapFS>>,
    pub(crate) default_libs: Mutex<Option<FxHashSet<String>>>,
    pub(crate) written_files: Mutex<FxHashSet<String>>,
}

impl testFs {
    fn remove_ignore_lib_path(&self, path: &str) {
        if let Some(libs) = self.default_libs.lock().unwrap().as_mut() {
            libs.remove(path);
        }
    }

    fn read_file_handling_build_info(&self, path: &str) -> Option<String> {
        let mut contents = self.fs.read_file(path)?;
        if tspath::file_extension_is(path, tspath::EXTENSION_TS_BUILD_INFO) {
            // read buildinfo and modify version
            if let Ok(mut build_info) = tsrs_incremental::BuildInfo::unmarshal(&contents) {
                if build_info.version == FAKE_TS_VERSION {
                    build_info.version = tsrs_core::version().to_string();
                    contents = build_info.marshal();
                }
            }
        }
        Some(contents)
    }

    fn write_file_handling_build_info(&self, path: &str, data: &str) -> Result<(), String> {
        let mut data = data.to_string();
        if tspath::file_extension_is(path, tspath::EXTENSION_TS_BUILD_INFO) {
            match tsrs_incremental::BuildInfo::unmarshal(&data) {
                Ok(mut build_info) => {
                    if build_info.version == tsrs_core::version() {
                        // Change it to harnessutil.FakeTSVersion
                        build_info.version = FAKE_TS_VERSION.to_string();
                        data = build_info.marshal();
                    }
                    // Write readable build info version
                    self.write_file(&format!("{path}.readable.baseline.txt"), &to_readable_build_info(&build_info, &sanitize_internal_symbol_name(&data)))?;
                }
                Err(err) => panic!("testFs.WriteFile: failed to unmarshal build info: {err}"),
            }
        }
        self.fs.write_file(path, &data)
    }
}

impl FS for testFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.fs.use_case_sensitive_file_names()
    }
    fn file_exists(&self, path: &str) -> bool {
        self.fs.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.remove_ignore_lib_path(path);
        self.read_file_handling_build_info(path)
    }
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.remove_ignore_lib_path(path);
        self.written_files.lock().unwrap().insert(path.to_string());
        self.write_file_handling_build_info(path, data)
    }
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.fs.append_file(path, data)
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        self.remove_ignore_lib_path(path);
        self.fs.remove(path)
    }
    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.fs.chtimes(path, a_time, m_time)
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.fs.directory_exists(path)
    }
    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.fs.get_accessible_entries(path)
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.fs.stat(path)
    }
    fn realpath(&self, path: &str) -> String {
        self.fs.realpath(path)
    }
}

// fsbaselineutil DiffEntry / Snapshot
#[derive(Clone)]
struct DiffEntry {
    content: String,
    m_time: Option<SystemTime>,
    is_written: bool,
    symlink_target: String,
}

struct Snapshot {
    snap: FxHashMap<String, DiffEntry>,
    default_libs: FxHashSet<String>,
}

pub(crate) struct TestSys {
    pub(crate) current_write: Mutex<String>,
    program_baselines: Mutex<String>,
    program_include_baselines: Mutex<String>,
    tracer: Mutex<FxHashMap<Path, bool>>, // TracerForBaselining.packageJsonCache
    serialized_diff: Mutex<Option<Snapshot>>,

    pub(crate) fs: Arc<testFs>,
    pub(crate) default_library_path: String,
    pub(crate) cwd: String,
    pub(crate) env: FxHashMap<String, String>,
    pub(crate) output_is_tty: bool,
    pub(crate) clock: Arc<TestClock>,
}

impl TestSys {
    pub(crate) fn new(
        fs: Arc<IoVFS<MapFS>>,
        default_libs: Option<FxHashSet<String>>,
        clock: Arc<TestClock>,
        cwd: String,
        default_library_path: String,
        env: FxHashMap<String, String>,
        output_is_tty: bool,
    ) -> TestSys {
        TestSys {
            current_write: Mutex::new(String::new()),
            program_baselines: Mutex::new(String::new()),
            program_include_baselines: Mutex::new(String::new()),
            tracer: Mutex::new(FxHashMap::default()),
            serialized_diff: Mutex::new(None),
            fs: Arc::new(testFs { fs, default_libs: Mutex::new(default_libs), written_files: Mutex::new(FxHashSet::default()) }),
            default_library_path,
            cwd,
            env,
            output_is_tty,
            clock,
        }
    }

    pub(crate) fn map_fs(&self) -> &MapFS {
        self.fs.fs.fsys()
    }

    pub(crate) fn fs_from_file_map(&self) -> &IoVFS<MapFS> {
        &self.fs.fs
    }

    fn write_header_to_baseline(&self, builder: &mut String, program: &tsrs_incremental::Program) {
        if !builder.is_empty() {
            builder.push('\n');
        }
        let config_file_path = &program.get_program().options().config_file_path;
        if !config_file_path.is_empty() {
            builder.push_str(&tspath::get_relative_path_from_directory(
                &self.cwd,
                config_file_path,
                &ComparePathsOptions { use_case_sensitive_file_names: self.fs.use_case_sensitive_file_names(), current_directory: self.cwd.clone() },
            ));
            builder.push_str("::\n");
        }
    }

    // sys.go baselinePrograms
    pub(crate) fn baseline_programs(&self, baseline: &mut String, header: &str) -> String {
        baseline.push_str(&std::mem::take(&mut *self.program_baselines.lock().unwrap()));
        let mut result = String::new();
        let include = std::mem::take(&mut *self.program_include_baselines.lock().unwrap());
        if !include.is_empty() {
            result += &format!("\n\n{header}\n!!! Include reasons expectations don't match pls review!!!\n");
            result += &include;
            baseline.push_str(&result);
        }
        result
    }

    // sys.go serializeState
    pub(crate) fn serialize_state(&self, baseline: &mut String) {
        baseline.push_str("\nOutput::\n");
        baseline.push_str(&self.get_output(false));
        self.baseline_fs_with_diff(baseline);
    }

    // sys.go getOutput
    pub(crate) fn get_output(&self, for_comparing: bool) -> String {
        let text = self.current_write.lock().unwrap().clone();
        let lines: Vec<&str> = text.split('\n').collect();
        let mut o = outputSanitizer { for_comparing, lines, index: 0, output_lines: Vec::new() };
        o.transform_lines()
    }

    pub(crate) fn clear_output(&self) {
        self.current_write.lock().unwrap().clear();
        self.tracer.lock().unwrap().clear();
    }

    // fsbaselineutil BaselineFSwithDiff
    pub(crate) fn baseline_fs_with_diff(&self, baseline: &mut String) {
        let mut snap: FxHashMap<String, DiffEntry> = FxHashMap::default();
        let mut diffs: FxHashMap<String, String> = FxHashMap::default();
        let written = std::mem::take(&mut *self.fs.written_files.lock().unwrap());
        let mut serialized = self.serialized_diff.lock().unwrap();
        let current_libs = self.fs.default_libs.lock().unwrap().clone();
        for (path, file) in self.map_fs().entries() {
            if file.mode.intersects(FileMode::Symlink) {
                let target = self.map_fs().get_target_of_symlink(&path).unwrap_or_else(|| panic!("Failed to resolve symlink target: {path}"));
                let entry = DiffEntry { content: String::new(), m_time: None, is_written: false, symlink_target: target };
                add_fs_entry_diff(&serialized, current_libs.as_ref(), &mut diffs, Some(&entry), &path);
                snap.insert(path, entry);
            } else if file.mode.is_regular() {
                let content = sanitize_internal_symbol_name(&String::from_utf8_lossy(&file.data));
                let entry = DiffEntry { content, m_time: file.mod_time, is_written: written.contains(&path), symlink_target: String::new() };
                add_fs_entry_diff(&serialized, current_libs.as_ref(), &mut diffs, Some(&entry), &path);
                snap.insert(path, entry);
            }
        }
        if let Some(old) = serialized.as_ref() {
            let mut deleted: Vec<String> = Vec::new();
            for path in old.snap.keys() {
                if self.map_fs().get_file_info(path).is_none() {
                    deleted.push(path.clone());
                }
            }
            for path in deleted {
                add_fs_entry_diff(&serialized, current_libs.as_ref(), &mut diffs, None, &path);
            }
        }
        *serialized = Some(Snapshot { snap, default_libs: current_libs.unwrap_or_default() });
        let mut keys: Vec<&String> = diffs.keys().collect();
        keys.sort();
        for path in keys {
            baseline.push_str(&format!("//// [{}] {}\n", path, diffs[path]));
        }
        baseline.push('\n');
    }

    // TracerForBaselining.sanitizeTrace
    fn sanitize_trace(&self, msg: &str, use_package_json_cache: bool) -> String {
        // Version
        let version = format!("'{}'", tsrs_core::version());
        if msg.contains(&version) {
            return msg.replacen(&version, &format!("'{FAKE_TS_VERSION}'"), 1);
        }
        let opts = |file: &str| tspath::to_path(file, &self.cwd, self.fs.use_case_sensitive_file_names());
        let mut cache = self.tracer.lock().unwrap();
        // caching of fs in trace to be replaces with non caching version
        if let Some(s) = msg.strip_suffix("' does not exist according to earlier cached lookups.") {
            let file = s.strip_prefix("File '").unwrap_or(s);
            if use_package_json_cache {
                let file_path = opts(file);
                if cache.contains_key(&file_path) {
                    return msg.to_string();
                }
                cache.insert(file_path, false);
            }
            return format!("File '{file}' does not exist.");
        }
        if let Some(s) = msg.strip_suffix("' exists according to earlier cached lookups.") {
            let file = s.strip_prefix("File '").unwrap_or(s);
            if use_package_json_cache {
                let file_path = opts(file);
                if cache.contains_key(&file_path) {
                    return msg.to_string();
                }
                cache.insert(file_path, true);
            }
            return format!("Found 'package.json' at '{file}'.");
        }
        if use_package_json_cache {
            if let Some(s) = msg.strip_suffix("' does not exist.") {
                let file = s.strip_prefix("File '").unwrap_or(s);
                let file_path = opts(file);
                if !cache.contains_key(&file_path) {
                    cache.insert(file_path, false);
                    return msg.to_string();
                }
                return format!("File '{file}' does not exist according to earlier cached lookups.");
            }
            if let Some(s) = msg.strip_prefix("Found 'package.json' at '") {
                let file = s.strip_suffix("'.").unwrap_or(s);
                let file_path = opts(file);
                if !cache.contains_key(&file_path) {
                    cache.insert(file_path, true);
                    return msg.to_string();
                }
                return format!("File '{file}' exists according to earlier cached lookups.");
            }
        }
        msg.to_string()
    }
}

// fsbaselineutil addFsEntryDiff
fn add_fs_entry_diff(
    serialized: &Option<Snapshot>,
    current_libs: Option<&FxHashSet<String>>,
    diffs: &mut FxHashMap<String, String>,
    new: Option<&DiffEntry>,
    path: &str,
) {
    let old = serialized.as_ref().and_then(|s| s.snap.get(path));
    let old_libs = serialized.as_ref().map(|s| &s.default_libs);
    match (old, new) {
        (None, Some(new)) => {
            if !current_libs.is_some_and(|l| l.contains(path)) {
                if !new.symlink_target.is_empty() {
                    diffs.insert(path.to_string(), format!("-> {} *new*", new.symlink_target));
                } else {
                    diffs.insert(path.to_string(), format!("*new* \n{}", new.content));
                }
            }
        }
        (Some(_), None) => {
            diffs.insert(path.to_string(), "*deleted*".to_string());
        }
        (Some(old), Some(new)) => {
            if new.content != old.content {
                diffs.insert(path.to_string(), format!("*modified* \n{}", new.content));
            } else if new.is_written {
                diffs.insert(path.to_string(), "*rewrite with same content*".to_string());
            } else if new.m_time != old.m_time {
                diffs.insert(path.to_string(), "*mTime changed*".to_string());
            } else if old_libs.is_some_and(|l| l.contains(path)) && current_libs.is_some_and(|l| !l.contains(path)) {
                // Lib file that was read
                diffs.insert(path.to_string(), format!("*Lib*\n{}", new.content));
            }
        }
        (None, None) => {}
    }
}

// fsbaselineutil SanitizeInternalSymbolName: \uFFFD@name@123 -> \uFFFD@name@<symbolId>
pub(crate) fn sanitize_internal_symbol_name(s: &str) -> String {
    if !s.contains("\u{FFFD}@") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\u{FFFD}@") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "\u{FFFD}@".len()..];
        // [^@]+@[0-9]+
        let name_end = after.find('@');
        let matched = name_end.filter(|&n| n > 0).and_then(|n| {
            let digits = after[n + 1..].bytes().take_while(|b| b.is_ascii_digit()).count();
            (digits > 0).then_some((n, digits))
        });
        match matched {
            Some((n, digits)) => {
                out.push_str("\u{FFFD}@");
                out.push_str(&after[..n]);
                out.push_str("@<symbolId>");
                rest = &after[n + 1 + digits..];
            }
            None => {
                out.push_str("\u{FFFD}@");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

struct outputSanitizer<'a> {
    for_comparing: bool,
    lines: Vec<&'a str>,
    index: usize,
    output_lines: Vec<String>,
}

impl outputSanitizer<'_> {
    fn add_output_line(&mut self, s: &str) {
        let s = s.replace(&format!("'{}'", tsrs_core::version()), &format!("'{FAKE_TS_VERSION}'"));
        let english_version = format!("Version {}", tsrs_core::version());
        let s = s.replace(&english_version, &format!("Version {FAKE_TS_VERSION}"));
        self.output_lines.push(sanitize_internal_symbol_name(&s));
    }

    fn sanitize_build_status_time_stamp(&self) -> String {
        let status_line = self.lines[self.index];
        let hh_separator = status_line.find(':').filter(|&i| i >= 2).expect("Expected timestamp");
        format!("{}{}{}", &status_line[..hh_separator - 2], FAKE_TIME_STAMP, &status_line[hh_separator + FAKE_TIME_STAMP.len() - 2..])
    }

    fn transform_lines(&mut self) -> String {
        while self.index < self.lines.len() {
            let line = self.lines[self.index];
            if !self.add_or_skip_lines_for_comparing(LIST_FILE_START, LIST_FILE_END, false, false)
                && !self.add_or_skip_lines_for_comparing(STATISTICS_START, STATISTICS_END, true, false)
                && !self.add_or_skip_lines_for_comparing(TRACE_START, TRACE_END, false, false)
                && !self.add_or_skip_lines_for_comparing(BUILD_STATUS_REPORT_START, BUILD_STATUS_REPORT_END, false, true)
            {
                self.add_output_line(line);
            }
            self.index += 1;
        }
        self.output_lines.join("\n")
    }

    fn add_or_skip_lines_for_comparing(&mut self, line_start: &str, line_end: &str, skip_even_if_not_comparing: bool, sanitize_first_line: bool) -> bool {
        if self.lines[self.index] != line_start {
            return false;
        }
        self.index += 1;
        let mut is_first_line = true;
        while self.index < self.lines.len() {
            if self.lines[self.index] == line_end {
                return true;
            }
            if !self.for_comparing && !skip_even_if_not_comparing {
                let mut line = self.lines[self.index].to_string();
                if is_first_line && sanitize_first_line {
                    line = self.sanitize_build_status_time_stamp();
                    is_first_line = false;
                }
                self.add_output_line(&line);
            }
            self.index += 1;
        }
        panic!("Expected lineEnd{line_end} not found after {line_start}");
    }
}

impl System for TestSys {
    fn fs(&self) -> Arc<dyn FS> {
        self.fs.clone()
    }
    fn default_library_path(&self) -> &str {
        &self.default_library_path
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
    fn write(&self, text: &str) {
        self.current_write.lock().unwrap().push_str(text);
    }
    fn flush(&self) {}
    fn write_output_is_tty(&self) -> bool {
        self.output_is_tty
    }
    fn get_environment_variable(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn since_start(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn now_time(&self) -> SystemTime {
        self.clock.now()
    }
}

impl CommandLineTesting for TestSys {
    fn on_list_files_start(&self, w: &dyn Fn(&str)) {
        w(&format!("{LIST_FILE_START}\n"));
    }
    fn on_list_files_end(&self, w: &dyn Fn(&str)) {
        w(&format!("{LIST_FILE_END}\n"));
    }
    fn on_statistics_start(&self, w: &dyn Fn(&str)) {
        w(&format!("{STATISTICS_START}\n"));
    }
    fn on_statistics_end(&self, w: &dyn Fn(&str)) {
        w(&format!("{STATISTICS_END}\n"));
    }
    fn on_build_status_report_start(&self, w: &dyn Fn(&str)) {
        w(&format!("{BUILD_STATUS_REPORT_START}\n"));
    }
    fn on_build_status_report_end(&self, w: &dyn Fn(&str)) {
        w(&format!("{BUILD_STATUS_REPORT_END}\n"));
    }

    // sys.go OnEmittedFiles
    fn on_emitted_files(&self, result: Option<&tsrs_compiler::EmitResult>, m_times_cache: Option<&MTimesCache>) {
        let Some(result) = result else { return };
        for file in &result.emitted_files {
            let mod_time = self.map_fs().get_mod_time(file);
            if let Some(serialized) = self.serialized_diff.lock().unwrap().as_ref() {
                if let Some(diff) = serialized.snap.get(file) {
                    if diff.m_time == mod_time {
                        // Even though written, timestamp was reverted
                        continue;
                    }
                }
            }

            // Ensure that the timestamp for emitted files is in the order
            let now = self.clock.now();
            if let Err(err) = self.fs_from_file_map().chtimes(file, now, now) {
                panic!("Failed to change time for emitted file: {file}: {err}");
            }
            // Update the mTime cache in --b mode to store the updated timestamp so tests will behave deteministically when finding newest output
            if let Some(cache) = m_times_cache {
                let path = tspath::to_path(file, &self.cwd, self.fs.use_case_sensitive_file_names());
                let mut cache = cache.lock().unwrap();
                if cache.contains_key(&path) {
                    cache.insert(path, Some(now));
                }
            }
        }
    }

    // sys.go OnProgram
    fn on_program(&self, program: std::sync::Arc<tsrs_incremental::Program>) {
        let mut b = self.program_baselines.lock().unwrap();
        self.write_header_to_baseline(&mut b, &program);

        let p = program.get_program();
        b.push_str("SemanticDiagnostics::\n");
        for &file in p.get_source_files() {
            match program.testing_semantic_diagnostics_state(file.path()) {
                tsrs_incremental::SemanticDiagnosticsState::Refreshed => {
                    b.push_str("*refresh*    ");
                    b.push_str(file.file_name());
                    b.push('\n');
                }
                tsrs_incremental::SemanticDiagnosticsState::NotCached => {
                    b.push_str("*not cached* ");
                    b.push_str(file.file_name());
                    b.push('\n');
                }
                tsrs_incremental::SemanticDiagnosticsState::Reused => {}
            }
        }

        // Write signature updates
        b.push_str("Signatures::\n");
        for &file in p.get_source_files() {
            if let Some(kind) = program.testing_updated_signature_kind(file.path()) {
                b.push_str(match kind {
                    tsrs_incremental::SignatureUpdateKind::ComputedDts => "(computed .d.ts) ",
                    tsrs_incremental::SignatureUpdateKind::StoredAtEmit => "(stored at emit) ",
                    tsrs_incremental::SignatureUpdateKind::UsedVersion => "(used version)   ",
                });
                b.push_str(file.file_name());
                b.push('\n');
            }
        }
        drop(b);

        let include_reasons = p.get_include_reasons();
        let files_without_include_reason: Vec<String> =
            p.get_source_files().iter().filter(|f| !include_reasons.contains_key(f.path())).map(|f| f.path().to_string()).collect();
        let mut file_not_in_program_with_include_reason: Vec<String> = include_reasons
            .keys()
            .filter(|path| p.get_source_file_by_path(path).is_none() && !p.is_missing_path(path))
            .map(|path| path.to_string())
            .collect();
        file_not_in_program_with_include_reason.sort();
        if !files_without_include_reason.is_empty() || !file_not_in_program_with_include_reason.is_empty() {
            let mut b = self.program_include_baselines.lock().unwrap();
            self.write_header_to_baseline(&mut b, &program);
            b.push_str("!!! Expected all files to have include reasons\nfilesWithoutIncludeReason::\n");
            for file in &files_without_include_reason {
                b.push_str("  ");
                b.push_str(file);
                b.push('\n');
            }
            b.push_str("filesNotInProgramWithIncludeReason::\n");
            for file in &file_not_in_program_with_include_reason {
                b.push_str("  ");
                b.push_str(file);
                b.push('\n');
            }
        }
    }

    // sys.go GetTrace
    fn get_trace(&'static self, w: SyncWriter, is_sys_writer: bool) -> Box<tsrs_compiler::TraceFn> {
        Box::new(move |msg: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]| {
            w(&format!("{TRACE_START}\n"));
            // With tsc -b building projects in parallel we cannot serialize the package.json lookup trace
            // so trace as if it wasnt cached
            let s = msg.localize(args);
            w(&format!("{}\n", self.sanitize_trace(&s, is_sys_writer)));
            w(&format!("{TRACE_END}\n"));
        })
    }
}
