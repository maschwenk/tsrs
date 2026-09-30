// tsrs-oracle-nilfields parses the test corpus and lib files with the Go parser (as .ts, .tsx and .js)
// and reports, for every AST node struct field of pointer type, whether it was ever observed nil.
// Output: JSON {"Owner.Field": {"seen": n, "nil": m}, ...}; Owner is the Go struct declaring the field.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/parser"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
)

type stat struct {
	Seen int `json:"seen"`
	Nil  int `json:"nil"`
}

var stats = map[string]*stat{}

var nodePtrType = reflect.TypeOf((*ast.Node)(nil))
var nodeListPtrType = reflect.TypeOf((*ast.NodeList)(nil))
var modListPtrType = reflect.TypeOf((*ast.ModifierList)(nil))

func record(owner string, v reflect.Value) {
	t := v.Type()
	for i := 0; i < t.NumField(); i++ {
		f := t.Field(i)
		fv := v.Field(i)
		if f.Anonymous && f.Type.Kind() == reflect.Struct {
			if f.Type.Name() == "Node" || f.Type.Name() == "NodeBase" || f.Type.Name() == "NodeDefault" {
				continue
			}
			record(f.Type.Name(), fv)
			continue
		}
		if f.Type == nodePtrType || f.Type == nodeListPtrType || f.Type == modListPtrType {
			key := owner + "." + f.Name
			s := stats[key]
			if s == nil {
				s = &stat{}
				stats[key] = s
			}
			s.Seen++
			if fv.IsNil() {
				s.Nil++
			}
		}
	}
}

func visitNode(file *ast.SourceFile, n *ast.Node) {
	data := reflect.ValueOf(n).Elem().FieldByName("data")
	if data.IsValid() && !data.IsNil() {
		ptr := data.Elem()
		if ptr.Kind() == reflect.Pointer && !ptr.IsNil() {
			st := ptr.Elem()
			record(st.Type().Name(), st)
		}
	}
	for _, jsdoc := range n.JSDoc(file) {
		visitNode(file, jsdoc)
	}
	n.ForEachChild(func(c *ast.Node) bool {
		visitNode(file, c)
		return false
	})
}

func parse(fileName string, text string, kind core.ScriptKind) {
	defer func() {
		if r := recover(); r != nil {
			fmt.Fprintf(os.Stderr, "panic parsing %s: %v\n", fileName, r)
		}
	}()
	opts := ast.SourceFileParseOptions{FileName: fileName, Path: tspath.Path(fileName)}
	file := parser.ParseSourceFile(opts, text, kind)
	visitNode(file, file.AsNode())
	for _, c := range file.ReparsedClones {
		visitNode(file, c)
	}
}

func main() {
	roots := os.Args[1:]
	count := 0
	for _, root := range roots {
		filepath.Walk(root, func(p string, info os.FileInfo, err error) error {
			if err != nil || info.IsDir() {
				return nil
			}
			ext := filepath.Ext(p)
			switch ext {
			case ".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs":
			default:
				return nil
			}
			b, err := os.ReadFile(p)
			if err != nil {
				return nil
			}
			text := string(b)
			base := "/x/" + strings.TrimSuffix(filepath.Base(p), ext)
			if strings.HasSuffix(p, ".d.ts") {
				parse(base+".d.ts", text, core.ScriptKindTS)
				return nil
			}
			parse(base+".ts", text, core.ScriptKindTS)
			parse(base+".tsx", text, core.ScriptKindTSX)
			parse(base+".js", text, core.ScriptKindJS)
			count++
			return nil
		})
	}
	fmt.Fprintf(os.Stderr, "parsed %d files\n", count)
	out, _ := json.MarshalIndent(stats, "", "  ")
	os.Stdout.Write(out)
}
