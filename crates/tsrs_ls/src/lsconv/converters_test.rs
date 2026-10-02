// converters_test.go (TestConvertersSourceFileProjectionExpansion is not ported: it builds content-mapped files,
// and content mappers are out of scope).

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use tsrs_core::json::Value;
use tsrs_core::TextPos;
use tsrs_lsproto as lsproto;

use super::*;
use crate::spanmap::{Feature, SpanMap};

// converters_test.go:19
#[test]
fn test_document_uri_to_file_name() {
    let tests: &[(&str, &str)] = &[
        ("file:///path/to/file.ts", "/path/to/file.ts"),
        ("file://server/share/file.ts", "//server/share/file.ts"),
        ("file:///d%3A/work/tsgo932/lib/utils.ts", "d:/work/tsgo932/lib/utils.ts"),
        ("file:///D%3A/work/tsgo932/lib/utils.ts", "D:/work/tsgo932/lib/utils.ts"),
        ("file:///d%3A/work/tsgo932/app/%28test%29/comp/comp-test.tsx", "d:/work/tsgo932/app/(test)/comp/comp-test.tsx"),
        ("file:///path/to/file.ts#section", "/path/to/file.ts"),
        ("file:///c:/test/me", "c:/test/me"),
        ("file://shares/files/c%23/p.cs", "//shares/files/c#/p.cs"),
        (
            "file:///c:/Source/Z%C3%BCrich%20or%20Zurich%20(%CB%88zj%CA%8A%C9%99r%C9%AAk,/Code/resources/app/plugins/c%23/plugin.json",
            "c:/Source/Zürich or Zurich (ˈzjʊərɪk,/Code/resources/app/plugins/c#/plugin.json",
        ),
        ("file:///c:/test %25/path", "c:/test %/path"),
        // {"file:?q", "/"},
        ("file:///_:/path", "/_:/path"),
        ("file:///users/me/c%23-projects/", "/users/me/c#-projects/"),
        ("file://localhost/c%24/GitDevelopment/express", "//localhost/c$/GitDevelopment/express"),
        ("file:///c%3A/test%20with%20%2525/c%23code", "c:/test with %25/c#code"),
        ("untitled:Untitled-1", "^/untitled/ts-nul-authority/Untitled-1"),
        ("untitled:Untitled-1#fragment", "^/untitled/ts-nul-authority/Untitled-1#fragment"),
        ("untitled:c:/Users/jrieken/Code/abc.txt", "^/untitled/ts-nul-authority/c:/Users/jrieken/Code/abc.txt"),
        ("untitled:C:/Users/jrieken/Code/abc.txt", "^/untitled/ts-nul-authority/C:/Users/jrieken/Code/abc.txt"),
        ("untitled://wsl%2Bubuntu/home/jabaile/work/TypeScript/newfile.ts", "^/untitled/wsl%2Bubuntu/home/jabaile/work/TypeScript/newfile.ts"),
    ];
    for &(uri, file_name) in tests {
        assert_eq!(lsproto::DocumentUri(uri.to_string()).file_name(), file_name, "{uri}");
    }
}

