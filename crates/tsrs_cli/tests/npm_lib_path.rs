// The npm packages ship the default libraries next to the binary, and a binary with a `lib.d.ts` beside it reads them
// from there, like the npm (noembed) tsgo: `--listFilesOnly` names real files rather than `bundled:///libs/...`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-npm-lib-path-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("project")).unwrap();
    std::fs::write(dir.join("project/tsconfig.json"), r#"{ "compilerOptions": { "target": "es2020", "lib": ["es2020"] }, "files": ["a.ts"] }"#).unwrap();
    std::fs::write(dir.join("project/a.ts"), "export const a = 1;\n").unwrap();
    dir
}

// The test binary under `bin`, linked rather than copied where the file system allows it (it is large).
fn install_binary(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("tsrs");
    if std::fs::hard_link(env!("CARGO_BIN_EXE_tsrs"), &exe).is_err() {
        std::fs::copy(env!("CARGO_BIN_EXE_tsrs"), &exe).unwrap();
    }
    exe
}

fn list_files(exe: &Path, dir: &Path) -> Vec<String> {
    let out = Command::new(exe).args(["-p", "tsconfig.json", "--listFilesOnly"]).current_dir(dir.join("project")).env_remove("TSRS_LIB_PATH").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    String::from_utf8(out.stdout).unwrap().lines().map(str::to_string).collect()
}

#[test]
fn libraries_next_to_the_binary_are_listed_by_their_paths() {
    let dir = scratch("beside");
    let exe = install_binary(&dir);
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../tsrs_vfs/libs")).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, dir.join("bin").join(path.file_name().unwrap())).unwrap();
    }
    let bin = dir.join("bin").to_string_lossy().replace('\\', "/");
    let files = list_files(&exe, &dir);
    assert!(files.contains(&format!("{bin}/lib.es5.d.ts")), "{files:?}");
    assert!(!files.iter().any(|f| f.starts_with("bundled:")), "{files:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_libraries_beside_it_the_binary_uses_the_embedded_ones() {
    let dir = scratch("embedded");
    let exe = install_binary(&dir);
    let files = list_files(&exe, &dir);
    assert!(files.contains(&"bundled:///libs/lib.es5.d.ts".to_string()), "{files:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
