// tsrs-oracle-ast: parser oracle for the Rust parser port.
//
// Usage:
//
//	tsrs-oracle-ast dump FILE [FLAGS]   print the AST dump of FILE
//	tsrs-oracle-ast hash < list         print "hash path" for every "path[\tFLAGS]" line on stdin
//	tsrs-oracle-ast split OUTDIR < list materialize the units of every test case named on stdin under
//	                                    OUTDIR and print a file list ("path\tFLAGS") for hash/dump
//	tsrs-oracle-ast bench < list        parse every listed file once (single goroutine) and report time
//
// FLAGS selects the ExternalModuleIndicatorOptions the file is parsed with: "-" (none), "j" (JSX),
// "f" (Force), "jf" (both). Files are read like the compiler reads them (osvfs: UTF-16 BOM decoding,
// UTF-8 BOM stripping) and parsed with the script kind the compiler host derives from the file name.
//
// The dump lists every node in ForEachChild pre-order ("N"), with the JSDoc nodes Node.JSDoc(file)
// returns for it printed before its children ("J", which triggers lazy JSDoc parsing in TS files): depth,
// kind, pos, end, node flags, the non-child fields of the node struct (fields_gen.go, generated from the
// AST schema), and the actual parent when it differs from the traversal parent. Then the SourceFile
// results: parse/JS/JSDoc diagnostics, imports, module augmentations, ambient module names, comment
// directives, reparsed clones, pragmas, referenced files, type reference and lib directives, the checkJs
// directive, CommonJS/external module indicators (also recomputed for the JSX and Force options).
// Keep this file in sync with crates/tsrs_parser/examples/ast_oracle/main.rs.
package main

import (
	"bufio"
	"fmt"
	"hash/fnv"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/parser"
	"github.com/microsoft/TypeScript/tsc/internal/testrunner"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
	"github.com/microsoft/TypeScript/tsc/internal/vfs/osvfs"
)

type fieldSpec struct {
	name string
	cat  string
}

func escape(sb *strings.Builder, s string) {
	sb.WriteByte('"')
	for i := 0; i < len(s); i++ {
		b := s[i]
		if b >= 0x20 && b < 0x7f && b != '\\' && b != '"' {
			sb.WriteByte(b)
		} else {
			fmt.Fprintf(sb, "\\x%02x", b)
		}
	}
	sb.WriteByte('"')
}

func kindName(k ast.Kind) string {
	return strings.TrimPrefix(k.String(), "Kind")
}

type dumper struct {
	sb   strings.Builder
	file *ast.SourceFile
}

func (d *dumper) nodeRef(n *ast.Node) string {
	if n == nil {
		return "-"
	}
	if n == d.file.AsNode() {
		return "self"
	}
	return fmt.Sprintf("%s@%d:%d", kindName(n.Kind), n.Pos(), n.End())
}

func intOf(v reflect.Value) int64 {
	if v.CanInt() {
		return v.Int()
	}
	return int64(v.Uint())
}

func textRange(v reflect.Value) (int64, int64) {
	return v.FieldByName("pos").Int(), v.FieldByName("end").Int()
}

