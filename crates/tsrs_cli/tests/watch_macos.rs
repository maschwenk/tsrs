// `--watch` on the real file system of a Mac (FSEvents): after each change, the cycle the watch runs must report what
// a fresh build of the same tree reports.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

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
    edits_through_a_symlinked_directory(&["--watch", "-p", "."]);
}

#[test]
fn edits_through_a_symlinked_directory_by_build_watch() {
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
