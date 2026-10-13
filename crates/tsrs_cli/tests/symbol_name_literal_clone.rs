// Cached type literals must keep internal-name escaping when cloned for declaration emit. The tuple repeats a
// cached type alongside user DEL text; expected output is pinned tsgo's.

use std::process::Command;

const SOURCE: &str = "export enum E { 0n = 0, b = 1 }
declare const v: E;
function g() {
    if (v !== E.b) return { p: v, u: \"\\u007f\\b\" as const, \"\\u007f\": 1 };
    throw 0;
}
export const pair = [g(), g()] as const;
";

const TSCONFIG: &str = r#"{"compilerOptions":{"strict":true,"declaration":true,"emitDeclarationOnly":true,"outDir":"out","target":"es2020","types":[],"skipLibCheck":true},"files":["a.ts"]}"#;

const ELEMENT: &str = "{\n    p: (typeof E)[\"\\uFFFDmissing\"];\n    u: \"\u{7f}\\b\";\n    \"\u{7f}\": number;\n}";

#[test]
fn cached_type_clone_keeps_internal_symbol_name_escape() {
    let dir = std::env::temp_dir().join(format!("tsrs-symbol-name-literal-clone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ts"), SOURCE).unwrap();
    std::fs::write(dir.join("tsconfig.json"), TSCONFIG).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_tsrs")).current_dir(&dir).args(["-p", ".", "--pretty", "false"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a.ts(1,17): error TS2452: An enum member cannot have a numeric name.\n");
    let dts = std::fs::read_to_string(dir.join("out/a.d.ts")).unwrap();
    let expected = format!("export declare enum E {{\n    0n = 0,\n    b = 1\n}}\nexport declare const pair: readonly [{ELEMENT}, {ELEMENT}];\n");
    assert_eq!(dts, expected);
    let _ = std::fs::remove_dir_all(&dir);
}
