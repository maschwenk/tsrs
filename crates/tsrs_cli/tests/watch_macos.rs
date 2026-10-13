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
}

impl Watch {
    fn start(dir: &Path) -> Watch {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tsrs"))
            .args(["--watch", "-p", ".", "--pretty", "false"])
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
        let mut watch = Watch { child, lines };
        watch.next_cycle();
        watch
    }

    // The error lines of the next cycle.
    fn next_cycle(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(30)).expect("no cycle");
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
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs")).args(["-p", ".", "--pretty", "false", "--noEmit"]).current_dir(dir).output().unwrap();
    let mut errors: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().filter(|l| l.contains("error TS")).map(str::to_string).collect();
    errors.sort();
    errors
}

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-watch-{}-{name}", std::process::id()));
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