// converters_test.go:56
#[test]
fn test_file_name_to_document_uri() {
    let tests: &[(&str, &str)] = &[
        ("/path/to/file.ts", "file:///path/to/file.ts"),
        ("//server/share/file.ts", "file://server/share/file.ts"),
        ("d:/work/tsgo932/lib/utils.ts", "file:///d%3A/work/tsgo932/lib/utils.ts"),
        ("D:/work/tsgo932/lib/utils.ts", "file:///d%3A/work/tsgo932/lib/utils.ts"),
        ("d:/work/tsgo932/app/(test)/comp/comp-test.tsx", "file:///d%3A/work/tsgo932/app/%28test%29/comp/comp-test.tsx"),
        ("/path/to/file.ts", "file:///path/to/file.ts"),
        ("c:/test/me", "file:///c%3A/test/me"),
        ("//shares/files/c#/p.cs", "file://shares/files/c%23/p.cs"),
        (
            "c:/Source/Zürich or Zurich (ˈzjʊərɪk,/Code/resources/app/plugins/c#/plugin.json",
            "file:///c%3A/Source/Z%C3%BCrich%20or%20Zurich%20%28%CB%88zj%CA%8A%C9%99r%C9%AAk%2C/Code/resources/app/plugins/c%23/plugin.json",
        ),
        ("c:/test %/path", "file:///c%3A/test%20%25/path"),
        ("/", "file:///"),
        ("/_:/path", "file:///_%3A/path"),
        ("/users/me/c#-projects/", "file:///users/me/c%23-projects/"),
        ("//localhost/c$/GitDevelopment/express", "file://localhost/c%24/GitDevelopment/express"),
        ("c:/test with %25/c#code", "file:///c%3A/test%20with%20%2525/c%23code"),
        ("^/untitled/ts-nul-authority/Untitled-1", "untitled:Untitled-1"),
        ("^/untitled/ts-nul-authority/c:/Users/jrieken/Code/abc.txt", "untitled:c:/Users/jrieken/Code/abc.txt"),
        ("^/untitled/ts-nul-authority///wsl%2Bubuntu/home/jabaile/work/TypeScript/newfile.ts", "untitled://wsl%2Bubuntu/home/jabaile/work/TypeScript/newfile.ts"),
    ];
    for &(file_name, uri) in tests {
        assert_eq!(file_name_to_document_uri(file_name).0, uri, "{file_name}");
    }
}

// converters_test.go:93
#[derive(Clone)]
struct TestScript {
    name: String,
    text: String,
    original_text: String,
}

impl Script for TestScript {
    fn file_name(&self) -> &str {
        &self.name
    }
    fn original_file_name(&self) -> &str {
        &self.name
    }
    fn text(&self) -> &str {
        &self.text
    }
    fn original_text(&self) -> &str {
        if !self.original_text.is_empty() {
            return &self.original_text;
        }
        &self.text
    }
    fn span_map(&self) -> Option<&SpanMap> {
        None
    }
}

// converters_test.go:111
fn new_test_converters(text: &str) -> (Arc<Converters>, TestScript) {
    let script = TestScript { name: "test.ts".to_string(), text: text.to_string(), original_text: String::new() };
    let line_map = compute_lsp_line_starts(text);
    let conv = new_converters(lsproto::PositionEncodingKind::UTF16, move |_| Some(line_map.clone()));
    (conv, script)
}

// converters_test.go:155 TestConvertersInvalidUTF8 is not ported: Rust `&str` text is always valid UTF-8 (the vfs
// decodes file contents), so the invalid-byte path of the conversions cannot be reached.

// converters_test.go:220 (jsReferenceScript, verbatim)
const JS_REFERENCE_SCRIPT: &str = r#"
const inChunks = [];
process.stdin.on('data', c => inChunks.push(c));
process.stdin.on('end', () => {
  const buf = Buffer.concat(inChunks);
  let off = 0;
  const readU32 = () => { const v = buf.readUInt32LE(off); off += 4; return v; };
  const n = readU32();
  const buffers = [];
  for (let i = 0; i < n; i++) {
    const len = readU32();
    buffers.push(buf.subarray(off, off + len));
    off += len;
  }

  const decoder = new TextDecoder('utf-8', { fatal: true });
  const out = buffers.map(bytes => {
    const text = decoder.decode(bytes);

    const lineStartsJs = [0];
    for (let i = 0; i < text.length; i++) {
      const c = text.charCodeAt(i);
      if (c === 13) {
        if (i + 1 < text.length && text.charCodeAt(i + 1) === 10) i++;
        lineStartsJs.push(i + 1);
      } else if (c === 10) {
        lineStartsJs.push(i + 1);
      }
    }

    const boundaries = [{ bytePos: 0, jsIdx: 0 }];
    let bytePos = 0, jsIdx = 0;
    while (bytePos < bytes.length) {
      const seq = utf8SeqLen(bytes[bytePos]);
      const cp = text.codePointAt(jsIdx);
      bytePos += seq;
      jsIdx += cp > 0xFFFF ? 2 : 1;
      boundaries.push({ bytePos, jsIdx });
    }

    return boundaries.map(({ bytePos, jsIdx }) => {
      let lo = 0, hi = lineStartsJs.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if (lineStartsJs[mid] <= jsIdx) lo = mid;
        else hi = mid - 1;
      }
      return { bytePos, line: lo, char: jsIdx - lineStartsJs[lo] };
    });
  });

  process.stdout.write(JSON.stringify(out));
});

