package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"go/types"
	"strings"
	"testing"
)

func checkedGen(t *testing.T, body string) (*gen, *ast.FuncDecl) {
	t.Helper()
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "input.go", "package p\nfunc sink(any) {}\n"+body, 0)
	if err != nil {
		t.Fatal(err)
	}
	info := newInfo()
	if _, err := new(types.Config).Check("p", fset, []*ast.File{file}, info); err != nil {
		t.Fatal(err)
	}
	d := file.Decls[len(file.Decls)-1].(*ast.FuncDecl)
	g := &gen{info: info, vars: map[types.Object]*varInfo{}, assigned: map[types.Object]bool{}}
	for id, obj := range info.Defs {
		if _, ok := obj.(*types.Var); ok {
			g.vars[obj] = &varInfo{name: id.Name, kind: kPlace}
		}
	}
	g.collectLastUses(d.Body)
	return g, d
}

func TestMoveAtLastUse(t *testing.T) {
	for _, test := range []struct {
		name, body, variable string
		want                 []bool
	}{
		{"last use", "func f(x string) { sink(x); sink(x) }", "x", []bool{false, true}},
		{"range input", "func f(xs []string) { for _, x := range xs { sink(x) } }", "xs", []bool{true}},
		{"range binding", "func f(xs []string) { for _, x := range xs { sink(x) } }", "x", []bool{true}},
		{"loop capture", "func f(x string) { for i := 0; i < 2; i++ { sink(x) } }", "x", []bool{false}},
		{"loop local", "func f() { for i := 0; i < 2; i++ { x := \"value\"; sink(x) } }", "x", []bool{true}},
		{"for initializer binding", "func f() { for x := \"value\"; len(x) > 0; { sink(x) } }", "x", []bool{false, false}},
		{"for initializer input", "func f(value string) { for x := value; len(x) > 0; { sink(x) } }", "value", []bool{true}},
		{"closure capture", "func f(x string) { cb := func() { sink(x) }; cb() }", "x", []bool{false}},
		{"closure parameter", "func f() { cb := func(x string) { sink(x) }; cb(\"value\") }", "x", []bool{true}},
	} {
		t.Run(test.name, func(t *testing.T) {
			g, d := checkedGen(t, test.body)
			var got []bool
			ast.Inspect(d.Body, func(n ast.Node) bool {
				if id, ok := n.(*ast.Ident); ok && id.Name == test.variable && g.info.Uses[id] != nil {
					got = append(got, g.canMove(id))
				}
				return true
			})
			if len(got) != len(test.want) {
				t.Fatalf("got %v, want %v", got, test.want)
			}
			for i := range got {
				if got[i] != test.want[i] {
					t.Fatalf("got %v, want %v", got, test.want)
				}
			}
		})
	}
}

func TestCannotMoveThroughPointerField(t *testing.T) {
	g, d := checkedGen(t, "func f() { x := struct { p *struct { values []int } }{}; sink(x.p.values) }")
	call := d.Body.List[1].(*ast.ExprStmt).X.(*ast.CallExpr)
	if g.canMove(call.Args[0]) {
		t.Fatal("a pointer field is borrowed, so its contents cannot move")
	}
}

func TestZeroInitialization(t *testing.T) {
	for _, test := range []struct {
		name, statements string
		want             bool
	}{
		{"both branches", "if condition { x = 1 } else { x = 2 }; sink(x)", true},
		{"direct assignment", "x = 1; sink(x)", true},
		{"one branch", "if condition { x = 1 }; sink(x)", false},
		{"read before assignment", "sink(x); x = 1", false},
		{"read in assignment", "x = x + 1", false},
		{"read in condition", "if x == 0 { x = 1 } else { x = 2 }", false},
		{"address escapes", "sink(&x); x = 1", false},
	} {
		t.Run(test.name, func(t *testing.T) {
			g, d := checkedGen(t, "func f(condition bool) { var x int; "+test.statements+" }")
			decl := d.Body.List[0].(*ast.DeclStmt).Decl.(*ast.GenDecl).Specs[0].(*ast.ValueSpec)
			_, got := g.assignedBeforeRead(g.info.Defs[decl.Names[0]], d.Body.List[1:])
			if got != test.want {
				t.Fatalf("got %v, want %v", got, test.want)
			}
		})
	}
}

func TestFormatCall(t *testing.T) {
	for _, test := range []struct {
		format string
		args   []string
		want   string
	}{
		{"{}?", []string{"name"}, `format!("{name}?")`},
		{"{{x:1}} {}", []string{"name"}, `format!("{{x:1}} {name}")`},
		{"{} {:?}", []string{"name", "value"}, `format!("{name} {value:?}")`},
		{"{} {}", []string{"name", "item.as_str()"}, `format!("{name} {}", item.as_str())`},
		{"{}", []string{"true"}, `format!("{}", true)`},
	} {
		if got := formatCall(test.format, test.args); got != test.want {
			t.Errorf("formatCall(%q, %v) = %s, want %s", test.format, test.args, got, test.want)
		}
	}
}

func TestRustStringDelimiters(t *testing.T) {
	for _, test := range []struct{ input, want string }{
		{"line\n", "r\"line\n\""},
		{`"quoted"`, `r#""quoted""#`},
		{`"#`, `r##""#"##`},
		{strings.Repeat("#", 3) + `"`, `r#"###""#`},
		{"first \nlast", `"first \nlast"`},
		{"<<<<<<< HEAD\n=======\n>>>>>>> branch", `"<<<<<<< HEAD\n=======\n>>>>>>> branch"`},
	} {
		got, ok := rustStr(test.input)
		if !ok || got != test.want {
			t.Errorf("rustStr(%q) = %s, %v; want %s", test.input, got, ok, test.want)
		}
	}
}

func TestLiteralFormattingFixture(t *testing.T) {
	g, d := checkedGen(t, `func f() { sink([]string{"const value={a:1};", "no braces"}) }`)
	if !g.literalFormattingFixture(d.Body.List[0]) {
		t.Fatal("a later argument must not erase an earlier fixture match")
	}
}

func TestNegatedBoolKeepsNilGuardFirst(t *testing.T) {
	g, d := checkedGen(t, "func f(value any) bool { return !(value != nil && value.(bool)) }")
	e := d.Body.List[0].(*ast.ReturnStmt).Results[0].(*ast.UnaryExpr)
	got := g.negatedBool(e.X)
	if want := "value.is_nil() || (!value.assert_bool())"; got != want {
		t.Fatalf("got %s, want %s", got, want)
	}
}
