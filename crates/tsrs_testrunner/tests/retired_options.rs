use std::process::Command;

#[test]
fn retired_port_options_fail_before_running_tests() {
    for command in ["run", "list", "show", "materialize", "__worker"] {
        for args in [
            vec!["--oracle", "missing.jsonl"],
            vec!["--options", "missing.json"],
            vec!["--syntax-only"],
            vec!["--oracle=missing.jsonl"],
            vec!["--options=missing.json"],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_tsrs-test")).arg(command).args(&args).output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(2), "{command} {args:?}: {stderr}");
            assert!(stderr.contains("removed porting option"), "{command} {args:?}: {stderr}");
            assert!(output.stdout.is_empty(), "{command} {args:?}: ran instead of rejecting the option");
        }
    }
}