func (d *dumper) fields(n *ast.Node) {
	data := reflect.ValueOf(n).Elem().FieldByName("data")
	if data.IsNil() {
		return
	}
	s := data.Elem().Elem()
	specs, ok := nodeFields[s.Type().Name()]
	if !ok {
		return
	}
	for _, spec := range specs {
		v := s.FieldByName(spec.name)
		if !v.IsValid() {
			panic("no field " + spec.name + " in " + s.Type().Name())
		}
		d.sb.WriteByte(' ')
		d.sb.WriteString(spec.name)
		switch spec.cat {
		case "list", "modlist":
			if v.IsNil() {
				d.sb.WriteString("-")
				continue
			}
			l := v.Elem()
			var loc reflect.Value
			var nodes reflect.Value
			if spec.cat == "modlist" {
				loc = l.FieldByName("NodeList").FieldByName("Loc")
				nodes = l.FieldByName("NodeList").FieldByName("Nodes")
			} else {
				loc = l.FieldByName("Loc")
				nodes = l.FieldByName("Nodes")
			}
			pos, end := textRange(loc)
			fmt.Fprintf(&d.sb, "[%d,%d,%d", pos, end, nodes.Len())
			if spec.cat == "modlist" {
				fmt.Fprintf(&d.sb, ",%x", uint32(intOf(l.FieldByName("ModifierFlags"))))
			}
			d.sb.WriteByte(']')
		case "rawnodes":
			d.sb.WriteByte('{')
			for i := 0; i < v.Len(); i++ {
				if i > 0 {
					d.sb.WriteByte(',')
				}
				e := v.Index(i).Elem()
				pos, end := textRange(e.FieldByName("Loc"))
				fmt.Fprintf(&d.sb, "%s@%d:%d", kindName(ast.Kind(e.FieldByName("Kind").Int())), pos, end)
			}
			d.sb.WriteByte('}')
		case "rawstrings":
			d.sb.WriteByte('{')
			for i := 0; i < v.Len(); i++ {
				if i > 0 {
					d.sb.WriteByte(',')
				}
				escape(&d.sb, v.Index(i).String())
			}
			d.sb.WriteByte('}')
		case "bool":
			if v.Bool() {
				d.sb.WriteString("=1")
			} else {
				d.sb.WriteString("=0")
			}
		case "string":
			d.sb.WriteByte('=')
			escape(&d.sb, v.String())
		case "int":
			fmt.Fprintf(&d.sb, "=%d", intOf(v))
		case "tokenflags":
			fmt.Fprintf(&d.sb, "=%x", uint32(intOf(v)))
		case "kind":
			d.sb.WriteString("=" + kindName(ast.Kind(intOf(v))))
		default:
			panic("bad category " + spec.cat)
		}
	}
}

func (d *dumper) node(n *ast.Node, depth int, parent *ast.Node, tag byte) {
	d.sb.WriteByte(tag)
	fmt.Fprintf(&d.sb, "%d %s %d %d %x", depth, kindName(n.Kind), n.Pos(), n.End(), uint32(n.Flags))
	d.fields(n)
	if n.Parent != parent {
		d.sb.WriteString(" P!" + d.nodeRef(n.Parent))
	}
	d.sb.WriteByte('\n')
	for _, j := range n.JSDoc(d.file) {
		d.node(j, depth+1, n, 'J')
	}
	n.ForEachChild(func(c *ast.Node) bool {
		d.node(c, depth+1, n, 'N')
		return false
	})
}

func (d *dumper) diagnostics(tag string, diags []*ast.Diagnostic) {
	for _, diag := range diags {
		fmt.Fprintf(&d.sb, "%s %d %d %d %d ", tag, diag.Code(), int(diag.Category()), diag.Pos(), diag.Len())
		escape(&d.sb, diag.MessageText())
		fmt.Fprintf(&d.sb, " C%d R", len(diag.MessageChain()))
		for _, r := range diag.RelatedInformation() {
			fmt.Fprintf(&d.sb, " %d:%d:%d", r.Code(), r.Pos(), r.Len())
		}
		d.sb.WriteByte('\n')
	}
}

func (d *dumper) fileRefs(tag string, refs []*ast.FileReference) {
	for _, r := range refs {
		fmt.Fprintf(&d.sb, "%s %d %d ", tag, r.Pos(), r.End())
		escape(&d.sb, r.FileName)
		fmt.Fprintf(&d.sb, " %d %v\n", int(r.ResolutionMode), r.Preserve)
	}
}

func parseFlags(flags string) ast.ExternalModuleIndicatorOptions {
	return ast.ExternalModuleIndicatorOptions{JSX: strings.Contains(flags, "j"), Force: strings.Contains(flags, "f")}
}

func parse(path string, flags string) (*ast.SourceFile, bool) {
	text, ok := osvfs.FS().ReadFile(path)
	if !ok {
		return nil, false
	}
	opts := ast.SourceFileParseOptions{FileName: path, Path: tspath.Path(path), ExternalModuleIndicatorOptions: parseFlags(flags)}
	return parser.ParseSourceFile(opts, text, core.EnsureScriptKindFromFileName(path)), true
}

