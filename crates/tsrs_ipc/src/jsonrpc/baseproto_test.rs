// Go has no jsonrpc tests of its own; lsp/lsproto/baseproto_test.go tests jsonrpc.Reader and jsonrpc.Writer through
// lsproto's thin BaseReader / BaseWriter wrappers, and is ported here against them directly. The cases after it
// pin the header parsing whose error text reaches users (a mapper that prints to stdout fails its connection with
// it); their expected values are the Go implementation's output.

use std::io::{self, Read, Write};

use crate::jsonrpc::{new_reader, new_writer};
use crate::ErrorTag;

// baseproto_test.go:12
#[test]
fn test_base_reader() {
    struct Test {
        name: &'static str,
        input: &'static [u8],
        value: Option<&'static [u8]>,
        err: &'static str,
    }
    let tests = [
        Test { name: "empty", input: b"Content-Length: 0\r\n\r\n", value: None, err: "jsonrpc: no content length" },
        Test { name: "early end", input: b"oops", value: None, err: "EOF" },
        Test {
            name: "negative length",
            input: b"Content-Length: -1\r\n\r\n",
            value: None,
            err: "jsonrpc: invalid content length: negative value -1",
        },
        Test { name: "invalid content", input: b"Content-Length: 1\r\n\r\n{", value: Some(b"{"), err: "" },
        Test { name: "valid content", input: b"Content-Length: 2\r\n\r\n{}", value: Some(b"{}"), err: "" },
        Test { name: "extra header values", input: b"Content-Length: 2\r\nExtra: 1\r\n\r\n{}", value: Some(b"{}"), err: "" },
        Test {
            name: "too long content length",
            input: b"Content-Length: 100\r\n\r\n{}",
            value: None,
            err: "jsonrpc: read content: unexpected EOF",
        },
        Test {
            name: "missing content length",
            input: b"Content-Length: \r\n\r\n{}",
            value: None,
            err: "jsonrpc: invalid content length: parse error: strconv.ParseInt: parsing \"\": invalid syntax",
        },
        Test { name: "invalid header", input: b"Nope\r\n\r\n{}", value: None, err: "jsonrpc: invalid header: \"Nope\\r\\n\"" },
    ];

    for tt in tests {
        let mut r = new_reader(tt.input);
        let out = r.read();
        if !tt.err.is_empty() {
            assert_eq!(out.as_ref().err().map(ToString::to_string).as_deref(), Some(tt.err), "{}", tt.name);
        }
        assert_eq!(out.ok().as_deref(), tt.value, "{}", tt.name);
    }
}

// baseproto_test.go:81
#[test]
fn test_base_reader_multiple_reads() {
    let data = b"Content-Length: 4\r\n\r\n1234Content-Length: 2\r\n\r\n{}";
    let mut r = new_reader(&data[..]);

    let v1 = r.read().unwrap();
    assert_eq!(v1, b"1234");

    let v2 = r.read().unwrap();
    assert_eq!(v2, b"{}");

    let err = r.read().unwrap_err();
    assert_eq!(err.to_string(), "EOF");
    // Run ends cleanly on it.
    assert!(err.is(ErrorTag::EOF));
}

// baseproto_test.go:108
#[test]
fn test_base_writer() {
    let tests: [(&str, &[u8], &[u8]); 2] = [
        ("empty", b"{}", b"Content-Length: 2\r\n\r\n{}"),
        ("bigger object", b"{\"key\":\"value\"}", b"Content-Length: 15\r\n\r\n{\"key\":\"value\"}"),
    ];

    for (name, value, input) in tests {
        let mut b = Vec::new();
        new_writer(&mut b).write(value).unwrap();
        assert_eq!(b, input, "{name}");
    }
}

