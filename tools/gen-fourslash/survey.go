package main

import (
	"fmt"
	"go/ast"
	"go/types"
	"os"
	"sort"
)

// survey prints the AST node kinds, call targets and interface conversions that appear in the test files, so the
// translator's coverage can be checked against what the tests actually use.
func survey(l *Loaded) {
	nodes := map[string]int{}
	calls := map[string]int{}
	ifaces := map[string]int{}
	for _, p := range append([]*Package{l.Util}, l.Tests...) {
		info := p.Info
		for _, f := range p.Files {
			ast.Inspect(f, func(n ast.Node) bool {
				if n == nil {
					return false
				}
				nodes[fmt.Sprintf("%T", n)]++
				switch n := n.(type) {
				case *ast.CallExpr:
					calls[calleeName(info, n.Fun)]++
					if sig, ok := info.TypeOf(n.Fun).(*types.Signature); ok {
						for i, a := range n.Args {
							var pt types.Type
							if sig.Variadic() && i >= sig.Params().Len()-1 {
								if n.Ellipsis.IsValid() {
									continue
								}
								pt = sig.Params().At(sig.Params().Len() - 1).Type().(*types.Slice).Elem()
							} else if i < sig.Params().Len() {
								pt = sig.Params().At(i).Type()
							} else {
								continue
							}
							noteIface(ifaces, info, pt, a)
						}
					}
				case *ast.CompositeLit:
					t := info.TypeOf(n)
					switch u := t.Underlying().(type) {
					case *types.Slice:
						for _, e := range n.Elts {
							noteIface(ifaces, info, u.Elem(), e)
						}
					case *types.Struct:
						for _, e := range n.Elts {
							if kv, ok := e.(*ast.KeyValueExpr); ok {
								name := kv.Key.(*ast.Ident).Name
								for i := 0; i < u.NumFields(); i++ {
									if u.Field(i).Name() == name {
										noteIface(ifaces, info, u.Field(i).Type(), kv.Value)
									}
								}
							}
						}
					}
				}
				return true
			})
		}
	}
	dump := func(title string, m map[string]int) {
		fmt.Fprintf(os.Stdout, "== %s\n", title)
		var ks []string
		for k := range m {
			ks = append(ks, k)
		}
		sort.Slice(ks, func(i, j int) bool { return m[ks[i]] > m[ks[j]] })
		for _, k := range ks {
			fmt.Fprintf(os.Stdout, "%6d %s\n", m[k], k)
		}
	}
	dump("nodes", nodes)
	dump("calls", calls)
	dump("interface conversions", ifaces)
}

func noteIface(m map[string]int, info *types.Info, want types.Type, e ast.Expr) {
	if !types.IsInterface(want) {
		return
	}
	have := info.TypeOf(e)
	if have == nil {
		return
	}
	m[fmt.Sprintf("%s <- %s", types.TypeString(want, nil), types.TypeString(have, nil))]++
}

func calleeName(info *types.Info, fun ast.Expr) string {
	switch fn := fun.(type) {
	case *ast.Ident:
		if o := info.Uses[fn]; o != nil {
			if o.Pkg() != nil {
				return o.Pkg().Name() + "." + o.Name()
			}
			return o.Name()
		}
		return fn.Name
	case *ast.SelectorExpr:
		if sel := info.Selections[fn]; sel != nil {
			return types.TypeString(sel.Recv(), func(p *types.Package) string { return p.Name() }) + "." + fn.Sel.Name
		}
		if o := info.Uses[fn.Sel]; o != nil && o.Pkg() != nil {
			return o.Pkg().Name() + "." + o.Name()
		}
	case *ast.IndexExpr:
		return calleeName(info, fn.X) + "[]"
	case *ast.ArrayType, *ast.ParenExpr, *ast.StarExpr, *ast.FuncLit:
		return fmt.Sprintf("<%T %s>", fn, types.TypeString(info.TypeOf(fn), nil))
	}
	return fmt.Sprintf("<%T>", fun)
}

func sigs(l *Loaded) {
	seen := map[types.Object]bool{}
	var objs []types.Object
	for _, p := range append([]*Package{l.Util}, l.Tests...) {
		for id, o := range p.Info.Uses {
			_ = id
			if o.Pkg() == nil || o.Pkg().Path() != modPath+"/internal/fourslash" {
				continue
			}
			if !seen[o] {
				seen[o] = true
				objs = append(objs, o)
			}
		}
	}
	sort.Slice(objs, func(i, j int) bool { return objs[i].Pos() < objs[j].Pos() })
	q := func(p *types.Package) string { return p.Name() }
	for _, o := range objs {
		switch o := o.(type) {
		case *types.Func:
			fmt.Println(o.Name(), types.TypeString(o.Type(), q))
		case *types.TypeName:
			fmt.Println("type", o.Name(), types.TypeString(o.Type().Underlying(), q))
		default:
			fmt.Printf("%T %s %s\n", o, o.Name(), types.TypeString(o.Type(), q))
		}
	}
}
