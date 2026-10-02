package main

import (
	"encoding/json"
	"go/ast"
	"go/constant"
	"go/types"
	"os"
	"sort"
	"strconv"
	"strings"
	"unicode"
)

// parserInputs writes, for every test whose fourslash content is a constant, the content and the file name
// fourslash.ParseTestData receives (JSON lines; input of tools/oracle/fourslash-parser).
func parserInputs(l *Loaded, out string) error {
	type input struct {
		Test     string `json:"test"`
		FileName string `json:"fileName"`
		Content  string `json:"content"`
	}
	var inputs []input
	for _, p := range l.Tests {
		for _, f := range p.Files {
			for _, d := range f.Decls {
				fd, ok := d.(*ast.FuncDecl)
				if !ok || !strings.HasPrefix(fd.Name.Name, "Test") || fd.Body == nil {
					continue
				}
				n := 0
				ast.Inspect(fd.Body, func(node ast.Node) bool {
					call, ok := node.(*ast.CallExpr)
					if !ok {
						return true
					}
					sel, ok := call.Fun.(*ast.SelectorExpr)
					if !ok {
						return true
					}
					obj, ok := p.Info.Uses[sel.Sel].(*types.Func)
					if !ok || obj.Pkg() == nil || obj.Pkg().Path() != fsPath {
						return true
					}
					var arg ast.Expr
					switch obj.Name() {
					case "NewFourslash":
						arg = call.Args[2]
					case "NewFourslashWithOptions":
						arg = call.Args[1]
					default:
						return true
					}
					tv := p.Info.Types[arg]
					if tv.Value == nil || tv.Value.Kind() != constant.String {
						return true
					}
					name := fd.Name.Name
					if n > 0 {
						name += "#" + strconv.Itoa(n)
					}
					n++
					inputs = append(inputs, input{Test: name, FileName: baseFileNameFromTest(fd.Name.Name) + ".ts", Content: constant.StringVal(tv.Value)})
					return true
				})
			}
		}
	}
	sort.Slice(inputs, func(i, j int) bool { return inputs[i].Test < inputs[j].Test })
	fh, err := os.Create(out)
	if err != nil {
		return err
	}
	defer fh.Close()
	enc := json.NewEncoder(fh)
	for _, in := range inputs {
		if err := enc.Encode(in); err != nil {
			return err
		}
	}
	return nil
}

// fourslash.getBaseFileNameFromTest for a top-level test.
func baseFileNameFromTest(name string) string {
	name = strings.TrimPrefix(name, "Test")
	if name != "" {
		r := []rune(name)
		r[0] = unicode.ToLower(r[0])
		name = string(r)
	}
	switch name {
	case "callHierarchyFunctionAmbiguity1", "callHierarchyFunctionAmbiguity2", "callHierarchyFunctionAmbiguity3",
		"callHierarchyFunctionAmbiguity4", "callHierarchyFunctionAmbiguity5":
		name = name[:len(name)-1] + "." + name[len(name)-1:]
	}
	return name
}
