package main

import (
	"go/ast"
	"go/constant"
	"go/types"
	"strings"
)

// callExpr translates a call; the result is an owned value.
func (g *gen) callExpr(e *ast.CallExpr, m mode) string {
	if tv := g.info.Types[e.Fun]; tv.IsType() {
		return g.conversion(e, tv.Type)
	}
	if id, ok := e.Fun.(*ast.Ident); ok {
		if b, ok := g.info.Uses[id].(*types.Builtin); ok {
			return g.builtin(b.Name(), e)
		}
	}
	switch fun := e.Fun.(type) {
	case *ast.SelectorExpr:
		if sel := g.info.Selections[fun]; sel != nil {
			switch sel.Kind() {
			case types.MethodVal:
				return g.methodCall(e, fun, sel)
			case types.FieldVal:
				// a func-valued field: harness closures get (f, t) like DoneFn
				sig := sel.Obj().Type().(*types.Signature)
				recv, _ := g.natural(fun.X)
				args := g.args(e, sig, false)
				return "(" + recv + "." + ident(fun.Sel.Name) + ")(" + strings.Join(append(g.fArgs(sig, ""), args...), ", ") + ")"
			}
			fail("call of %s", fun.Sel.Name)
		}
		obj := g.info.Uses[fun.Sel]
		if f, ok := obj.(*types.Func); ok {
			return g.qualifiedCall(e, f)
		}
		fail("call of %s", fun.Sel.Name)
	case *ast.Ident:
		obj := g.info.Uses[fun]
		switch o := obj.(type) {
		case *types.Func:
			if o.Pkg() != nil && o.Pkg().Path() != g.p.Path {
				return g.qualifiedCall(e, o)
			}
			sig := o.Type().(*types.Signature)
			return pkgRef(o, ident(o.Name())) + "(" + strings.Join(g.args(e, sig, false), ", ") + ")"
		case *types.Var:
			vi := g.vars[o]
			if vi == nil {
				fail("call of unknown variable %s", fun.Name)
			}
			sig := o.Type().(*types.Signature)
			args := g.args(e, sig, false)
			switch {
			case vi.fclosure:
				return vi.name + "(" + strings.Join(append(g.fArgs(sig, vi.fOwner), args...), ", ") + ")"
			case vi.takesF:
				if g.curF == "" {
					fail("closure needs the FourslashTest but none is in scope")
				}
				return vi.name + "(" + strings.Join(append([]string{"&mut *" + g.curF}, args...), ", ") + ")"
			}
			return vi.name + "(" + strings.Join(args, ", ") + ")"
		}
	case *ast.FuncLit:
		fail("immediately invoked function literal")
	}
	fail("call of %T", e.Fun)
	return ""
}

// fArgs: harness closures (DoneFn, VerifyCompletionsResult fields) take the FourslashTest and, if their Go
// signature has no *testing.T, the T.
func (g *gen) fArgs(sig *types.Signature, owner string) []string {
	if owner == "" {
		owner = g.curF
	}
	if owner == "" {
		fail("harness closure called without a FourslashTest in scope")
	}
	out := []string{"&mut *" + owner}
	hasT := sig.Params().Len() > 0 && isTestingTPtr(sig.Params().At(0).Type())
	if !hasT {
		if g.curT == "" {
			fail("harness closure called without a T in scope")
		}
		out = append(out, g.curT)
	}
	return out
}

// args translates call arguments for signature sig. harness: function literals receive the FourslashTest first.
func (g *gen) args(e *ast.CallExpr, sig *types.Signature, harness bool) []string {
	params := sig.Params()
	var out []string
	n := params.Len()
	for i, a := range e.Args {
		if sig.Variadic() && i >= n-1 {
			st := params.At(n - 1).Type().(*types.Slice)
			if e.Ellipsis.IsValid() {
				out = append(out, g.expr(a, st, mParam))
				break
			}
			var parts []string
			for _, va := range e.Args[n-1:] {
				if isString(st.Elem()) {
					parts = append(parts, g.expr(va, st.Elem(), mParam))
				} else {
					parts = append(parts, g.elem(va, st.Elem()))
				}
			}
			out = append(out, "&["+strings.Join(parts, ", ")+"]")
			break
		}
		pt := params.At(i).Type()
		if fl, ok := a.(*ast.FuncLit); ok {
			out = append(out, g.funcLit(fl, harness, harness))
			continue
		}
		out = append(out, g.expr(a, pt, mParam))
	}
	if sig.Variadic() && len(e.Args) == n-1 {
		out = append(out, "&[]")
	}
	return out
}

