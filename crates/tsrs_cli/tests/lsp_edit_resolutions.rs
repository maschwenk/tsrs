// After an edit, the language server builds the next program with `Program::reuse_program`, which shares the old
// program's `processedFiles`, module resolutions included. The keys of a file's resolutions are its import
// specifiers; they used to borrow the text of the file version they were resolved from. Once the language server
// frees that file version's region, the reused keys dangle: in a release build the next import lookup reads freed
// (or reused) memory and segfaults; with TSRS_ARENA_POISON=1 (freed memory filled with 0xA5 and kept) the lookup
// misses and `./b` reports TS2307. The keys are now copied (fileloader.rs), as the type reference keys already were.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{ChildStdout, Command, Stdio};

const A_TS: &str = "import { environment } from \"./b\"\n\nexport function main() {\n\treturn environment.name\n}\n";
const B_TS: &str = "export const environment = { name: \"b\" }\n";

#[test]
fn edit_keeps_module_resolutions_of_the_edited_file() {
    let tsconfig = r#"{ "compilerOptions": { "strict": true, "module": "esnext", "moduleResolution": "bundler", "noEmit": true }, "files": ["a.ts"] }"#;
    // Insert an undeclared identifier into main's body; the import is unchanged, so the program is reused.
    let (before, after) = diagnostics_after_edits("resolutions", &[("tsconfig.json", tsconfig), ("a.ts", A_TS), ("b.ts", B_TS)], "a.ts", "typescript", 3, 1);

    assert!(!before.contains("\"code\":"), "unexpected diagnostics before the edit: {before}");
    assert!(after.contains("\"code\":2304"), "the edit's error is missing: {after}");
    assert!(!after.contains("\"code\":2307"), "./b lost its resolution after the edit: {after}");
}

// The file loader's synthetic JSX runtime import has the file version it was made for as its parent. The language
// server frees that version once the next program replaces it, so `reuse_program` must not keep the import there
// (watch mode keeps the full build's files and may): checking the JSX in the edited file walks from the import to
// its parent. Every version cloned from the full build keeps the full build's import, so the third version reads
// the first after the language server freed it: the server crashed there.
const A_TSX: &str = "export function main() {\n\treturn <div />\n}\n";
const JSX_RUNTIME_D_TS: &str = "export namespace JSX {\n\tinterface IntrinsicElements {\n\t\tdiv: {}\n\t}\n\ttype Element = {}\n}\nexport declare function jsx(type: any, props: any): JSX.Element\n";

#[test]
fn edit_of_a_jsx_file_keeps_the_jsx_runtime() {
    let tsconfig = r#"{ "compilerOptions": { "strict": true, "jsx": "react-jsx", "module": "esnext", "moduleResolution": "bundler", "noEmit": true }, "files": ["a.tsx"] }"#;
    let files = [
        ("tsconfig.json", tsconfig),
        ("a.tsx", A_TSX),
        ("node_modules/react/package.json", r#"{ "name": "react", "version": "1.0.0" }"#),
        ("node_modules/react/jsx-runtime.d.ts", JSX_RUNTIME_D_TS),
    ];
    let (before, after) = diagnostics_after_edits("jsx-runtime", &files, "a.tsx", "typescriptreact", 1, 4);

    assert!(!before.contains("\"code\":"), "unexpected diagnostics before the edit: {before}");
    assert!(after.contains("\"code\":2304"), "the edit's error is missing: {after}");
    assert!(!after.contains("\"code\":2875"), "the JSX runtime lost its resolution after the edit: {after}");
}

/// Writes `files` to a temporary project, opens `open` in `tsrs --lsp` (with TSRS_ARENA_POISON=1), `edits` times
/// inserts an undeclared identifier `c` as a new line before line `line` (0-based), and returns the file's
/// diagnostics before the first edit and after the last.
fn diagnostics_after_edits(name: &str, files: &[(&str, &str)], open: &str, language_id: &str, line: u32, edits: u32) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("tsrs-lsp-edit-{name}-{}", std::process::id()));
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let open_text = files.iter().find(|(path, _)| *path == open).unwrap().1;
    let root = format!("file://{}", dir.display());
    let uri = format!("{root}/{open}");

    let mut child = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(["--lsp", "-stdio"])
        .env("TSRS_ARENA_POISON", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run tsrs --lsp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |body: String| {
        write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stdin.flush().unwrap();
    };

    send(format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"processId":null,"rootUri":"{root}","capabilities":{{"textDocument":{{"diagnostic":{{}}}}}}}}}}"#
    ));
    response(&mut stdout, 1, &mut send);
    send(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.to_string());
    send(format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"{language_id}","version":1,"text":{}}}}}}}"#,
        json_string(open_text)
    ));
    send(format!(r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/diagnostic","params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#));
    let before = response(&mut stdout, 2, &mut send);

    let mut after = String::new();
    for edit in 0..edits {
        let (version, id) = (edit + 2, edit + 3);
        send(format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{uri}","version":{version}}},"contentChanges":[{{"range":{{"start":{{"line":{line},"character":0}},"end":{{"line":{line},"character":0}}}},"text":"\tc\n"}}]}}}}"#
        ));
        send(format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/diagnostic","params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#));
        after = response(&mut stdout, id, &mut send);
    }

    send(format!(r#"{{"jsonrpc":"2.0","id":{},"method":"shutdown"}}"#, edits + 3));
    send(r#"{"jsonrpc":"2.0","method":"exit"}"#.to_string());
    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    (before, after)
}

/// Reads messages until the response to `id`, answering the server's own requests (configuration, registrations)
/// with `null`.
fn response(stdout: &mut BufReader<ChildStdout>, id: u32, send: &mut impl FnMut(String)) -> String {
    loop {
        let message = read_message(stdout);
        let request_id = message.find("\"method\":").and(field(&message, "\"id\":"));
        if let Some(request_id) = request_id {
            send(format!(r#"{{"jsonrpc":"2.0","id":{request_id},"result":null}}"#));
            continue;
        }
        if field(&message, "\"id\":").as_deref() == Some(id.to_string().as_str()) {
            return message;
        }
    }
}

fn read_message(stdout: &mut BufReader<ChildStdout>) -> String {
    let mut length = None;
    loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "tsrs --lsp exited (crashed?)");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length: ") {
            length = Some(n.parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("Content-Length")];
    stdout.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}

/// The raw value after `key` up to the next `,` or `}` (numbers and quoted strings without those characters).
fn field(message: &str, key: &str) -> Option<String> {
    let start = message.find(key)? + key.len();
    let end = message[start..].find([',', '}'])? + start;
    Some(message[start..end].to_string())
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    out = s.chars().fold(out, |mut out, c| {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }

        out
    });
    out.push('"');
    out
}
