// tsrs-oracle-printer: printer oracle for the Rust printer port (crates/tsrs_printer).
//
// Usage:
//
//	tsrs-oracle-printer print MODE FILE [FLAGS]   print FILE with the printer
//	tsrs-oracle-printer hash MODE < list          print "hash path" for every "path[\tFLAGS]" line on stdin
//
// MODE selects the PrinterOptions: "default" (PrinterOptions{}, what `createPrinterWithDefaults` uses) or
// "nocomments" (RemoveComments, what `createPrinterWithRemoveComments` uses for type printing). The "synth" modes
// exercise the paths the checker's node builder hits: every top-level statement is deep-cloned through an EmitContext
// factory (synthesized nodes, no positions, original pointers) and written without a source file; "synth" uses a
// TextWriter("\n"), "synthflags" additionally sets EFSingleLine|EFNoAsciiEscaping on every cloned node and writes to
// the single-line string writer, "synthmulti" sets EFMultiLine|EFStartOnNewLine|EFIndented. Files are read and
// parsed like tools/oracle/ast does (osvfs, script kind from the file name, FLAGS = external module indicator options).
// Files with parse diagnostics are reported as "SKIP" (hash mode) since the comparison only covers files that parse
// cleanly; a printer panic is reported as "PANIC".
// Keep this file in sync with crates/tsrs_printer/examples/printer_oracle.rs.
package main

import (
	"bufio"
	"fmt"
	"hash/fnv"
	"os"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/parser"
	"github.com/microsoft/TypeScript/tsc/internal/printer"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
	"github.com/microsoft/TypeScript/tsc/internal/vfs/osvfs"
)

func parseFlags(flags string) ast.ExternalModuleIndicatorOptions {
	return ast.ExternalModuleIndicatorOptions{JSX: strings.Contains(flags, "j"), Force: strings.Contains(flags, "f")}
}

func splitLine(line string) (string, string) {
	path, flags, _ := strings.Cut(line, "\t")
	if flags == "" {
		flags = "-"
	}
	return path, flags
}

func options(mode string) printer.PrinterOptions {
	switch mode {
	case "default":
		return printer.PrinterOptions{}
	case "nocomments":
		return printer.PrinterOptions{RemoveComments: true}
	default:
		panic("unknown mode " + mode)
	}
}

// Returns (text, "") or ("", "SKIP"/"PANIC"/"UNREADABLE").
func printFile(mode string, path string, flags string) (text string, status string) {
	content, ok := osvfs.FS().ReadFile(path)
	if !ok {
		return "", "UNREADABLE"
	}
	opts := ast.SourceFileParseOptions{FileName: path, Path: tspath.Path(path), ExternalModuleIndicatorOptions: parseFlags(flags)}
	file := parser.ParseSourceFile(opts, content, core.EnsureScriptKindFromFileName(path))
	if len(file.Diagnostics()) > 0 {
		return "", "SKIP"
	}
	defer func() {
		if r := recover(); r != nil {
			text = ""
			status = "PANIC"
		}
	}()
	if strings.HasPrefix(mode, "synth") {
		return printSynthesized(mode, file), ""
	}
	p := printer.NewPrinter(options(mode), printer.PrintHandlers{}, nil)
	return p.EmitSourceFile(file), ""
}

func setFlagsRecursive(ec *printer.EmitContext, node *ast.Node, flags printer.EmitFlags) {
	ec.AddEmitFlags(node, flags)
	node.ForEachChild(func(child *ast.Node) bool {
		setFlagsRecursive(ec, child, flags)
		return false
	})
}

func printSynthesized(mode string, file *ast.SourceFile) string {
	ec := printer.NewEmitContext()
	p := printer.NewPrinter(printer.PrinterOptions{RemoveComments: true}, printer.PrintHandlers{}, ec)
	var sb strings.Builder
	for _, stmt := range file.Statements.Nodes {
		clone := ec.Factory.DeepCloneNode(stmt)
		var w printer.EmitTextWriter
		switch mode {
		case "synth":
			w = printer.NewTextWriter("\n", 0)
		case "synthflags":
			setFlagsRecursive(ec, clone, printer.EFSingleLine|printer.EFNoAsciiEscaping)
			w, _ = printer.GetSingleLineStringWriter()
		case "synthmulti":
			setFlagsRecursive(ec, clone, printer.EFMultiLine|printer.EFStartOnNewLine|printer.EFIndented)
			w = printer.NewTextWriter("\n", 0)
		default:
			panic("unknown mode " + mode)
		}
		p.Write(clone, nil, w, nil)
		sb.WriteString(w.String())
		sb.WriteString("\n---\n")
	}
	return sb.String()
}

func main() {
	switch os.Args[1] {
	case "print":
		flags := "-"
		if len(os.Args) > 4 {
			flags = os.Args[4]
		}
		text, status := printFile(os.Args[2], os.Args[3], flags)
		if status != "" {
			fmt.Fprintln(os.Stderr, status)
		}
		os.Stdout.WriteString(text)
	case "hash":
		mode := os.Args[2]
		in := bufio.NewScanner(os.Stdin)
		in.Buffer(make([]byte, 1<<20), 1<<20)
		w := bufio.NewWriter(os.Stdout)
		defer w.Flush()
		for in.Scan() {
			path, flags := splitLine(in.Text())
			text, status := printFile(mode, path, flags)
			if status == "UNREADABLE" {
				continue
			}
			if status != "" {
				fmt.Fprintf(w, "%s %s\n", status, path)
				continue
			}
			h := fnv.New64a()
			h.Write([]byte(text))
			fmt.Fprintf(w, "%016x %s\n", h.Sum64(), path)
		}
	default:
		panic("unknown command " + os.Args[1])
	}
}