func (g *gen) methodCall(e *ast.CallExpr, fun *ast.SelectorExpr, sel *types.Selection) string {
	recvT := sel.Recv()
	name := fun.Sel.Name
	sig := sel.Obj().Type().(*types.Signature)
	recv, rk := g.natural(fun.X)
	if rk == kOptArc {
		recv = parenIfNeeded(recv) + ".as_ref().unwrap()"
	} else if isOptionPtr(recvT) && rk != kDeref {
		recv = parenIfNeeded(recv) + ".as_ref().unwrap()"
	} else {
		recv = parenIfNeeded(recv)
	}
	switch {
	case isTestingTPtr(recvT):
		switch name {
		case "Parallel", "Helper":
			return recv + "." + ident(name) + "()"
		case "Name":
			return recv + ".name()"
		case "Skip", "Fatal", "Error", "Log":
			return recv + "." + ident(name) + "(&" + g.sprint(e.Args) + ")"
		case "Fatalf", "Errorf", "Logf", "Skipf":
			return recv + "." + ident(strings.TrimSuffix(name, "f")) + "(&" + g.sprintf(e.Args) + ")"
		case "Run":
			fl, ok := e.Args[1].(*ast.FuncLit)
			if !ok {
				fail("t.Run with a non-literal function")
			}
			return recv + ".run(" + g.expr(e.Args[0], types.Typ[types.String], mParam) + ", " + g.funcLit(fl, false, false) + ")"
		}
		fail("testing.T.%s", name)
	case isNamed(derefType(recvT), "strings", "Builder"):
		switch name {
		case "WriteString":
			return recv + ".push_str(" + g.expr(e.Args[0], types.Typ[types.String], mParam) + ")"
		case "String":
			return recv + ".clone()"
		}
		fail("strings.Builder.%s", name)
	}
	if isNamed(derefType(recvT), collectionsPath, "MultiMap") {
		var args []string
		for i, a := range e.Args {
			args = append(args, "&"+parenIfNeeded(g.expr(a, sig.Params().At(i).Type(), mOwned)))
		}
		return recv + "." + ident(name) + "(" + strings.Join(args, ", ") + ")"
	}
	harness := false
	if p := sel.Obj().Pkg(); p != nil && p.Path() == fsPath {
		harness = true
	}
	args := g.args(e, sig, harness)
	if isFourslashTestPtr(recvT) && g.curF != "" {
		// Arguments that pass the FourslashTest to a closure are evaluated before the receiver is borrowed.
		hoist := false
		for _, a := range args {
			if strings.Contains(a, "&mut *"+g.curF) {
				hoist = true
			}
		}
		if hoist {
			var b strings.Builder
			b.WriteString("{ ")
			var names []string
			for i, a := range args {
				n := sprintf("__arg%d", i)
				names = append(names, n)
				b.WriteString("let " + n + " = " + a + "; ")
			}
			b.WriteString(recv + "." + ident(name) + "(" + strings.Join(names, ", ") + ") }")
			return b.String()
		}
	}
	return recv + "." + ident(name) + "(" + strings.Join(args, ", ") + ")"
}

func derefType(t types.Type) types.Type {
	if p, ok := types.Unalias(t).(*types.Pointer); ok {
		return p.Elem()
	}
	return t
}

