// tsrs-oracle-project-types: the `.types` / `.symbols` baseline walk of testutil/tsbaseline
// (type_symbol_baseline.go) over a whole tsconfig project, single-threaded.
//
//	tsrs-oracle-project-types [--mode types|symbols|both] [--text all|none|<list file>] --out <dir> <tsconfig.json>
//
// Loads the project like `tsc -p` (cached FS compiler host, config from the given tsconfig, cwd = process cwd),
// collects diagnostics like tsc (GetDiagnosticsOfAnyProgram, which checks every file), then walks every source
// file that is not under node_modules and not a default library file, in program order, exactly like
// iterateBaseline/typeWriterWalker (hasErrorBaseline = any diagnostics). `both` runs the type walk over all files
// and then the symbol walk (the test harness order).
//
// Output: <out>/manifest.<kind> with one line per file "<fnv1a64 hex>\t<line count>\t<relative path>" and a final
// "#counts\t<types>\t<symbols>\t<instantiations>" line (checker counters after the walk); the per-file text goes to
// <out>/<kind>/<relative path>.<kind> for every file (`--text all`, default), none, or the files listed (relative
// paths, one per line; with a single --mode the walk stops after the last listed file). Relative paths are relative to the tsconfig directory, with `..` segments written as `_up_`.
//
// Build (from ts-ref/tsc, source copied to cmd/tsrs-oracle-project-types/main.go):
//
//	GOTOOLCHAIN=auto go build -o ../../bin/tsrs-oracle-project-types ./cmd/tsrs-oracle-project-types
package main

import (
	"bufio"
	"context"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/bundled"
	"github.com/microsoft/TypeScript/tsc/internal/checker"
	"github.com/microsoft/TypeScript/tsc/internal/compiler"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/execute/tsc"
	"github.com/microsoft/TypeScript/tsc/internal/nodebuilder"
	"github.com/microsoft/TypeScript/tsc/internal/printer"
	"github.com/microsoft/TypeScript/tsc/internal/scanner"
	"github.com/microsoft/TypeScript/tsc/internal/tsoptions"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
	"github.com/microsoft/TypeScript/tsc/internal/vfs"
	"github.com/microsoft/TypeScript/tsc/internal/vfs/osvfs"
)

var (
	codeLinesRegexp  = regexp.MustCompile("[\r  ]|\r?\n")
	bracketLineRegex = regexp.MustCompile(`^\s*[{|}]\s*$`)
	lineDelimiter    = regexp.MustCompile("\r?\n")

	testPathPrefixReplacer = strings.NewReplacer(
		"/.ts/", "",
		"/.lib/", "",
		"/.src/", "",
		"bundled:///libs/", "",
		"file:///./ts/", "file:///",
		"file:///./lib/", "file:///",
		"file:///./src/", "file:///",
	)
)

type host struct {
	fs  vfs.FS
	cwd string
}

func (h *host) FS() vfs.FS                  { return h.fs }
func (h *host) GetCurrentDirectory() string { return h.cwd }

