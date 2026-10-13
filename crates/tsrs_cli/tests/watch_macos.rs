// `--watch` on the real file system of a Mac (FSEvents): after each change, the cycle the watch runs must report what
// a fresh build of the same tree reports.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{RwLock, RwLockReadGuard};
use std::time::Duration;

// Tests share the temporary directory, and a watch's streams see the writes of the others: a test that needs quiet
// takes this exclusively, the others shared.
static EXCLUSIVE: RwLock<()> = RwLock::new(());

fn shared() -> RwLockReadGuard<'static, ()> {
    EXCLUSIVE.read().unwrap_or_else(|e| e.into_inner())
}

struct Watch {
    child: Child,
    lines: Receiver<String>,
    // The watch's own decisions (TS_WATCH_DEBUG), for failure messages.
    log: Vec<String>,
}

impl Watch {
    fn start(dir: &Path) -> Watch {
        Watch::start_with(dir, &["--watch", "-p", "."])
    }

    fn start_with(dir: &Path, args: &[&str]) -> Watch {
        let mut watch = Watch::spawn(dir, args);
        watch.next_cycle();
        watch
    }

    // Does not wait for the first build.
    fn spawn(dir: &Path, args: &[&str]) -> Watch {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tsrs"))
            .args(args)
            .args(["--pretty", "false"])
            .env("TS_WATCH_DEBUG", "1")
            .current_dir(dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (tx, lines) = channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Watch { child, lines, log: Vec::new() }
    }

    // The error lines of the next cycle.
    fn next_cycle(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(15)).unwrap_or_else(|_| panic!("no cycle; watch log:\n{}", self.log.join("\n")));
            if line.starts_with("[watch] ") {
                if !line.contains("watching directory") && !line.contains("resolved ") {
                    self.log.push(line);
                }
                continue;
            }
            if line.contains("File change detected") || line.contains("Starting compilation") {
                errors.clear();
            } else if line.contains("Watching for file changes") {
                errors.sort();
                return errors;
            } else if line.contains("error TS") {
                errors.push(line);
            }
        }
    }
}

