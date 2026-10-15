package main

import (
	"fmt"
	"regexp"
	"strconv"
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

// snake is the port's snake_case rule (acronym runs are one word), with the word replacements of gosig's checker.json
// (JSDoc, NaN, IDs, URLs) applied first; tools/gen-lsproto/generate.mts uses the same rule without them.
var wordReplacer = strings.NewReplacer("JSDoc", "Jsdoc", "NaN", "Nan", "IDs", "Ids", "URLs", "Urls")

func snake(s string) string {
	s = wordReplacer.Replace(s)
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
	// Escaped literals preserve fixture whitespace without putting trailing spaces or conflict markers in Rust source.
	firstLine, _, _ := strings.Cut(s, "\n")
	if strings.HasSuffix(firstLine, " ") || strings.HasSuffix(firstLine, "\t") || strings.HasSuffix(s, " ") || strings.HasSuffix(s, "\t") || strings.HasPrefix(s, "<<<<<<<") || strings.Contains(s, "\n=======") {
		needsEscape = true
	}
	if !needsEscape && (strings.ContainsAny(s, "\n\"\\")) && !strings.Contains(s, "\r") {
		hashes := ""
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

// rustCompressedString renders strings with very long newline runs as a small runtime builder expression. The
// really-large-file fourslash fixture contains more than half a million blank lines; spelling those lines in Rust
// source makes rustc lex and index them on every generated-test rebuild without testing anything extra.
func rustCompressedString(s string) (string, bool) {
	const minRun = 1024
	var b strings.Builder
	start := 0
	found := false
	for i := 0; i < len(s); {
		if s[i] != '\n' {
			i++
			continue
		}
		end := i + 1
		for end < len(s) && s[end] == '\n' {
			end++
		}
		if end-i < minRun {
			i = end
			continue
		}
		if !found {
			fmt.Fprintf(&b, "{ let mut s = String::with_capacity(%s); ", rustCount(len(s)))
			found = true
		}
		if start < i {
			push, ok := rustStringPush(s[start:i])
			if !ok {
				return "", false
			}
			b.WriteString(push)
		}
		fmt.Fprintf(&b, "for _ in 0..%s { s.push('\\n'); } ", rustCount(end-i))
		start = end
		i = end
	}
	if !found {
		return "", false
	}
	if start < len(s) {
		push, ok := rustStringPush(s[start:])
		if !ok {
			return "", false
		}
		b.WriteString(push)
	}
	b.WriteString("s }")
	return b.String(), true
}

func rustCount(n int) string {
	digits := strconv.Itoa(n)
	for i := len(digits) - 3; i > 0; i -= 3 {
		digits = digits[:i] + "_" + digits[i:]
	}
	return digits
}

func rustStringPush(s string) (string, bool) {
	lit, ok := rustStr(s)
	if !ok {
		return "", false
	}
	if utf8.RuneCountInString(s) == 1 {
		r, _ := utf8.DecodeRuneInString(s)
		lit = fmt.Sprintf("'\\u{%x}'", r)
		if r >= 0x20 && r <= 0x7e {
			lit = strconv.QuoteRune(r)
		}
		return "s.push(" + lit + "); ", true
	}
	return "s.push_str(" + lit + "); ", true
}