func main() {
	mode := flag.String("mode", "types", "types|symbols|both")
	text := flag.String("text", "all", "all|none|<file with relative paths>")
	out := flag.String("out", "", "output directory")
	flag.Parse()
	if *out == "" || flag.NArg() != 1 {
		fmt.Fprintln(os.Stderr, "usage: tsrs-oracle-project-types [--mode types|symbols|both] [--text all|none|<list>] --out <dir> <tsconfig.json>")
		os.Exit(2)
	}

	cwd, _ := os.Getwd()
	h := &host{fs: bundled.WrapFS(osvfs.FS()), cwd: tspath.NormalizePath(cwd)}
	ecc := &tsc.ExtendedConfigCache{}
	configPath := tspath.GetNormalizedAbsolutePath(flag.Arg(0), h.cwd)
	cfg, errs := tsoptions.GetParsedCommandLineOfConfigFile(configPath, &core.CompilerOptions{}, nil, h, ecc)
	if len(errs) > 0 {
		fmt.Fprintln(os.Stderr, "config errors:", len(errs))
		os.Exit(1)
	}
	ch := compiler.NewCachedFSCompilerHost(h.cwd, h.fs, bundled.LibPath(), ecc, nil, nil)
	program := compiler.NewProgram(compiler.ProgramOptions{
		ProgramConfig: compiler.ProgramConfig{Config: cfg, SingleThreaded: core.TSTrue},
		ProgramHosts:  compiler.ProgramHosts{Host: ch},
	})
	ctx := context.Background()
	diags := compiler.GetDiagnosticsOfAnyProgram(ctx, program, nil, false, program.GetBindDiagnostics, program.GetSemanticDiagnostics)
	fmt.Fprintf(os.Stderr, "diagnostics: %d\n", len(diags))

	configDir := tspath.GetDirectoryPath(configPath)
	var files []*ast.SourceFile
	var rels []string
	for _, f := range program.SourceFiles() {
		if strings.Contains(f.FileName(), "/node_modules/") || program.IsSourceFileDefaultLibrary(f.Path()) {
			continue
		}
		files = append(files, f)
		rels = append(rels, relName(configDir, f.FileName()))
	}

	var wanted map[string]bool
	switch *text {
	case "all", "none":
	default:
		data, err := os.ReadFile(*text)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		wanted = map[string]bool{}
		for _, l := range strings.Split(string(data), "\n") {
			if l = strings.TrimSpace(l); l != "" {
				wanted[l] = true
			}
		}
	}

	walker := &typeWriterWalker{program: program, hadErrorBaseline: len(diags) > 0}
	var kinds []bool
	switch *mode {
	case "types":
		kinds = []bool{false}
	case "symbols":
		kinds = []bool{true}
	case "both":
		kinds = []bool{false, true}
	default:
		fmt.Fprintln(os.Stderr, "bad --mode")
		os.Exit(2)
	}
	for _, isSymbol := range kinds {
		kind := core.IfElse(isSymbol, "symbols", "types")
		if err := os.MkdirAll(*out, 0o755); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		mf, err := os.Create(filepath.Join(*out, "manifest."+kind))
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		mw := bufio.NewWriter(mf)
		for i, f := range files {
			section := walker.fileBaseline(f, rels[i], isSymbol)
			fmt.Fprintf(mw, "%016x\t%d\t%s\n", fnv1a64(section), strings.Count(section, "\n"), rels[i])
			if *text == "all" || wanted[rels[i]] {
				p := filepath.Join(*out, kind, rels[i]+"."+kind)
				_ = os.MkdirAll(filepath.Dir(p), 0o755)
				if err := os.WriteFile(p, []byte(section), 0o644); err != nil {
					fmt.Fprintln(os.Stderr, err)
					os.Exit(1)
				}
			}
			if wanted != nil && len(kinds) == 1 && wanted[rels[i]] {
				delete(wanted, rels[i])
				if len(wanted) == 0 {
					// Every listed file is written; later files cannot change them.
					break
				}
			}
			if (i+1)%1000 == 0 {
				fmt.Fprintf(os.Stderr, "%s: %d/%d\n", kind, i+1, len(files))
			}
		}
		c, done := program.GetTypeChecker(ctx)
		fmt.Fprintf(mw, "#counts\t%d\t%d\t%d\n", c.TypeCount, c.SymbolCount, c.TotalInstantiationCount)
		done()
		mw.Flush()
		mf.Close()
	}
}

func relName(dir string, fileName string) string {
	rel := tspath.GetRelativePathFromDirectory(dir, fileName, tspath.ComparePathsOptions{UseCaseSensitiveFileNames: true})
	parts := strings.Split(rel, "/")
	for i, p := range parts {
		if p == ".." {
			parts[i] = "_up_"
		}
	}
	return strings.Join(parts, "/")
}

func fnv1a64(s string) uint64 {
	h := uint64(14695981039346656037)
	for i := 0; i < len(s); i++ {
		h ^= uint64(s[i])
		h *= 1099511628211
	}
	return h
}

// iterateBaseline for one file, without the "//// [header] ////" prefix.
func (walker *typeWriterWalker) fileBaseline(file *ast.SourceFile, unitName string, isSymbolBaseline bool) string {
	var typeLines strings.Builder
	typeLines.WriteString("=== ")
	typeLines.WriteString(unitName)
	typeLines.WriteString(" ===\r\n")
	codeLines := codeLinesRegexp.Split(file.Text(), -1)
	walker.currentSourceFile = file
	results := walker.visitNode(file.AsNode(), isSymbolBaseline)
	lastIndexWritten := -1
	for _, result := range results {
		if isSymbolBaseline && result.symbol == "" {
			break
		}
		if lastIndexWritten == -1 {
			typeLines.WriteString(strings.Join(codeLines[:result.line+1], "\r\n"))
			typeLines.WriteString("\r\n")
		} else if lastIndexWritten != result.line {
			if !(lastIndexWritten+1 < len(codeLines) &&
				(bracketLineRegex.MatchString(codeLines[lastIndexWritten+1]) || strings.TrimSpace(codeLines[lastIndexWritten+1]) == "")) {
				typeLines.WriteString("\r\n")
			}
			typeLines.WriteString(strings.Join(codeLines[lastIndexWritten+1:result.line+1], "\r\n"))
			typeLines.WriteString("\r\n")
		}
		lastIndexWritten = result.line
		typeOrSymbolString := core.IfElse(isSymbolBaseline, result.symbol, result.typ)
		lineText := lineDelimiter.ReplaceAllString(result.sourceText, "")
		typeLines.WriteString(">")
		fmt.Fprintf(&typeLines, "%s : %s", lineText, typeOrSymbolString)
		typeLines.WriteString("\r\n")
	}

	if lastIndexWritten+1 < len(codeLines) {
		if !(lastIndexWritten+1 < len(codeLines) &&
			(bracketLineRegex.MatchString(codeLines[lastIndexWritten+1]) || strings.TrimSpace(codeLines[lastIndexWritten+1]) == "")) {
			typeLines.WriteString("\r\n")
		}
		typeLines.WriteString(strings.Join(codeLines[lastIndexWritten+1:], "\r\n"))
	}
	typeLines.WriteString("\r\n")
	return testPathPrefixReplacer.Replace(typeLines.String())
}