function utf8SeqLen(b) {
  if (b < 0x80) return 1;
  if ((b & 0xE0) === 0xC0) return 2;
  if ((b & 0xF0) === 0xE0) return 3;
  if ((b & 0xF8) === 0xF0) return 4;
  throw new Error('invalid UTF-8 lead byte 0x' + b.toString(16));
}
"#;

// converters_test.go:301 (returns None when node is not available, like t.Skipf)
fn run_js_reference(texts: &[&str]) -> Option<Vec<Vec<(i64, u32, u32)>>> {
    let mut input: Vec<u8> = Vec::new();
    input.extend_from_slice(&(texts.len() as u32).to_le_bytes());
    for s in texts {
        input.extend_from_slice(&(s.len() as u32).to_le_bytes());
        input.extend_from_slice(s.as_bytes());
    }

    let mut child = match Command::new("node").arg("-e").arg(JS_REFERENCE_SCRIPT).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(err) => {
            eprintln!("node not available: {err}");
            return None;
        }
    };
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "node failed: {}", String::from_utf8_lossy(&output.stderr));

    let value = tsrs_core::json::unmarshal(&String::from_utf8(output.stdout).unwrap()).unwrap();
    let num = |v: Option<&Value>| match v {
        Some(Value::Number(n)) => *n as i64,
        _ => panic!("bad tuple"),
    };
    let Value::Array(cases) = value else { panic!("bad output") };
    Some(
        cases
            .iter()
            .map(|case| {
                let Value::Array(tuples) = case else { panic!("bad case") };
                tuples
                    .iter()
                    .map(|t| {
                        let Value::Object(o) = t else { panic!("bad tuple") };
                        (num(o.get("bytePos")), num(o.get("line")) as u32, num(o.get("char")) as u32)
                    })
                    .collect()
            })
            .collect(),
    )
}

// TestConvertersAgainstJSReference cross-checks the UTF-16 conversions against
// authoritative results computed by Node.js using real UTF-16 string semantics.
// converters_test.go:337
#[test]
fn test_converters_against_js_reference() {
    let cases: &[(&str, &str)] = &[
        ("empty", ""),
        ("ascii", "hello\nworld"),
        ("ascii_crlf", "hello\r\nworld\r\n!"),
        ("ascii_cr_only", "a\rb\rc"),
        ("trailing_newline", "abc\n"),
        ("bmp_em_dash", "ab\u{2014}cd\nef"),
        ("bmp_multi", "α\nβ\nγδε\nzz"),
        ("supplementary_emoji", "x\u{1F600}y\nz"), // 😀 is 4 UTF-8 bytes, 2 UTF-16 units
        ("supplementary_at_lineend", "ab\u{1F600}\ncd\u{1F60A}"),
        ("supplementary_only", "\u{1F600}\u{1F601}\u{1F602}"),
        ("mixed", "α — \u{1F600}\r\nβ\nγ\r"),
        ("long_mixed_ws", "  \tαβ\n\t\u{1F600}  end\n"),
        ("zwj_emoji", "\u{1F468}\u{200D}\u{1F4BB}\nnext"),
        ("only_newlines", "\n\n\r\n\r"),
    ];

    let texts: Vec<&str> = cases.iter().map(|c| c.1).collect();
    let Some(refs) = run_js_reference(&texts) else { return };
    assert_eq!(refs.len(), cases.len());

    for (i, &(name, text)) in cases.iter().enumerate() {
        let (conv, script) = new_test_converters(text);
        for &(byte_pos, line, char) in &refs[i] {
            let byte_pos = byte_pos as TextPos;
            let expected_lc = lsproto::Position { line, character: char };

            let (got_lc, _) = conv.to_lsp_position(&script, byte_pos);
            assert_eq!(got_lc, expected_lc, "{name}: PositionToLineAndCharacter({byte_pos}) mismatch in {text:?}");

            let positions = conv.from_lsp_position(script.clone(), expected_lc, Feature::All);
            assert_eq!(positions.len(), 1);
            assert_eq!(positions[0].position, byte_pos, "{name}: LineAndCharacterToPosition({line},{char}) mismatch in {text:?}");
        }
    }
}