// baseproto_test.go:139
#[test]
fn test_base_writer_write_error() {
    struct errorWriter;

    impl Write for errorWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("test error"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut w = new_writer(errorWriter);
    let err = w.write(b"{}").unwrap_err();
    assert_eq!(err.to_string(), "test error");
}

// Regression: a write after a failed one must fail with the same error instead of sending a frame after a
// truncated one (bufio.Writer's sticky error).
#[test]
fn writer_fails_every_write_after_the_first_failure() {
    struct failOnce {
        failed: bool,
        written: Vec<u8>,
    }

    impl Write for failOnce {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if !self.failed {
                self.failed = true;
                return Err(io::Error::other("transient"));
            }
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut sink = failOnce { failed: false, written: Vec::new() };
    let mut w = new_writer(&mut sink);
    assert_eq!(w.write(b"{\"a\":1}").unwrap_err().to_string(), "transient");
    assert_eq!(w.write(b"{}").unwrap_err().to_string(), "transient");
    drop(w);
    assert_eq!(sink.written, b"");
}

// Header handling: Go's TrimSpace (Unicode white space), ParseInt (sign, range), the last Content-Length winning,
// unknown and differently cased headers ignored, a header line ending in a bare "\n", and %q in errors.
#[test]
fn reader_matches_go_header_parsing() {
    let ok: [&[u8]; 6] = [
        b"Content-Length: 5\x0b\r\n\r\nhello",
        "Content-Length: \u{a0}5\r\n\r\nhello".as_bytes(),
        b"Content-Length: +5\r\n\r\nhello",
        b"Content-Length: 9\r\ncontent-length: 5\r\nContent-Length: 5\r\n\r\nhello",
        b"Content-Length:5\n\r\nhello",
        b"X-Other: 1\r\nContent-Length: 5\r\n\r\nhello",
    ];
    for input in ok {
        assert_eq!(new_reader(input).read().unwrap(), b"hello", "{}", String::from_utf8_lossy(input));
    }

    let errors: [(&[u8], &str); 10] = [
        (
            b"Content-Length: 5x\r\n\r\nhello",
            "jsonrpc: invalid content length: parse error: strconv.ParseInt: parsing \"5x\": invalid syntax",
        ),
        (
            b"Content-Length: 99999999999999999999\r\n\r\n",
            "jsonrpc: invalid content length: parse error: strconv.ParseInt: parsing \"99999999999999999999\": value out of range",
        ),
        (
            b"Content-Length: 1 2\r\n\r\n",
            "jsonrpc: invalid content length: parse error: strconv.ParseInt: parsing \"1 2\": invalid syntax",
        ),
        (
            "Content-Length: \u{663}\r\n\r\n".as_bytes(),
            "jsonrpc: invalid content length: parse error: strconv.ParseInt: parsing \"\u{663}\": invalid syntax",
        ),
        (
            b"Content-Length: \xff5\r\n\r\n",
            "jsonrpc: invalid content length: parse error: strconv.ParseInt: parsing \"\\xff5\": invalid syntax",
        ),
        (b"Content-Length: -0\r\n\r\n", "jsonrpc: no content length"),
        (b"Content-Length: 5\r\n\r\n", "jsonrpc: read content: EOF"),
        (b"Debugger attached.\n", "jsonrpc: invalid header: \"Debugger attached.\\n\""),
        (b"\n", "jsonrpc: invalid header: \"\\n\""),
        (
            "Bad\x01\x7f\u{a0}\u{200b}\u{2028} \"q\" \\ \t\x07\x08\x0c\x0b\u{1f600}\u{ad}\u{feff}\r\n".as_bytes(),
            "jsonrpc: invalid header: \"Bad\\x01\\x7f\\u00a0\\u200b\\u2028 \\\"q\\\" \\\\ \\t\\a\\b\\f\\v\u{1f600}\\u00ad\\ufeff\\r\\n\"",
        ),
    ];
    for (input, err) in errors {
        assert_eq!(new_reader(input).read().unwrap_err().to_string(), err, "{}", String::from_utf8_lossy(input));
    }
    assert_eq!(
        new_reader(&b"Bad\xff\r\n"[..]).read().unwrap_err().to_string(),
        "jsonrpc: invalid header: \"Bad\\xff\\r\\n\""
    );
}

// A body cut short by EOF: io.ReadFull's io.EOF (no byte read) still tests true for EOF, so Run treats a stream that
// ends right after a header as closed rather than broken, as Go does.
#[test]
fn reader_reports_a_missing_body_as_eof() {
    let err = new_reader(&b"Content-Length: 5\r\n\r\n"[..]).read().unwrap_err();
    assert!(err.is(ErrorTag::EOF));
    let err = new_reader(&b"Content-Length: 5\r\n\r\nhel"[..]).read().unwrap_err();
    assert_eq!(err.to_string(), "jsonrpc: read content: unexpected EOF");
    assert!(!err.is(ErrorTag::EOF));
}

// A declared length far beyond what arrives must not be allocated up front.
#[test]
fn reader_does_not_trust_a_huge_content_length() {
    let err = new_reader(&b"Content-Length: 1000000000000\r\n\r\n{}"[..]).read().unwrap_err();
    assert_eq!(err.to_string(), "jsonrpc: read content: unexpected EOF");
}

// Frames split across reads, as from a pipe.
#[test]
fn reader_reassembles_frames_delivered_a_byte_at_a_time() {
    struct trickle(std::collections::VecDeque<u8>);

    impl Read for trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match (buf.first_mut(), self.0.pop_front()) {
                (Some(slot), Some(byte)) => {
                    *slot = byte;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    let data = "Content-Length: 6\r\n\r\n\"\u{e9}\u{e9}\"Content-Length: 2\r\n\r\n{}".as_bytes();
    let mut r = new_reader(trickle(data.iter().copied().collect()));
    assert_eq!(r.read().unwrap(), "\"\u{e9}\u{e9}\"".as_bytes());
    assert_eq!(r.read().unwrap(), b"{}");
    assert!(r.read().unwrap_err().is(ErrorTag::EOF));
}
