package main

import (
	"fmt"
	"regexp"
	"strings"
	"unicode/utf8"
)

var rustKeywords = map[string]bool{}

func init() {
	for _, k := range strings.Fields(`as break const continue crate else enum extern false fn for if impl in let loop
		match mod move mut pub ref return self Self static struct super trait true type unsafe use where while
		async await dyn abstract become box do final macro override priv typeof unsized virtual yield try gen`) {
		rustKeywords[k] = true
	}
}

var (
	snakeRe1 = regexp.MustCompile(`([a-z0-9])([A-Z])`)
	snakeRe2 = regexp.MustCompile(`([A-Z]+)([A-Z][a-z])`)
)

// snake is the snake_case rule of the rest of the port (tools/gen-lsproto/generate.mts): acronym runs are one word.
func snake(s string) string {
	s = snakeRe1.ReplaceAllString(s, "${1}_${2}")
	s = snakeRe2.ReplaceAllString(s, "${1}_${2}")
	return strings.ToLower(s)
}

func ident(s string) string {
	n := snake(s)
	if rustKeywords[n] {
		n += "_"
	}
	return n
}

func screaming(s string) string {
	return strings.ToUpper(snake(s))
}

// rustStr renders a Go string value as a Rust string literal (a raw string when that reads better).
func rustStr(s string) (string, bool) {
	if !utf8.ValidString(s) {
		return "", false
	}
	needsEscape := false
	for _, r := range s {
		if r < 0x20 && r != '\n' && r != '\t' || r == 0x7f {
			needsEscape = true
		}
	}
	if !needsEscape && (strings.ContainsAny(s, "\n\"\\")) && !strings.Contains(s, "\r") {
		hashes := "#"
		for strings.Contains(s, "\""+hashes) {
			hashes += "#"
		}
		return "r" + hashes + "\"" + s + "\"" + hashes, true
	}
	var b strings.Builder
	b.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			b.WriteString(`\"`)
		case '\\':
			b.WriteString(`\\`)
		case '\n':
			b.WriteString(`\n`)
		case '\r':
			b.WriteString(`\r`)
		case '\t':
			b.WriteString(`\t`)
		case 0:
			b.WriteString(`\0`)
		default:
			if r < 0x20 || r == 0x7f {
				fmt.Fprintf(&b, `\x%02x`, r)
			} else {
				b.WriteRune(r)
			}
		}
	}
	b.WriteByte('"')
	return b.String(), true
}
