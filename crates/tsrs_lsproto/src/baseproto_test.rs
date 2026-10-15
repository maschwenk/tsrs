use std::io::{Read, Write};

use crate::{new_base_reader, new_base_writer};

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
        let mut r = new_base_reader(tt.input);
        let out = r.read();
        if !tt.err.is_empty() {
            assert_eq!(out.as_ref().err().map(std::string::ToString::to_string).as_deref(), Some(tt.err), "{}", tt.name);
        }
        assert_eq!(out.ok().as_deref(), tt.value, "{}", tt.name);
    }
}

// Go trims the Content-Length value with bytes.TrimSpace, which also trims '\v' and Unicode spaces. A raw 0x85 byte is
// invalid UTF-8, not U+0085, and is not trimmed.
#[test]
fn test_base_reader_content_length_trim_space() {
    for input in [
        &b"Content-Length:\t2\x0b\r\n\r\n{}"[..],
        "Content-Length: 2\u{a0}\r\n\r\n{}".as_bytes(),
        "Content-Length:\u{3000}2\u{85}\r\n\r\n{}".as_bytes(),
        "Content-Length: \u{2028}2 \r\n\r\n{}".as_bytes(),
    ] {
        assert_eq!(new_base_reader(input).read().ok().as_deref(), Some(&b"{}"[..]), "{:?}", String::from_utf8_lossy(input));
    }
    assert!(new_base_reader(&b"Content-Length: 2\x85\r\n\r\n{}"[..]).read().is_err());
    assert!(new_base_reader(&b"Content-Length: 2\x1c\r\n\r\n{}"[..]).read().is_err());
}

// baseproto_test.go:79
#[test]
fn test_base_reader_multiple_reads() {
    let data = b"Content-Length: 4\r\n\r\n1234Content-Length: 2\r\n\r\n{}";
    let mut r = new_base_reader(&data[..]);

    let v1 = r.read().unwrap();
    assert_eq!(v1, b"1234");

    let v2 = r.read().unwrap();
    assert_eq!(v2, b"{}");

    let err = r.read().unwrap_err();
    assert_eq!(err.to_string(), "EOF");
    assert!(err.is_eof());
}

struct ErrorReader;

impl Read for ErrorReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("test error"))
    }
}

#[test]
fn test_base_reader_read_error() {
    let mut r = new_base_reader(ErrorReader);
    assert_eq!(r.read().unwrap_err().to_string(), "jsonrpc: read header: test error");
}

// baseproto_test.go:106
#[test]
fn test_base_writer() {
    let tests: [(&str, &[u8], &[u8]); 2] = [
        ("empty", b"{}", b"Content-Length: 2\r\n\r\n{}"),
        ("bigger object", b"{\"key\":\"value\"}", b"Content-Length: 15\r\n\r\n{\"key\":\"value\"}"),
    ];

    for (name, value, input) in tests {
        let mut b = Vec::new();
        {
            let mut w = new_base_writer(&mut b);
            w.write(value).unwrap();
        }
        assert_eq!(b, input, "{name}");
    }
}

// baseproto_test.go:138
#[test]
fn test_base_writer_write_error() {
    let mut w = new_base_writer(ErrorWriter);
    let err = w.write(b"{}").unwrap_err();
    assert_eq!(err.to_string(), "test error");
}

struct ErrorWriter;

impl Write for ErrorWriter {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("test error"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
