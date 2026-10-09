// Go's testutil.RecoverAndFail and testutil/baseline (the subset fourslash uses).

use crate::testing::T;

// Signature-compatible form of Go's testutil.RecoverAndFail; `go::run` now returns only `Ok(())`.
pub fn recover_and_fail(t: &T, msg: &str, r: std::thread::Result<()>) {
    if r.is_err() {
        t.fatal(msg);
    }
}

pub mod baseline {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::testing::T;

    // baseline.go:14
    #[derive(Clone, Debug, Default)]
    pub struct Options {
        pub subfolder: String,
        pub skip_diff_with_old: bool,
    }

    // baseline.go:21
    pub const NO_CONTENT: &str = "<no content>";

    // baseline.go:23
    pub fn run(t: &T, file_name: &str, actual: &str, opts: Options) {
        let subfolder = &opts.subfolder;
        let local_path = local_root().join(subfolder).join(file_name);
        let reference_path = reference_root().join(subfolder).join(file_name);
        write_comparison(t, actual, &local_path, &reference_path);
    }

    // baseline.go:42
    fn write_comparison(t: &T, actual_content: &str, local: &Path, reference: &Path) {
        if actual_content.is_empty() {
            panic!("the generated content was \"\". Return 'baseline.NoContent' if no baselining is required.");
        }
        if let Err(err) = fs::create_dir_all(local.parent().unwrap()) {
            t.error(&format!("failed to create directories for the local baseline file {}: {}", local.display(), err));
            return;
        }
        if local.exists() {
            if let Err(err) = fs::remove_file(local) {
                t.error(&format!("failed to remove the local baseline file {}: {}", local.display(), err));
                return;
            }
        }

        let mut expected = NO_CONTENT.to_string();
        let mut found_expected = false;
        if let Ok(content) = fs::read_to_string(reference) {
            expected = content;
            found_expected = true;
        }
        if expected == actual_content && !(actual_content == NO_CONTENT && found_expected) {
            return;
        }
        if actual_content == NO_CONTENT {
            let delete = PathBuf::from(format!("{}.delete", local.display()));
            if let Err(err) = fs::write(&delete, b"") {
                t.error(&format!("failed to write the local baseline file {}: {}", delete.display(), err));
            }
            return;
        }
        if let Err(err) = fs::write(local, actual_content) {
            t.error(&format!("failed to write the local baseline file {}: {}", local.display(), err));
            return;
        }
        if !found_expected {
            t.error(&format!("new baseline created at {}.", local.display()));
            return;
        }
        t.error(&format!("the baseline file {} has changed. (Run `hereby baseline-accept` if the new baseline is correct.)", reference.display()));
    }

    // Go: repo.TestDataPath()/baselines/{local,reference}. The references are the Go checkout's
    // (ts-ref/tsc/testdata); actuals go under target/fourslash-results/local.
    pub fn reference_root() -> PathBuf {
        crate::runner::repo_root().join("ts-ref").join("tsc").join("testdata").join("baselines").join("reference")
    }

    pub fn local_root() -> PathBuf {
        crate::runner::results_dir().join("local")
    }

    // baseline/testmain.go Track: records which baselines a test run produced (used by Go's TestMain to find
    // unused reference files). Not needed by the Rust runner.
    pub fn track() {}
}
