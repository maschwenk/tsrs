// The checker cost cache's default file (execute.rs `default_checker_cost_cache`, notes/perf-cost-cache-default.md):
// a project with a package.json and a node_modules gets node_modules/.cache/tsrs/checker-costs-<hash>.txt, the second
// run reads it and prints the same; `--noCheckerCostCache`, a named assignment and a project without node_modules
// write nothing; TSRS_CACHE_DIR moves the file.

use std::path::{Path, PathBuf};
use std::process::Command;

fn project(name: &str, node_modules: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-cost-cache-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("package.json"), "{}\n").unwrap();
    std::fs::write(dir.join("tsconfig.json"), r#"{ "compilerOptions": { "strict": true, "noEmit": true }, "include": ["src"] }"#).unwrap();
    for i in 0..6 {
        std::fs::write(dir.join(format!("src/f{i}.ts")), format!("export const v{i}: number = {i};\nexport const bad{i}: string = v{i};\n")).unwrap();
    }
    if node_modules {
        std::fs::create_dir(dir.join("node_modules")).unwrap();
    }
    dir
}

fn run(dir: &Path, args: &[&str], cache_dir: Option<&Path>) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.current_dir(dir).args(["-p", ".", "--pretty", "false", "--checkers", "2"]).args(args);
    for var in ["TSRS_CHECKER_ASSIGNMENT", "TSRS_CHECKER_COST_CACHE", "TSRS_CACHE_DIR", "TSRS_FILE_TIMES", "TSRS_ASSIGNMENT_STATS"] {
        cmd.env_remove(var);
    }
    if let Some(cache_dir) = cache_dir {
        cmd.env("TSRS_CACHE_DIR", cache_dir);
    }
    let out = cmd.output().expect("run tsrs");
    assert_eq!(out.status.code(), Some(2), "{}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn cache_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.map(|e| e.unwrap().path()).collect()).unwrap_or_default()
}

#[test]
fn default_cost_cache_lives_in_node_modules_cache() {
    let dir = project("default", true);
    let first = run(&dir, &[], None);
    assert_eq!(first.matches("error TS2322").count(), 6, "{first}");
    let files = cache_files(&dir.join("node_modules/.cache/tsrs"));
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].extension().and_then(|e| e.to_str()), Some("txt"), "{}", files[0].display());
    assert!(files[0].file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("checker-costs-")), "{}", files[0].display());
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert!(text.starts_with("# tsrs checker cost cache v2 checkers=2\n"), "{text}");
    // One line per checked file: the six sources and the default library files (no skipLibCheck).
    assert_eq!(text.lines().filter(|l| l.contains("/src/f")).count(), 6, "{text}");
    // The second run reads the cache; the output does not depend on it.
    assert_eq!(run(&dir, &[], None), first);
    assert_eq!(cache_files(&dir.join("node_modules/.cache/tsrs")).len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_default_cost_cache_when_opted_out_or_without_node_modules() {
    let dir = project("off", true);
    let expected = run(&dir, &["--noCheckerCostCache"], None);
    assert!(!dir.join("node_modules/.cache").exists());
    // A named assignment is a function of the program alone: no default cache.
    assert_eq!(run(&dir, &["--checkerAssignment", "locality"], None), expected);
    assert!(!dir.join("node_modules/.cache").exists());
    std::fs::remove_dir_all(&dir).unwrap();

    let dir = project("bare", false);
    // Paths print relative to the project directory, so the two projects print the same.
    assert_eq!(run(&dir, &[], None), expected);
    assert!(!dir.join("node_modules").exists(), "tsrs must not create node_modules");
    let elsewhere = dir.join("elsewhere");
    run(&dir, &[], Some(&elsewhere));
    assert_eq!(cache_files(&elsewhere).len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