impl Watch {
    // Fails if a cycle starts within `quiet` (nothing changed, so nothing should).
    fn assert_idle(&mut self, quiet: Duration) {
        let deadline = std::time::Instant::now() + quiet;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match self.lines.recv_timeout(left) {
                Ok(line) if line.starts_with("[watch] ") => self.log.push(line),
                Ok(line) => assert!(!line.contains("File change detected"), "a cycle started by itself: {line}; watch log:\n{}", self.log.join("\n")),
                Err(_) => return,
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fresh(dir: &Path) -> Vec<String> {
    fresh_with(dir, ".")
}

fn fresh_with(dir: &Path, project: &str) -> Vec<String> {
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs")).args(["-p", project, "--pretty", "false", "--noEmit"]).current_dir(dir).output().unwrap();
    let mut errors: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().filter(|l| l.contains("error TS")).map(str::to_string).collect();
    errors.sort();
    errors
}

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    project_in(&std::env::temp_dir(), name, files)
}

fn project_in(root: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = root.join(format!("tsrs-watch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, content) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    std::fs::canonicalize(&dir).unwrap()
}

const TSCONFIG: &str = r#"{ "compilerOptions": { "strict": true, "noEmit": true, "types": [], "forceConsistentCasingInFileNames": true }, "include": ["src"] }"#;

#[test]
fn a_case_only_rename_reports_what_a_fresh_build_does() {
    let _shared = shared();
    let dir = project("case", &[("tsconfig.json", TSCONFIG), ("src/a.ts", "export const a = 1;\n"), ("src/b.ts", "import { a } from './a';\nexport const b: number = a;\n")]);
    let mut watch = Watch::start(&dir);
    std::thread::sleep(Duration::from_millis(600));

    std::fs::rename(dir.join("src/a.ts"), dir.join("src/A.ts")).unwrap();
    let errors = watch.next_cycle();
    let want = fresh(&dir);
    assert!(want.iter().any(|e| e.contains("TS1149")), "{want:?}");
    assert_eq!(errors, want);

    std::thread::sleep(Duration::from_millis(600));
    std::fs::rename(dir.join("src/A.ts"), dir.join("src/a.ts")).unwrap();
    assert_eq!(watch.next_cycle(), fresh(&dir));
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
}

const ERR: &str = "\nexport const err: number = 'x';\n";

#[test]
fn a_project_under_tmp_sees_edits() {
    let _shared = shared();
    // /tmp is a symlink to /private/tmp, and FSEvents reports /private/tmp paths. The project is named through /tmp
    // (`-p /tmp/...`, from another directory: a process's working directory is always the resolved one).
    let real = project_in(Path::new("/private/tmp"), "tmp", &[("tsconfig.json", TSCONFIG), ("src/a.ts", "export const a = 1;\n")]);
    let logical = Path::new("/tmp").join(real.strip_prefix("/private/tmp").unwrap());
    let config = logical.join("tsconfig.json");
    let mut watch = Watch::start_with(Path::new("/"), &["--watch", "-p", config.to_str().unwrap()]);
    std::thread::sleep(Duration::from_millis(600));

    std::fs::write(logical.join("src/a.ts"), format!("export const a = 1;{ERR}")).unwrap();
    let errors = watch.next_cycle();
    assert_eq!(errors, fresh_with(Path::new("/"), config.to_str().unwrap()));
    assert_eq!(errors.len(), 1, "{errors:?}");
    std::fs::write(real.join("src/b.ts"), "export const b: string = 1;\n").unwrap();
    let errors = watch.next_cycle();
    assert_eq!(errors.len(), 2, "{errors:?}");
    drop(watch);
    let _ = std::fs::remove_dir_all(&real);
}

#[test]
fn edits_through_a_symlinked_directory_by_watch() {
    let _shared = shared();
    edits_through_a_symlinked_directory(&["--watch", "-p", "."]);
}

#[test]
fn edits_through_a_symlinked_directory_by_build_watch() {
    let _shared = shared();
    edits_through_a_symlinked_directory(&["-b", ".", "--watch"]);
}

fn edits_through_a_symlinked_directory(args: &[&str]) {
    // src/shared is a symlink to a directory outside the project; edits made through either path are seen.
    let mode = args[0].trim_start_matches('-');
    let shared = project(&format!("shared-target-{mode}"), &[("s.ts", "export const s = 1;\n")]);
    let dir = project(&format!("symlink-{mode}"), &[("tsconfig.json", TSCONFIG), ("src/a.ts", "import { s } from './shared/s';\nexport const a: number = s;\n")]);
    std::os::unix::fs::symlink(&shared, dir.join("src/shared")).unwrap();
    let mut watch = Watch::start_with(&dir, args);
    std::thread::sleep(Duration::from_millis(600));

    std::fs::write(shared.join("s.ts"), "export const s = 'x';\n").unwrap();
    let errors = watch.next_cycle();
    assert_eq!(errors, fresh(&dir));
    assert_eq!(errors.len(), 1, "{errors:?}");
    std::thread::sleep(Duration::from_millis(600));
    std::fs::write(dir.join("src/shared/s.ts"), "export const s = 1;\n").unwrap();
    assert_eq!(watch.next_cycle(), Vec::<String>::new());
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&shared);
}

#[test]
fn outputs_inside_the_watched_tree_start_no_cycle() {
    let _shared = shared();
    // Everything under the project root is included and watched recursively; the outputs and the build info land
    // there too, and must not start a cycle of their own.
    let config = r#"{ "compilerOptions": { "strict": true, "types": [], "outDir": "dist", "declaration": true, "incremental": true }, "include": ["**/*"] }"#;
    let mut files = vec![("tsconfig.json".to_string(), config.to_string())];
    files.extend((0..50).map(|i| (format!("f{i}.ts"), format!("export const f{i} = {i};\n"))));
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let dir = project("outdir", &files);
    let mut watch = Watch::start(&dir);
    watch.assert_idle(Duration::from_secs(2));
    assert!(dir.join("dist/f0.js").exists() && dir.join("dist/tsconfig.tsbuildinfo").exists());

    for round in 0..3 {
        std::fs::write(dir.join("f0.ts"), format!("export const f0 = {round};\nexport const g{round} = 1;\n")).unwrap();
        assert_eq!(watch.next_cycle(), Vec::<String>::new());
        watch.assert_idle(Duration::from_secs(2));
    }
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
}

// A project whose build is mostly checking: its files are read within the first few percent of the build.
fn slow_to_check_project(name: &str) -> PathBuf {
    let mut files = vec![("tsconfig.json".to_string(), TSCONFIG.to_string()), ("src/a.ts".to_string(), "export const a = 1;\n".to_string())];
    files.extend((0..1500).map(|i| {
        (format!("src/f{i}.ts"), format!("type T{i}<N extends number, A extends any[] = []> = A[\"length\"] extends N ? A : T{i}<N, [...A, 0]>;\nexport const x{i}: T{i}<900>[\"length\"] = 900;\n"))
    }));
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    project(name, &files)
}

fn an_edit_during_the_first_build_is_seen(args: &[&str]) {
    let dir = slow_to_check_project(&format!("during-{}", args[0].trim_start_matches('-')));
    let started = std::time::Instant::now();
    assert!(fresh(&dir).is_empty());
    let build_time = started.elapsed();

    // The edit must land after the first build read a.ts and before its watches exist: when the first report already
    // has the edit's error, it was too early, so try again later in the build.
    for fraction in [0.4, 0.6, 0.8, 0.9] {
        std::fs::write(dir.join("src/a.ts"), "export const a = 1;\n").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let mut watch = Watch::spawn(&dir, args);
        std::thread::sleep(build_time.mul_f64(fraction));
        std::fs::write(dir.join("src/a.ts"), format!("export const a = 1;{ERR}")).unwrap();
        if !watch.next_cycle().is_empty() {
            continue;
        }
        let errors = watch.next_cycle();
        assert_eq!(errors, fresh(&dir));
        assert_eq!(errors.len(), 1, "{errors:?}");
        drop(watch);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    panic!("every edit landed before the first build read the file");
}

#[test]
fn an_edit_during_the_first_build_is_seen_by_watch() {
    let _shared = shared();
    an_edit_during_the_first_build_is_seen(&["--watch", "-p", "."]);
}

#[test]
fn an_edit_during_the_first_build_is_seen_by_build_watch() {
    let _shared = shared();
    an_edit_during_the_first_build_is_seen(&["-b", ".", "--watch"]);
}

#[test]
fn a_metadata_change_starts_no_cycle() {
    let _shared = shared();
    // chmod changes no content and no modification time; FSEvents still reports the file.
    let dir = project("chmod", &[("tsconfig.json", TSCONFIG), ("src/a.ts", "export const a = 1;\n")]);
    let mut watch = Watch::start(&dir);
    watch.assert_idle(Duration::from_millis(600));
    std::fs::set_permissions(dir.join("src/a.ts"), std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    watch.assert_idle(Duration::from_secs(1));
    // An edit after it is still seen.
    std::fs::write(dir.join("src/a.ts"), format!("export const a = 1;{ERR}")).unwrap();
    assert_eq!(watch.next_cycle().len(), 1);
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
}

// The process's interrupt wakeups so far (what Activity Monitor's "Idle Wake Ups" counts).
fn wakeups(pid: u32) -> u64 {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: `info` is a rusage_info_v4, the struct RUSAGE_INFO_V4 fills.
    let r = unsafe { libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) };
    assert_eq!(r, 0, "proc_pid_rusage");
    // SAFETY: filled above.
    unsafe { info.assume_init() }.ri_interrupt_wkups
}

#[test]
fn an_idle_watch_does_not_wake_up() {
    // Alone: the other tests' writes under the same temporary directory reach this watch's streams too.
    let _alone = EXCLUSIVE.write().unwrap_or_else(|e| e.into_inner());
    let dir = project("idle", &[("tsconfig.json", TSCONFIG), ("src/a.ts", "export const a = 1;\n")]);
    let mut watch = Watch::start(&dir);
    watch.assert_idle(Duration::from_millis(500));
    let before = wakeups(watch.child.id());
    watch.assert_idle(Duration::from_secs(3));
    let woke = wakeups(watch.child.id()) - before;
    assert!(woke <= 1, "{woke} wakeups in 3 s of idling");
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_directory_beside_the_project_starts_no_cycle_in_watch() {
    let _shared = shared();
    a_new_directory_beside_the_project_starts_no_cycle(&["--watch", "-p", "."]);
}

#[test]
fn a_new_directory_beside_the_project_starts_no_cycle_in_build_watch() {
    let _shared = shared();
    a_new_directory_beside_the_project_starts_no_cycle(&["-b", ".", "--watch"]);
}

fn a_new_directory_beside_the_project_starts_no_cycle(args: &[&str]) {
    // The project's parent is watched for node_modules lookups; a directory made there that no lookup goes through
    // changes nothing.
    let dir = project(&format!("beside-{}", args[0].trim_start_matches('-')), &[("tsconfig.json", TSCONFIG), ("src/a.ts", "export const a = 1;\n")]);
    let mut watch = Watch::start_with(&dir, args);
    watch.assert_idle(Duration::from_millis(500));
    let beside = dir.with_file_name(format!("{}-sibling", dir.file_name().unwrap().to_str().unwrap()));
    std::fs::create_dir(&beside).unwrap();
    std::fs::write(beside.join("x.txt"), "x").unwrap();
    watch.assert_idle(Duration::from_secs(1));
    // A directory the project's wildcards cover still counts.
    std::fs::create_dir(dir.join("src/sub")).unwrap();
    std::fs::write(dir.join("src/sub/b.ts"), "export const b: number = 'x';\n").unwrap();
    assert_eq!(watch.next_cycle().len(), 1);
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&beside);
}

#[test]
fn a_watch_that_cannot_watch_says_so() {
    let _shared = shared();
    // The kqueue fallback with a descriptor limit it cannot raise: the watch error and the build's report are printed
    // (they were held in the output buffer for good).
    let mut files = vec![("tsconfig.json".to_string(), TSCONFIG.to_string())];
    files.extend((0..200).map(|i| (format!("src/f{i}.ts"), format!("export const f{i} = {i};\n"))));
    let files: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
    let dir = project("nofds", &files);
    let mut command = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    command.args(["--watch", "-p", ".", "--pretty", "false"]).current_dir(&dir).env("TSRS_WATCH_BACKEND", "kqueue").stdout(Stdio::piped());
    // SAFETY: setrlimit is async-signal-safe; the closure touches nothing else.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut command, || {
            let lim = libc::rlimit { rlim_cur: 64, rlim_max: 64 };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lim) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(10)) {
        let done = line.contains("Watching for file changes");
        lines.push(line);
        if done {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(lines.iter().any(|l| l.contains("ulimit -n")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Found 0 errors")), "{lines:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn installing_a_package_resolves_its_import(args: &[&str]) {
    // node_modules does not exist when the watch starts; the import resolves once the package is installed.
    let dir = project(&format!("install-{}", args[0].trim_start_matches('-')), &[("tsconfig.json", TSCONFIG), ("src/a.ts", "import type { N } from 'pkg';\nexport const n: N = 42;\n")]);
    let mut watch = Watch::start_with(&dir, args);
    std::thread::sleep(Duration::from_millis(600));
    let pkg = dir.join("node_modules/pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("index.d.ts"), "export type N = 42;\n").unwrap();
    std::fs::write(pkg.join("package.json"), r#"{ "name": "pkg", "version": "1.0.0", "types": "index.d.ts" }"#).unwrap();
    let mut errors = watch.next_cycle();
    // The files may come in two batches.
    while !errors.is_empty() {
        errors = watch.next_cycle();
    }
    assert_eq!(errors, fresh(&dir));
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn installing_a_package_resolves_its_import_in_watch() {
    let _shared = shared();
    installing_a_package_resolves_its_import(&["--watch", "-p", "."]);
}

#[test]
fn installing_a_package_resolves_its_import_in_build_watch() {
    let _shared = shared();
    installing_a_package_resolves_its_import(&["-b", ".", "--watch"]);
}

fn edits_after_the_first_build_are_seen(args: &[&str]) {
    // Incremental, with outputs (the build info is read and written), and each edit made as soon as the previous cycle
    // reported. (-b --watch once reused a file parsed before an edit: a stat from between cycles was still cached.)
    // The outputs go outside the watched tree (as vscode's ../out), so that no cycle for them runs between the edits.
    let name = format!("edits-{}", args[0].trim_start_matches('-'));
    let out = std::env::temp_dir().join(format!("tsrs-watch-{}-{name}-out", std::process::id()));
    let config = format!(r#"{{ "compilerOptions": {{ "strict": true, "types": [], "incremental": true, "outDir": "{}", "rootDir": "src" }}, "include": ["src"] }}"#, out.display());
    let dir = project(&name, &[("tsconfig.json", &config), ("src/a.ts", "export const a = 1;\n"), ("src/b.ts", "export const b = 1;\n")]);
    let mut watch = Watch::start_with(&dir, args);
    for (file, content, count) in [("src/a.ts", format!("export const a = 1;{ERR}"), 1), ("src/b.ts", format!("export const b = 1;{ERR}"), 2), ("src/a.ts", "export const a = 1;\n".to_string(), 1)] {
        std::fs::write(dir.join(file), content).unwrap();
        let errors = watch.next_cycle();
        assert_eq!(errors.len(), count, "{errors:?}; watch log:\n{}", watch.log.join("\n"));
        assert_eq!(errors, fresh(&dir));
    }
    drop(watch);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn edits_after_the_first_build_are_seen_in_watch() {
    let _shared = shared();
    edits_after_the_first_build_are_seen(&["--watch", "-p", "."]);
}

#[test]
fn edits_after_the_first_build_are_seen_in_build_watch() {
    let _shared = shared();
    edits_after_the_first_build_are_seen(&["-b", ".", "--watch"]);
}