func (g *gen) qualifiedCall(e *ast.CallExpr, f *types.Func) string {
	path := f.Pkg().Path()
	name := f.Name()
	sig := f.Type().(*types.Signature)
	switch path {
	case "fmt":
		if name == "Sprintf" {
			return g.sprintf(e.Args)
		}
		if name == "Sprint" {
			return g.sprint(e.Args)
		}
	case "strconv":
		if name == "Quote" {
			return "go::quote(" + g.expr(e.Args[0], types.Typ[types.String], mParam) + ")"
		}
		if name == "Itoa" {
			return g.expr(e.Args[0], types.Typ[types.Int], mOwned) + ".to_string()"
		}
	case "strings":
		return "go::strings::" + ident(name) + "(" + strings.Join(g.args(e, sig, false), ", ") + ")"
	case "slices":
		switch name {
		case "Concat":
			var parts []string
			for _, a := range e.Args {
				parts = append(parts, g.sliceView(a))
			}
			return "[" + strings.Join(parts, ", ") + "].concat()"
		case "Clone":
			return g.expr(e.Args[0], g.typeOf(e.Args[0]), mOwned)
		}
	case corePath:
		switch name {
		case "IfElse":
			rt := g.typeOf(e)
			return "if " + g.expr(e.Args[0], types.Typ[types.Bool], mOwned) + " { " + g.expr(e.Args[1], rt, mOwned) + " } else { " + g.expr(e.Args[2], rt, mOwned) + " }"
		case "Filter":
			fl, ok := e.Args[1].(*ast.FuncLit)
			if !ok {
				fail("core.Filter with a non-literal function")
			}
			return "go::filter(" + g.sliceView(e.Args[0]) + ", " + g.refClosure(fl) + ")"
		case "Map":
			fl, ok := e.Args[1].(*ast.FuncLit)
			if !ok {
				fail("core.Map with a non-literal function")
			}
			return "go::map(" + g.sliceView(e.Args[0]) + ", " + g.refClosure(fl) + ")"
		case "OrElse":
			rt := g.typeOf(e)
			return "go::or_else(" + g.expr(e.Args[0], rt, mOwned) + ", " + g.expr(e.Args[1], rt, mOwned) + ")"
		}
	case "gotest.tools/v3/assert":
		var parts []string
		parts = append(parts, g.expr(e.Args[0], g.typeOf(e.Args[0]), mParam))
		switch name {
		case "Equal", "DeepEqual":
			parts = append(parts, g.expr(e.Args[1], g.typeOf(e.Args[1]), mOwned), g.expr(e.Args[2], g.typeOf(e.Args[1]), mOwned))
			rest := e.Args[3:]
			parts = append(parts, "&"+g.sprint(rest))
			return "go::assert::" + ident(name) + "(" + strings.Join(parts, ", ") + ")"
		case "Assert", "Check":
			parts = append(parts, g.expr(e.Args[1], types.Typ[types.Bool], mOwned))
			parts = append(parts, "&"+g.sprint(e.Args[2:]))
			return "go::assert::" + ident(name) + "(" + strings.Join(parts, ", ") + ")"
		}
	case lsutilPath:
		if name == "ParseUserPreferences" {
			return "lsutil::parse_user_preferences(&go::json_object(&" + parenIfNeeded(g.expr(e.Args[0], g.typeOf(e.Args[0]), mOwned)) + "))"
		}
	case utilPath:
		if name == "ToAny" {
			return "util::to_any(" + g.sliceView(e.Args[0]) + ")"
		}
	case testutilPath:
		fail("testutil.%s outside defer", name)
	}
	mod := pkgModule[path]
	if mod == "" {
		fail("call of %s.%s", path, name)
	}
	if sig.TypeParams().Len() > 0 {
		fail("generic call %s.%s", path, name)
	}
	return mod + "::" + ident(name) + "(" + strings.Join(g.args(e, sig, path == fsPath), ", ") + ")"
}

// sliceView: a slice expression as `&[T]`.
func (g *gen) sliceView(e ast.Expr) string {
	if g.isNil(e) {
		return "&[]"
	}
	c, k := g.natural(e)
	switch k {
	case kSlice:
		return c
	case kStrSlice:
		return "&go::owned_strs(" + c + ")[..]"
	}
	return "&" + parenIfNeeded(c) + "[..]"
}

// refClosure translates a function literal whose parameters are passed by reference (core.Filter, core.Map).
func (g *gen) refClosure(fl *ast.FuncLit) string {
	sig := g.typeOf(fl).(*types.Signature)
	var params []string
	i := 0
	for _, field := range fl.Type.Params.List {
		for _, n := range field.Names {
			v := sig.Params().At(i)
			i++
			name := ident(n.Name)
			if obj := g.info.Defs[n]; obj != nil {
				g.vars[obj] = &varInfo{name: name, kind: kRef}
			}
			params = append(params, name+": &"+elemType(v.Type()))
		}
	}
	return "|" + strings.Join(params, ", ") + "|" + g.closureRet(sig) + " " + g.closureBody(fl, sig)
}

func (g *gen) closureRet(sig *types.Signature) string {
	r := sig.Results()
	switch r.Len() {
	case 0:
		return ""
	case 1:
		return " -> " + g.resultType(r.At(0).Type())
	}
	var ts []string
	for i := 0; i < r.Len(); i++ {
		ts = append(ts, g.resultType(r.At(i).Type()))
	}
	return " -> (" + strings.Join(ts, ", ") + ")"
}

