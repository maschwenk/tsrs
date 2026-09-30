// tsrs-oracle-binder: binder oracle for the Rust binder port.
//
// Usage:
//   tsrs-oracle-binder dump FILE        print the binder dump of FILE
//   tsrs-oracle-binder hash < filelist  print "hash path" for every file named on stdin
//
// Each test file is parsed as one source file named /oracle/<basename> and bound. The dump lists, in
// ForEachChild pre-order, every node that carries binder output: binder-set node flags, its declared
// symbol (name, flags, declaration count, value declaration position, sorted exports/members), local
// symbol, sorted locals table, and references to flow nodes. Flow nodes are numbered in discovery order
// and printed afterwards (flags, associated node, antecedent(s)). Bind diagnostics and the symbol count
// come last. Internal symbol names are printed with the prefix replaced by "@@" (Go uses 0xFE, Rust 0x7F).
// Keep this file in sync with crates/tsrs_binder/examples/binder_oracle.rs.
package main

import (
	"bufio"
	"fmt"
	"hash/fnv"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/binder"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/parser"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
)

const binderNodeFlags = ast.NodeFlagsExportContext | ast.NodeFlagsContainsThis | ast.NodeFlagsHasImplicitReturn |
	ast.NodeFlagsHasExplicitReturn | ast.NodeFlagsThisNodeOrAnySubNodesHasError | ast.NodeFlagsHasAsyncFunctions |
	ast.NodeFlagsUnreachable

func escape(sb *strings.Builder, s string) {
	s = strings.ReplaceAll(s, ast.InternalSymbolNamePrefix, "@@")
	for i := 0; i < len(s); i++ {
		b := s[i]
		if b >= 0x20 && b < 0x7f && b != '\\' && b != ' ' {
			sb.WriteByte(b)
		} else {
			fmt.Fprintf(sb, "\\x%02x", b)
		}
	}
}

type dumper struct {
	sb    strings.Builder
	ids   map[*ast.FlowNode]int
	queue []*ast.FlowNode
}

func (d *dumper) flowRef(f *ast.FlowNode) string {
	if f == nil {
		return "-"
	}
	id, ok := d.ids[f]
	if !ok {
		id = len(d.ids) + 1
		d.ids[f] = id
		d.queue = append(d.queue, f)
	}
	return fmt.Sprintf("#%d", id)
}

func (d *dumper) flowList(l *ast.FlowList) string {
	var parts []string
	for ; l != nil; l = l.Next {
		parts = append(parts, d.flowRef(l.Flow))
	}
	return "[" + strings.Join(parts, ",") + "]"
}

func nodeRef(n *ast.Node) string {
	if n == nil {
		return "-"
	}
	return fmt.Sprintf("%d@%d", int(n.Kind), n.Pos())
}

func (d *dumper) table(tag string, t ast.SymbolTable) {
	if t == nil {
		return
	}
	names := make([]string, 0, len(t))
	for name := range t {
		names = append(names, name)
	}
	// Sort on the printed form: the internal-name prefix byte differs between Go and Rust.
	sort.Slice(names, func(i, j int) bool {
		return strings.ReplaceAll(names[i], ast.InternalSymbolNamePrefix, "@@") < strings.ReplaceAll(names[j], ast.InternalSymbolNamePrefix, "@@")
	})
	d.sb.WriteString(" " + tag + "{")
	for i, name := range names {
		if i > 0 {
			d.sb.WriteByte(' ')
		}
		escape(&d.sb, name)
		fmt.Fprintf(&d.sb, ":%x", uint32(t[name].Flags))
	}
	d.sb.WriteString("}")
}

func (d *dumper) symbol(tag string, s *ast.Symbol) {
	d.sb.WriteString(" " + tag + "(")
	escape(&d.sb, s.Name)
	fmt.Fprintf(&d.sb, " %x %d %s", uint32(s.Flags), len(s.Declarations), nodeRef(s.ValueDeclaration))
	if s.Parent != nil {
		d.sb.WriteString(" p=")
		escape(&d.sb, s.Parent.Name)
	}
	if s.ExportSymbol != nil {
		fmt.Fprintf(&d.sb, " x=%x", uint32(s.ExportSymbol.Flags))
	}
	d.table("X", s.Exports)
	d.table("M", s.Members)
	d.sb.WriteString(")")
}