type typeWriterWalker struct {
	program           *compiler.Program
	hadErrorBaseline  bool
	currentSourceFile *ast.SourceFile
}

type typeWriterResult struct {
	line       int
	sourceText string
	symbol     string
	typ        string
}

func (walker *typeWriterWalker) visitNode(node *ast.Node, isSymbolWalk bool) []*typeWriterResult {
	nodes := forEachASTNode(node)
	var results []*typeWriterResult
	for _, n := range nodes {
		if ast.IsExpressionNode(n) || n.Kind == ast.KindIdentifier || ast.IsDeclarationName(n) ||
			ast.IsQualifiedName(n) && ast.IsNameOfHeritageClauseTypeReference(n) && (isSymbolWalk || ast.IsQualifiedName(n.Parent)) {
			result := walker.writeTypeOrSymbol(n, isSymbolWalk)
			if result != nil {
				results = append(results, result)
			}
		}
	}
	return results
}

func forEachASTNode(node *ast.Node) []*ast.Node {
	var result []*ast.Node
	work := []*ast.Node{node}

	var resChildren []*ast.Node
	addChild := func(child *ast.Node) bool {
		resChildren = append(resChildren, child)
		return false
	}

	for len(work) > 0 {
		elem := work[len(work)-1]
		work = work[:len(work)-1]
		if elem.Flags&ast.NodeFlagsReparsed == 0 || elem.Kind == ast.KindAsExpression || elem.Kind == ast.KindSatisfiesExpression ||
			((elem.Parent.Kind == ast.KindSatisfiesExpression || elem.Parent.Kind == ast.KindAsExpression) && elem == elem.Parent.Expression()) {
			if elem.Flags&ast.NodeFlagsReparsed == 0 || elem.Parent.Kind == ast.KindAsExpression || elem.Parent.Kind == ast.KindSatisfiesExpression {
				result = append(result, elem)
			}
			elem.ForEachChild(addChild)
			slices.Reverse(resChildren)
			work = append(work, resChildren...)
			resChildren = resChildren[:0]
		}
	}
	return result
}