func (g *gen) closureBody(fl *ast.FuncLit, sig *types.Signature) string {
	savedBuf, savedIndent := g.buf, g.indent
	savedRes, savedInRes := g.resultTypes, g.inResultFunc
	scope := g.saveScope()
	var b strings.Builder
	g.buf = &b
	g.resultTypes = nil
	for i := 0; i < sig.Results().Len(); i++ {
		g.resultTypes = append(g.resultTypes, sig.Results().At(i).Type())
	}
	g.inResultFunc = sig.Results().Len() > 0
	g.indent = savedIndent + 1
	g.stmts(fl.Body.List)
	g.indent = savedIndent
	g.buf, g.resultTypes, g.inResultFunc = savedBuf, savedRes, savedInRes
	g.restoreScope(scope)
	return "{\n" + b.String() + strings.Repeat("    ", savedIndent) + "}"
}

// funcLit translates a closure. takesF: the FourslashTest is passed as the first parameter (named like the
// enclosing one, shadowing it).
func (g *gen) funcLit(fl *ast.FuncLit, takesF bool, harness bool) string {
	sig := g.typeOf(fl).(*types.Signature)
	scope := g.saveScope()
	var params []string
	fName := g.curF
	if fName == "" {
		fName = "f"
	}
	if takesF {
		params = append(params, fName+": &mut fourslash::FourslashTest")
	}
	ps, prologue := g.params(sig, fl.Type.Params, true)
	params = append(params, ps...)
	g.curF = fName
	if !takesF && harness {
		g.curF = scope.curF
	}
	body := g.closureBodyWithPrologue(fl, sig, prologue)
	g.restoreScope(scope)
	return "|" + strings.Join(params, ", ") + "|" + g.closureRet(sig) + " " + body
}

func (g *gen) closureBodyWithPrologue(fl *ast.FuncLit, sig *types.Signature, prologue []string) string {
	if len(prologue) == 0 {
		return g.closureBody(fl, sig)
	}
	body := g.closureBody(fl, sig)
	pre := ""
	for _, p := range prologue {
		pre += strings.Repeat("    ", g.indent+1) + p + "\n"
	}
	return "{\n" + pre + strings.TrimPrefix(body, "{\n")
}

func (g *gen) builtin(name string, e *ast.CallExpr) string {
	switch name {
	case "len":
		c, k := g.natural(e.Args[0])
		_ = k
		return parenIfNeeded(c) + ".len() as i32"
	case "new":
		inner, _ := g.ptrLiteral(e)
		if isArcPtr(g.typeOf(e)) {
			return "Arc::new(" + inner + ")"
		}
		return "Some(" + inner + ")"
	case "append":
		st, _ := isSlice(g.typeOf(e))
		base := g.expr(e.Args[0], g.typeOf(e.Args[0]), mOwned)
		if e.Ellipsis.IsValid() {
			return "go::append(" + base + ", " + g.sliceView(e.Args[1]) + ")"
		}
		var parts []string
		for _, a := range e.Args[1:] {
			parts = append(parts, g.elem(a, st.Elem()))
		}
		return "go::append(" + base + ", &[" + strings.Join(parts, ", ") + "])"
	case "make":
		t := g.info.Types[e.Args[0]].Type
		switch types.Unalias(t).Underlying().(type) {
		case *types.Slice:
			if len(e.Args) >= 2 {
				return "vec![Default::default(); " + g.index(e.Args[1]) + "]"
			}
			return "Vec::new()"
		case *types.Map:
			return "OrderedMap::default()"
		}
		fail("make(%s)", types.TypeString(t, nil))
	case "panic":
		return "panic!(\"{}\", " + g.expr(e.Args[0], g.typeOf(e.Args[0]), mParam) + ")"
	}
	fail("builtin %s", name)
	return ""
}