func (d *dumper) visit(n *ast.Node) bool {
	var line strings.Builder
	old := d.sb
	d.sb = strings.Builder{}
	if f := n.Flags & binderNodeFlags; f != 0 {
		fmt.Fprintf(&d.sb, " F%x", uint32(f))
	}
	if data := n.DeclarationData(); data != nil && data.Symbol != nil {
		d.symbol("S", data.Symbol)
	}
	if data := n.ExportableData(); data != nil && data.LocalSymbol != nil {
		d.symbol("LS", data.LocalSymbol)
	}
	if data := n.LocalsContainerData(); data != nil {
		d.table("L", data.Locals)
		if data.NextContainer != nil {
			fmt.Fprintf(&d.sb, " next=%s", nodeRef(data.NextContainer))
		}
	}
	if data := n.FlowNodeData(); data != nil && data.FlowNode != nil {
		d.sb.WriteString(" f=" + d.flowRef(data.FlowNode))
	}
	if data := n.BodyData(); data != nil && data.EndFlowNode != nil {
		d.sb.WriteString(" e=" + d.flowRef(data.EndFlowNode))
	}
	var ret *ast.FlowNode
	switch n.Kind {
	case ast.KindConstructor:
		ret = n.AsConstructorDeclaration().ReturnFlowNode
	case ast.KindFunctionDeclaration:
		ret = n.AsFunctionDeclaration().ReturnFlowNode
	case ast.KindFunctionExpression:
		ret = n.AsFunctionExpression().ReturnFlowNode
	case ast.KindClassStaticBlockDeclaration:
		ret = n.AsClassStaticBlockDeclaration().ReturnFlowNode
	case ast.KindCaseClause, ast.KindDefaultClause:
		if t := n.AsCaseOrDefaultClause().FallthroughFlowNode; t != nil {
			d.sb.WriteString(" t=" + d.flowRef(t))
		}
	}
	if ret != nil {
		d.sb.WriteString(" r=" + d.flowRef(ret))
	}
	body := d.sb.String()
	d.sb = old
	if body != "" {
		fmt.Fprintf(&line, "N %d %d %d%s\n", int(n.Kind), n.Pos(), n.End(), body)
		d.sb.WriteString(line.String())
	}
	return n.ForEachChild(d.visit)
}

func (d *dumper) flowNode(id int, f *ast.FlowNode) {
	fmt.Fprintf(&d.sb, "F #%d %x ", id, uint32(f.Flags))
	switch {
	case f.Flags&ast.FlowFlagsSwitchClause != 0:
		data := f.Node.AsFlowSwitchClauseData()
		fmt.Fprintf(&d.sb, "SW(%s,%d,%d)", nodeRef(data.SwitchStatement), data.ClauseStart, data.ClauseEnd)
	case f.Flags&ast.FlowFlagsReduceLabel != 0:
		data := f.Node.AsFlowReduceLabelData()
		fmt.Fprintf(&d.sb, "RL(%s,%s)", d.flowRef(data.Target), d.flowList(data.Antecedents))
	default:
		d.sb.WriteString(nodeRef(f.Node))
	}
	fmt.Fprintf(&d.sb, " %s %s\n", d.flowRef(f.Antecedent), d.flowList(f.Antecedents))
}

func dump(path string) (string, error) {
	text, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	fileName := "/oracle/" + filepath.Base(path)
	opts := ast.SourceFileParseOptions{FileName: fileName, Path: tspath.Path(fileName)}
	file := parser.ParseSourceFile(opts, string(text), core.GetScriptKindFromFileName(fileName))
	binder.BindSourceFile(file)
	d := &dumper{ids: make(map[*ast.FlowNode]int)}
	d.visit(file.AsNode())
	for i := 0; i < len(d.queue); i++ {
		d.flowNode(i+1, d.queue[i])
	}
	for _, diag := range file.BindDiagnostics() {
		fmt.Fprintf(&d.sb, "B %d %d %d ", diag.Code(), diag.Pos(), diag.Len())
		escape(&d.sb, diag.MessageText())
		d.sb.WriteString(" R")
		for _, r := range diag.RelatedInformation() {
			fmt.Fprintf(&d.sb, " %d:%d:%d", r.Code(), r.Pos(), r.Len())
		}
		d.sb.WriteByte('\n')
	}
	fmt.Fprintf(&d.sb, "C %d %d\n", file.SymbolCount, len(file.PatternAmbientModules))
	return d.sb.String(), nil
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