func dump(path string, flags string) (string, error) {
	file, ok := parse(path, flags)
	if !ok {
		return "", fmt.Errorf("cannot read %s", path)
	}
	d := &dumper{file: file}
	fmt.Fprintf(&d.sb, "FILE sk=%d lv=%d decl=%v uri=%d ids=%d nodes=%d texts=%d\n", int(file.ScriptKind), int(file.LanguageVariant), file.IsDeclarationFile, int(file.UsesUriStyleNodeCoreModules), file.IdentifierCount, file.NodeCount, file.TextCount)
	d.node(file.AsNode(), 0, nil, 'N')
	d.diagnostics("D", file.Diagnostics())
	d.diagnostics("DJS", file.JSDiagnostics())
	d.diagnostics("DJD", file.JSDocDiagnostics())
	for _, imp := range file.Imports() {
		d.sb.WriteString("IMP " + d.nodeRef(imp) + " ")
		escape(&d.sb, imp.Text())
		d.sb.WriteByte('\n')
	}
	for _, aug := range file.ModuleAugmentations {
		d.sb.WriteString("AUG " + d.nodeRef(aug) + " ")
		escape(&d.sb, aug.Text())
		d.sb.WriteByte('\n')
	}
	for _, name := range file.AmbientModuleNames {
		d.sb.WriteString("AMB ")
		escape(&d.sb, name)
		d.sb.WriteByte('\n')
	}
	for _, cd := range file.CommentDirectives {
		fmt.Fprintf(&d.sb, "CD %d %d %d\n", cd.Loc.Pos(), cd.Loc.End(), int(cd.Kind))
	}
	for _, rc := range file.ReparsedClones {
		d.sb.WriteString("RC " + d.nodeRef(rc) + "\n")
	}
	for _, p := range file.Pragmas {
		fmt.Fprintf(&d.sb, "PRAGMA %s %d %d %d %v", p.Name, int(p.Kind), p.Pos(), p.End(), p.HasTrailingNewLine)
		keys := make([]string, 0, len(p.Args))
		for k := range p.Args {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			a := p.Args[k]
			fmt.Fprintf(&d.sb, " %s=(%d,%d,", k, a.Pos(), a.End())
			escape(&d.sb, a.Name)
			d.sb.WriteByte(',')
			escape(&d.sb, a.Value)
			d.sb.WriteByte(')')
		}
		d.sb.WriteByte('\n')
	}
	d.fileRefs("REF", file.ReferencedFiles)
	d.fileRefs("TREF", file.TypeReferenceDirectives)
	d.fileRefs("LREF", file.LibReferenceDirectives)
	if c := file.CheckJsDirective; c != nil {
		fmt.Fprintf(&d.sb, "CHECKJS %v %d %d\n", c.Enabled, c.Range.Pos(), c.Range.End())
	}
	d.sb.WriteString("CJS " + d.nodeRef(file.CommonJSModuleIndicator) + "\n")
	d.sb.WriteString("EMI " + d.nodeRef(file.ExternalModuleIndicator) + "\n")
	ast.SetExternalModuleIndicator(file, ast.ExternalModuleIndicatorOptions{JSX: true})
	d.sb.WriteString("EMI_J " + d.nodeRef(file.ExternalModuleIndicator) + "\n")
	ast.SetExternalModuleIndicator(file, ast.ExternalModuleIndicatorOptions{Force: true})
	d.sb.WriteString("EMI_F " + d.nodeRef(file.ExternalModuleIndicator) + "\n")
	return d.sb.String(), nil
}

func splitLine(line string) (string, string) {
	path, flags, _ := strings.Cut(line, "\t")
	if flags == "" {
		flags = "-"
	}
	return path, flags
}

var jsxOptionValues = map[string]bool{"react-jsx": true, "react-jsxdev": true}