func (walker *typeWriterWalker) writeTypeOrSymbol(node *ast.Node, isSymbolWalk bool) *typeWriterResult {
	actualPos := scanner.SkipTrivia(walker.currentSourceFile.Text(), node.Pos())
	line := scanner.GetECMALineOfPosition(walker.currentSourceFile, actualPos)
	sourceText := scanner.GetSourceTextOfNodeFromSourceFile(walker.currentSourceFile, node, false /*includeTrivia*/)
	fileChecker, done := walker.program.GetTypeCheckerForFile(context.Background(), walker.currentSourceFile)
	defer done()

	ctx, putCtx := printer.GetEmitContext()
	defer putCtx()

	if !isSymbolWalk {
		if ast.IsPartOfTypeNode(node) ||
			(node.Kind == ast.KindAsExpression || node.Kind == ast.KindSatisfiesExpression) && node.Type().Flags&ast.NodeFlagsReparsed != 0 ||
			ast.IsIdentifier(node) &&
				(ast.GetMeaningFromDeclaration(node.Parent)&ast.SemanticMeaningValue) == 0 &&
				!(ast.IsTypeOrJSTypeAliasDeclaration(node.Parent) && node == node.Parent.Name()) {
			return nil
		}

		if ast.IsOmittedExpression(node) {
			return nil
		}

		var t *checker.Type
		if ast.IsExpressionWithTypeArgumentsInClassExtendsClause(node.Parent) {
			t = fileChecker.GetTypeAtLocation(node.Parent)
		}
		if t == nil || checker.IsTypeAny(t) {
			t = fileChecker.GetTypeAtLocation(node)
		}
		var typeString string
		if !walker.hadErrorBaseline &&
			checker.IsTypeAny(t) &&
			!ast.IsBindingElement(node.Parent) &&
			!ast.IsPropertyAccessOrQualifiedName(node.Parent) &&
			!ast.IsLabelName(node) &&
			!ast.IsGlobalScopeAugmentation(node.Parent) &&
			!ast.IsMetaProperty(node.Parent) &&
			!isImportStatementName(node) &&
			!isExportStatementName(node) &&
			!isIntrinsicJsxTag(node, walker.currentSourceFile) {
			typeString = t.AsIntrinsicType().IntrinsicName()
		} else {
			ctx.Reset()
			builder := checker.NewNodeBuilder(fileChecker, ctx)
			typeFormatFlags := checker.TypeFormatFlagsNoTruncation | checker.TypeFormatFlagsAllowUniqueESSymbolType | checker.TypeFormatFlagsGenerateNamesForShadowedTypeParams
			typeNode := builder.TypeToTypeNode(t, node.Parent, nodebuilder.Flags(typeFormatFlags&checker.TypeFormatFlagsNodeBuilderFlagsMask)|nodebuilder.FlagsIgnoreErrors, nodebuilder.InternalFlagsAllowUnresolvedNames, nil)
			if ast.IsIdentifier(node) && ast.IsTypeAliasDeclaration(node.Parent) && node.Parent.Name() == node && ast.IsIdentifier(typeNode) && typeNode.Text() == node.Text() {
				typeNode = builder.TypeToTypeNode(t, node.Parent, nodebuilder.Flags((typeFormatFlags|checker.TypeFormatFlagsInTypeAlias)&checker.TypeFormatFlagsNodeBuilderFlagsMask)|nodebuilder.FlagsIgnoreErrors, nodebuilder.InternalFlagsAllowUnresolvedNames, nil)
			}

			writer := printer.NewTextWriter("", 0)
			printer := printer.NewPrinter(printer.PrinterOptions{RemoveComments: true}, printer.PrintHandlers{}, ctx)
			printer.Write(typeNode, walker.currentSourceFile, writer, nil)
			typeString = writer.String()
		}
		return &typeWriterResult{line: line, sourceText: sourceText, typ: typeString}
	}

	symbol := fileChecker.GetSymbolAtLocation(node)
	if symbol == nil {
		return nil
	}

	var symbolString strings.Builder
	symbolString.Grow(256)
	symbolString.WriteString("Symbol(")
	symbolString.WriteString(ast.EscapeAllInternalSymbolNames(fileChecker.SymbolToStringEx(symbol, node.Parent, ast.SymbolFlagsNone, checker.SymbolFormatFlagsAllowAnyNodeKind)))
	count := 0
	for _, declaration := range symbol.Declarations {
		if count >= 5 {
			fmt.Fprintf(&symbolString, " ... and %d more", len(symbol.Declarations)-count)
			break
		}
		count++
		symbolString.WriteString(", ")
		declSourceFile := ast.GetSourceFileOfNode(declaration)
		declLine, declChar := scanner.GetECMALineAndUTF16CharacterOfPosition(declSourceFile, declaration.Pos())
		fileName := tspath.GetBaseFileName(declSourceFile.FileName())
		symbolString.WriteString("Decl(")
		symbolString.WriteString(fileName)
		symbolString.WriteString(", ")
		if isDefaultLibraryFile(fileName) {
			symbolString.WriteString("--, --)")
		} else {
			fmt.Fprintf(&symbolString, "%d, %d)", declLine, int(declChar))
		}
	}
	symbolString.WriteString(")")
	return &typeWriterResult{line: line, sourceText: sourceText, symbol: symbolString.String()}
}

func isDefaultLibraryFile(filePath string) bool {
	fileName := tspath.GetBaseFileName(filePath)
	return strings.HasPrefix(fileName, "lib.") && strings.HasSuffix(fileName, tspath.ExtensionDts)
}

func isImportStatementName(node *ast.Node) bool {
	if ast.IsImportSpecifier(node.Parent) && (node == node.Parent.Name() || node == node.Parent.PropertyName()) {
		return true
	}
	if ast.IsImportClause(node.Parent) && node == node.Parent.Name() {
		return true
	}
	if ast.IsImportEqualsDeclaration(node.Parent) && node == node.Parent.Name() {
		return true
	}
	return false
}

func isExportStatementName(node *ast.Node) bool {
	if ast.IsExportAssignment(node.Parent) && node == node.Parent.Expression() {
		return true
	}
	if ast.IsExportSpecifier(node.Parent) && (node == node.Parent.Name() || node == node.Parent.PropertyName()) {
		return true
	}
	return false
}

func isIntrinsicJsxTag(node *ast.Node, sourceFile *ast.SourceFile) bool {
	if !(ast.IsJsxOpeningElement(node.Parent) || ast.IsJsxClosingElement(node.Parent) || ast.IsJsxSelfClosingElement(node.Parent)) {
		return false
	}
	if node.Parent.TagName() != node {
		return false
	}
	text := scanner.GetSourceTextOfNodeFromSourceFile(sourceFile, node, false /*includeTrivia*/)
	return scanner.IsIntrinsicJsxName(text)
}
