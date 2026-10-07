// testdata/regressions/base-type-cycle-entry: the global interfaces `ProcessEnv` and `Env` extend each other
// (declared in env-a.d.ts, env-b.d.ts and env-c.d.ts, as @types/node, bun-types and expo do on drizzle-orm). Go
// resolves the cycle from whichever member a checker asks about first, and that member drops its base in the cycle:
// read-env.ts enters at `Env` (then `ProcessEnv` inherits `Env`'s optional `NODE_ENV`), spawn.ts at `ProcessEnv` (then
// it inherits `RequiredEnv`'s required one and spawn.ts gets a TS2322). So spawn.ts's error depends on whether
// read-env.ts was checked before it on its checker (tsgo-ref: no error at 1-5 checkers, an error at 8 and 16;
// expected.txt is its output at 8). By default tsrs enters every cycle at the member declared first in program order
// (`ProcessEnv`, which is also what tsgo-ref does with `--skipLibCheck false`); `--checkerAssignment go` keeps Go's
// behaviour (notes/fix-history-dependent-diagnostics.md).

use std::path::Path;
use std::process::Command;

fn case() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/base-type-cycle-entry")
}

fn run(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .current_dir(case())
        .args(["-p", ".", "--pretty", "false"])
        .args(args)
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .env_remove("TSRS_HISTORY")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let expected_code = if stdout.is_empty() { 0 } else { 2 };
    assert_eq!(out.status.code(), Some(expected_code), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    stdout
}

#[test]
fn base_type_cycle_is_entered_at_its_first_declared_member() {
    let expected = std::fs::read_to_string(case().join("expected.txt")).unwrap();
    assert_eq!(run(&["--singleThreaded"]), expected, "--singleThreaded");
    for checkers in ["1", "2", "3", "4"] {
        assert_eq!(run(&["--checkers", checkers]), expected, "--checkers {checkers}");
        for seed in 1..=8 {
            let assignment = format!("random:{seed}");
            assert_eq!(run(&["--checkers", checkers, "--checkerAssignment", &assignment]), expected, "--checkers {checkers} {assignment}");
        }
    }
    // Go's history: one checker checks read-env.ts first and enters the cycle at `Env`, as tsgo-ref does.
    assert_eq!(run(&["--checkers", "1", "--checkerAssignment", "go"]), "");
}