// Materializes the units of one test case under dir. Unit names are sanitized to stay inside dir and
// prefixed with their index so that repeated names do not collide.
func split(outDir string, casePath string, w *bufio.Writer) {
	content, ok := osvfs.FS().ReadFile(casePath)
	if !ok {
		fmt.Fprintln(os.Stderr, "cannot read", casePath)
		return
	}
	type unit struct {
		name    string
		content string
	}
	units, _, _, globalOptions, err := testrunner.ParseTestFilesAndSymlinks(content, tspath.GetBaseFileName(casePath), func(filename string, content string, fileOptions map[string]string) (*unit, error) {
		return &unit{name: filename, content: content}, nil
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, casePath, err)
		return
	}
	rel := strings.TrimSuffix(filepath.Base(filepath.Dir(casePath))+"/"+filepath.Base(casePath), filepath.Ext(casePath))
	jsx := jsxOptionValues[strings.ToLower(globalOptions["jsx"])]
	force := strings.ToLower(globalOptions["moduledetection"]) == "force"
	for i, u := range units {
		name := tspath.NormalizePath(u.name)
		name = strings.TrimLeft(name, "/")
		name = strings.ReplaceAll(name, "..", "__")
		name = strings.ReplaceAll(name, ":", "_")
		if name == "" {
			name = "unnamed.ts"
		}
		path := fmt.Sprintf("%s/%s/%d/%s", outDir, rel, i, name)
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			fmt.Fprintln(os.Stderr, err)
			continue
		}
		if err := os.WriteFile(path, []byte(u.content), 0o644); err != nil {
			fmt.Fprintln(os.Stderr, err)
			continue
		}
		flags := ""
		if !tspath.IsDeclarationFileName(path) {
			if jsx {
				flags += "j"
			}
			if force || tspath.FileExtensionIsOneOf(path, []string{tspath.ExtensionCjs, tspath.ExtensionCts, tspath.ExtensionMjs, tspath.ExtensionMts}) {
				flags += "f"
			}
		}
		if flags == "" {
			flags = "-"
		}
		fmt.Fprintf(w, "%s\t%s\n", path, flags)
	}
}

func main() {
	switch os.Args[1] {
	case "dump":
		flags := "-"
		if len(os.Args) > 3 {
			flags = os.Args[3]
		}
		out, err := dump(os.Args[2], flags)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Stdout.WriteString(out)
	case "hash":
		in := bufio.NewScanner(os.Stdin)
		in.Buffer(make([]byte, 1<<20), 1<<20)
		w := bufio.NewWriter(os.Stdout)
		defer w.Flush()
		for in.Scan() {
			path, flags := splitLine(in.Text())
			out, err := dump(path, flags)
			if err != nil {
				fmt.Fprintln(os.Stderr, err)
				continue
			}
			h := fnv.New64a()
			h.Write([]byte(out))
			fmt.Fprintf(w, "%016x %s\n", h.Sum64(), path)
		}
	case "split":
		in := bufio.NewScanner(os.Stdin)
		in.Buffer(make([]byte, 1<<20), 1<<20)
		w := bufio.NewWriter(os.Stdout)
		defer w.Flush()
		for in.Scan() {
			split(os.Args[2], in.Text(), w)
		}
	case "bench":
		in := bufio.NewScanner(os.Stdin)
		in.Buffer(make([]byte, 1<<20), 1<<20)
		type input struct {
			path, text string
			opts       ast.SourceFileParseOptions
		}
		var inputs []input
		total := 0
		for in.Scan() {
			path, flags := splitLine(in.Text())
			text, ok := osvfs.FS().ReadFile(path)
			if !ok {
				continue
			}
			total += len(text)
			inputs = append(inputs, input{path, text, ast.SourceFileParseOptions{FileName: path, Path: tspath.Path(path), ExternalModuleIndicatorOptions: parseFlags(flags)}})
		}
		rounds := 1
		if len(os.Args) > 2 {
			rounds, _ = strconv.Atoi(os.Args[2])
		}
		for r := 0; r < rounds; r++ {
			start := time.Now()
			nodes := 0
			for _, in := range inputs {
				f := parser.ParseSourceFile(in.opts, in.text, core.EnsureScriptKindFromFileName(in.path))
				nodes += f.NodeCount
			}
			el := time.Since(start)
			fmt.Printf("go: %d files, %.1f MB, %d nodes, %.3f s, %.1f MB/s\n", len(inputs), float64(total)/1e6, nodes, el.Seconds(), float64(total)/1e6/el.Seconds())
		}
	}
}
