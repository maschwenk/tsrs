// Formats every file listed on stdin (one path per line) with FormatDocument and prints, per file, one line:
//   <path>\t<edits>
// where <edits> is the concatenation of "pos,end,<json-quoted newText>;" for each edit. Used to compare
// tsrs_ls::format against the Go formatter (crates/tsrs_ls/src/format/oracle_test.rs).
//
// Copy to ts-ref/tsc/cmd/tsrs-oracle-format/main.go and run with GOTOOLCHAIN=auto go run ./cmd/tsrs-oracle-format.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/format"
	"github.com/microsoft/TypeScript/tsc/internal/ls/lsutil"
	"github.com/microsoft/TypeScript/tsc/internal/parser"
)

func main() {
	settings := lsutil.GetDefaultFormatCodeSettings()
	if len(os.Args) > 1 && os.Args[1] == "api" {
		settings = lsutil.FormatCodeSettings{
			EditorSettings: lsutil.EditorSettings{
				TabSize:                4,
				IndentSize:             4,
				BaseIndentSize:         4,
				NewLineCharacter:       "\n",
				ConvertTabsToSpaces:    core.TSTrue,
				IndentStyle:            lsutil.IndentStyleSmart,
				TrimTrailingWhitespace: core.TSTrue,
			},
			InsertSpaceBeforeTypeAnnotation: core.TSTrue,
		}
	}
	if len(os.Args) > 1 && (os.Args[1] == "alt" || os.Args[1] == "insert") {
		settings.TabSize = 2
		settings.IndentSize = 2
		settings.BaseIndentSize = 1
		settings.ConvertTabsToSpaces = core.TSFalse
		settings.InsertSpaceAfterCommaDelimiter = core.TSFalse
		settings.InsertSpaceAfterSemicolonInForStatements = core.TSFalse
		settings.InsertSpaceBeforeAndAfterBinaryOperators = core.TSFalse
		settings.InsertSpaceAfterConstructor = core.TSTrue
		settings.InsertSpaceAfterKeywordsInControlFlowStatements = core.TSFalse
		settings.InsertSpaceAfterFunctionKeywordForAnonymousFunctions = core.TSTrue
		settings.InsertSpaceAfterOpeningAndBeforeClosingNonemptyParenthesis = core.TSTrue
		settings.InsertSpaceAfterOpeningAndBeforeClosingNonemptyBrackets = core.TSTrue
		settings.InsertSpaceAfterOpeningAndBeforeClosingNonemptyBraces = core.TSFalse
		settings.InsertSpaceAfterOpeningAndBeforeClosingEmptyBraces = core.TSTrue
		settings.InsertSpaceAfterOpeningAndBeforeClosingTemplateStringBraces = core.TSTrue
		settings.InsertSpaceAfterOpeningAndBeforeClosingJsxExpressionBraces = core.TSTrue
		settings.InsertSpaceAfterTypeAssertion = core.TSTrue
		settings.InsertSpaceBeforeFunctionParenthesis = core.TSTrue
		settings.PlaceOpenBraceOnNewLineForFunctions = core.TSTrue
		settings.PlaceOpenBraceOnNewLineForControlBlocks = core.TSTrue
		settings.InsertSpaceBeforeTypeAnnotation = core.TSTrue
		settings.IndentMultiLineObjectLiteralBeginningOnBlankLine = core.TSTrue
		settings.IndentSwitchCase = core.TSFalse
		settings.TrimTrailingWhitespace = core.TSFalse
		settings.Semicolons = lsutil.SemicolonPreferenceRemove
		if os.Args[1] == "insert" {
			settings.Semicolons = lsutil.SemicolonPreferenceInsert
			settings.NewLineCharacter = "\r\n"
		}
	}
	ctx := format.WithFormatCodeSettings(context.Background(), settings, "\n")
	in := bufio.NewScanner(os.Stdin)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		path := in.Text()
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		kind := core.ScriptKindTS
		name := "/file.ts"
		if strings.HasSuffix(path, ".tsx") {
			kind = core.ScriptKindTSX
			name = "/file.tsx"
		}
		var sb strings.Builder
		func() {
			defer func() {
				if r := recover(); r != nil {
					sb.Reset()
					sb.WriteString("PANIC")
				}
			}()
			file := parser.ParseSourceFile(ast.SourceFileParseOptions{FileName: name, Path: "/file"}, string(data), kind)
			if len(os.Args) > 1 && os.Args[1] == "indent" {
				// GetIndentation at every line start (smart indenter), assumeNewLineBeforeCloseBrace alternating by line.
				for i, ls := range file.ECMALineMap() {
					fmt.Fprintf(&sb, "%d,", format.GetIndentation(int(ls), file, settings, i%2 == 0))
				}
				return
			}
			for _, e := range format.FormatDocument(ctx, file) {
				q, _ := json.Marshal(e.NewText)
				fmt.Fprintf(&sb, "%d,%d,%s;", e.Pos(), e.End(), q)
			}
		}()
		fmt.Fprintf(out, "%s\t%s\n", path, sb.String())
	}
}
