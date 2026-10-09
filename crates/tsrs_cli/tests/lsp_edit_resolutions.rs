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
    let dir = std::env::temp_dir().join(format!("tsrs-lsp-edit-resolutions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tsconfig.json"), r#"{ "compilerOptions": { "strict": true, "module": "esnext", "moduleResolution": "bundler", "noEmit": true }, "files": ["a.ts"] }"#).unwrap();
    std::fs::write(dir.join("a.ts"), A_TS).unwrap();
    std::fs::write(dir.join("b.ts"), B_TS).unwrap();
    let root = format!("file://{}", dir.display());
    let a = format!("{root}/a.ts");

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
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{a}","languageId":"typescript","version":1,"text":{}}}}}}}"#,
        json_string(A_TS)
    ));
    send(format!(r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/diagnostic","params":{{"textDocument":{{"uri":"{a}"}}}}}}"#));
    let before = response(&mut stdout, 2, &mut send);
    assert!(!before.contains("\"code\":"), "unexpected diagnostics before the edit: {before}");

    // Insert an undeclared identifier into main's body; the import is unchanged, so the program is reused.
    send(format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{a}","version":2}},"contentChanges":[{{"range":{{"start":{{"line":3,"character":0}},"end":{{"line":3,"character":0}}}},"text":"\tc\n"}}]}}}}"#
    ));
    send(format!(r#"{{"jsonrpc":"2.0","id":3,"method":"textDocument/diagnostic","params":{{"textDocument":{{"uri":"{a}"}}}}}}"#));
    let after = response(&mut stdout, 3, &mut send);

    send(r#"{"jsonrpc":"2.0","id":4,"method":"shutdown"}"#.to_string());
    send(r#"{"jsonrpc":"2.0","method":"exit"}"#.to_string());
    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(after.contains("\"code\":2304"), "the edit's error is missing: {after}");
    assert!(!after.contains("\"code\":2307"), "./b lost its resolution after the edit: {after}");
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
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
