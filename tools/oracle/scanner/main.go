// tsrs-oracle-scanner: token-stream oracle for the Rust scanner port.
//
// Usage:
//   tsrs-oracle-scanner dump FILE        print the token streams of FILE
//   tsrs-oracle-scanner hash < filelist  print "hash path" for every file named on stdin
//
// For each file three passes are made (see pass()):
//   1: plain Scan() loop, skipTrivia=true (the parser's mode)
//   2: plain Scan() loop, skipTrivia=false (trivia tokens)
//   3: Scan() loop with parser-like rescans (regex with reportErrors, template continuation, '>' combos)
//   4: as 3, with target ES5 (regex feature availability errors)
// Every token prints "kind pos end flags value"; every scanner error prints "E code start length args".
// Keep this file in sync with crates/tsrs_scanner/examples/scanner_oracle.rs.
package main

import (
	"bufio"
	"hash/fnv"
	"fmt"
	"os"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/diagnostics"
	"github.com/microsoft/TypeScript/tsc/internal/scanner"
)

func escape(sb *strings.Builder, s string) {
	for i := 0; i < len(s); i++ {
		b := s[i]
		if b >= 0x20 && b < 0x7f && b != '\\' {
			sb.WriteByte(b)
		} else {
			fmt.Fprintf(sb, "\\x%02x", b)
		}
	}
}

func isRegexContext(prev ast.Kind) bool {
	switch prev {
	case ast.KindUnknown, ast.KindOpenParenToken, ast.KindCommaToken, ast.KindEqualsToken, ast.KindColonToken,
		ast.KindOpenBracketToken, ast.KindExclamationToken, ast.KindAmpersandAmpersandToken, ast.KindBarBarToken,
		ast.KindQuestionToken, ast.KindOpenBraceToken, ast.KindCloseBraceToken, ast.KindSemicolonToken,
		ast.KindReturnKeyword, ast.KindTypeOfKeyword, ast.KindEqualsEqualsToken, ast.KindEqualsEqualsEqualsToken,
		ast.KindExclamationEqualsToken, ast.KindExclamationEqualsEqualsToken, ast.KindPlusToken, ast.KindMinusToken,
		ast.KindEqualsGreaterThanToken, ast.KindCaseKeyword, ast.KindQuestionQuestionToken:
		return true
	}
	return false
}

func pass(sb *strings.Builder, text string, variant core.LanguageVariant, mode int) {
	fmt.Fprintf(sb, "P %d\n", mode)
	s := scanner.NewScanner()
	s.SetText(text)
	s.SetLanguageVariant(variant)
	s.SetSkipTrivia(mode != 2)
	if mode == 4 {
		s.SetScriptTarget(core.ScriptTargetES5)
	}
	s.SetOnError(func(diag *diagnostics.Message, start, length int, args ...any) {
		fmt.Fprintf(sb, "E %d %d %d", diag.Code(), start, length)
		for _, a := range args {
			sb.WriteByte(' ')
			escape(sb, fmt.Sprint(a))
		}
		sb.WriteByte('\n')
	})
	prev := ast.KindUnknown
	// Template/brace stack for mode 3: true = template substitution, false = plain brace.
	var stack []bool
	for {
		tok := s.Scan()
		if mode >= 3 {
			switch tok {
			case ast.KindSlashToken, ast.KindSlashEqualsToken:
				if isRegexContext(prev) {
					tok = s.ReScanSlashToken(true)
				}
			case ast.KindGreaterThanToken:
				tok = s.ReScanGreaterThanToken()
			case ast.KindTemplateHead:
				stack = append(stack, true)
			case ast.KindOpenBraceToken:
				stack = append(stack, false)
			case ast.KindCloseBraceToken:
				if len(stack) > 0 {
					top := stack[len(stack)-1]
					stack = stack[:len(stack)-1]
					if top {
						tok = s.ReScanTemplateToken(false)
						if tok == ast.KindTemplateMiddle {
							stack = append(stack, true)
						}
					}
				}
			}
		}
		fmt.Fprintf(sb, "%d %d %d %d ", int(tok), s.TokenStart(), s.TokenEnd(), int(s.TokenFlags()))
		escape(sb, s.TokenValue())
		sb.WriteByte('\n')
		if tok == ast.KindEndOfFile {
			break
		}
		if tok != ast.KindWhitespaceTrivia && tok != ast.KindNewLineTrivia && tok != ast.KindSingleLineCommentTrivia &&
			tok != ast.KindMultiLineCommentTrivia && tok != ast.KindConflictMarkerTrivia {
			prev = tok
		}
	}
	for _, d := range s.CommentDirectives() {
		fmt.Fprintf(sb, "D %d %d %d\n", int(d.Kind), d.Loc.Pos(), d.Loc.End())
	}
}

func dump(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	text := string(data)
	variant := core.LanguageVariantStandard
	if strings.HasSuffix(path, ".tsx") || strings.HasSuffix(path, ".jsx") {
		variant = core.LanguageVariantJSX
	}
	var sb strings.Builder
	for mode := 1; mode <= 4; mode++ {
		pass(&sb, text, variant, mode)
	}
	return sb.String(), nil
}

func main() {
	switch os.Args[1] {
	case "dump":
		out, err := dump(os.Args[2])
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Stdout.WriteString(out)
	case "hash":
		in := bufio.NewScanner(os.Stdin)
		w := bufio.NewWriter(os.Stdout)
		defer w.Flush()
		for in.Scan() {
			path := in.Text()
			out, err := dump(path)
			if err != nil {
				fmt.Fprintln(os.Stderr, err)
				continue
			}
			h := fnv.New64a()
			h.Write([]byte(out))
			fmt.Fprintf(w, "%016x %s\n", h.Sum64(), path)
		}
	}
}