func (g *gen) conversion(e *ast.CallExpr, to types.Type) string {
	arg := e.Args[0]
	if tv := g.info.Types[e]; tv.Value != nil {
		lit := g.constLit(tv.Value, to)
		if isString(to) {
			return lit + ".to_string()"
		}
		return lit
	}
	from := g.typeOf(arg)
	switch {
	case isString(to) && stringNewtype(from):
		c, _ := g.natural(arg)
		return parenIfNeeded(c) + ".0.to_string()"
	case isString(to) && isString(from):
		return g.expr(arg, from, mOwned)
	case isString(to) && isByteSlice(from):
		return "String::from_utf8(" + g.expr(arg, from, mOwned) + ").unwrap()"

	case rustEnumType(to) && isString(from):
		n := types.Unalias(to).(*types.Named)
		return "go::conv::" + ident(n.Obj().Name()) + "(" + g.expr(arg, from, mParam) + ")"
	}
	if tb, ok := types.Unalias(to).Underlying().(*types.Basic); ok && tb.Info()&types.IsNumeric != 0 {
		if fb, ok := types.Unalias(from).Underlying().(*types.Basic); ok && fb.Info()&types.IsNumeric != 0 {
			return parenIfNeeded(g.expr(arg, from, mOwned)) + " as " + rtype(to, false)
		}
	}
	fail("conversion %s -> %s", types.TypeString(from, nil), types.TypeString(to, nil))
	return ""
}

// sprintf translates fmt.Sprintf (and the f-variants of testing.T) to format!.
func (g *gen) sprintf(args []ast.Expr) string {
	tv := g.info.Types[args[0]]
	if tv.Value == nil {
		fail("non-constant format string")
	}
	f := constant.StringVal(tv.Value)
	var out strings.Builder
	ai := 1
	var rargs []string
	for i := 0; i < len(f); i++ {
		c := f[i]
		switch c {
		case '{':
			out.WriteString("{{")
			continue
		case '}':
			out.WriteString("}}")
			continue
		case '%':
		default:
			out.WriteByte(c)
			continue
		}
		i++
		if i >= len(f) {
			fail("bad format")
		}
		verb := f[i]
		if verb == '%' {
			out.WriteByte('%')
			continue
		}
		if ai >= len(args) {
			fail("format: missing argument")
		}
		a := args[ai]
		ai++
		at := g.typeOf(a)
		switch verb {
		case '#':
			if i+1 < len(f) && f[i+1] == 'v' {
				i++
				out.WriteString("{:?}")
				rargs = append(rargs, g.expr(a, at, mParam))
			} else {
				fail("format verb %%#%c", f[i+1])
			}
		case 's', 'v', 'd', 't':
			out.WriteString("{}")
			rargs = append(rargs, g.fmtArg(a, at))
		case 'q':
			out.WriteString("{}")
			rargs = append(rargs, "go::quote("+g.expr(a, at, mParam)+")")
		default:
			fail("format verb %%%c", verb)
		}
	}
	lit, ok := rustStr(out.String())
	if !ok {
		fail("non-UTF-8 format")
	}
	if len(rargs) == 0 {
		return "format!(" + lit + ")"
	}
	return "format!(" + lit + ", " + strings.Join(rargs, ", ") + ")"
}

func (g *gen) fmtArg(a ast.Expr, at types.Type) string {
	if isString(at) {
		return g.expr(a, at, mParam)
	}
	if b, ok := types.Unalias(at).Underlying().(*types.Basic); ok && b.Info()&(types.IsInteger|types.IsBoolean|types.IsFloat) != 0 {
		return g.expr(a, at, mOwned)
	}
	return "go::GoFmt(&" + parenIfNeeded(g.expr(a, at, mOwned)) + ")"
}

// sprint: fmt.Sprint-like concatenation of arguments (t.Fatal, t.Skip, assert messages).
func (g *gen) sprint(args []ast.Expr) string {
	if len(args) == 0 {
		return "String::new()"
	}
	if len(args) == 1 {
		at := g.typeOf(args[0])
		if isString(at) {
			return g.expr(args[0], at, mOwned)
		}
	}
	var fs []string
	var rargs []string
	for _, a := range args {
		fs = append(fs, "{}")
		rargs = append(rargs, g.fmtArg(a, g.typeOf(a)))
	}
	// fmt.Sprint adds spaces between operands when neither is a string; the tests only pass strings.
	return "format!(\"" + strings.Join(fs, "") + "\", " + strings.Join(rargs, ", ") + ")"
}

func isByteSlice(t types.Type) bool {
	s, ok := isSlice(t)
	if !ok {
		return false
	}
	b, ok := types.Unalias(s.Elem()).(*types.Basic)
	return ok && b.Kind() == types.Uint8
}
