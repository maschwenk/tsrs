package main

import (
	"go/ast"
	"go/constant"
	"go/token"
	"go/types"
	"strconv"
	"strings"
)

type mode int

const (
	mOwned mode = iota // an owned value of rtype(want)
	mParam             // a function argument for a parameter of Go type want (string -> &str, slice -> &[_])
	mPlace             // a place / receiver (no clone)
	mStmt              // statement position
)

func unquote(s string) (string, error) { return strconv.Unquote(s) }

func (g *gen) typeOf(e ast.Expr) types.Type { return g.info.TypeOf(e) }

// expr translates e for a destination of Go type want (nil: e's own type).
func (g *gen) expr(e ast.Expr, want types.Type, m mode) string {
	have := g.typeOf(e)
	if want == nil {
		want = have
	}
	if p, ok := e.(*ast.ParenExpr); ok {
		return g.expr(p.X, want, m)
	}
	if g.isNil(e) {
		return g.nilValue(want, m)
	}
	if isInterface(want) && have != nil && !isInterface(have) {
		return g.wrapInterface(e, want)
	}
	if isOptionPtr(want) {
		if inner, ok := g.ptrLiteral(e); ok {
			return "Some(" + inner + ")"
		}
	}
	if isArcPtr(want) {
		if inner, ok := g.ptrLiteral(e); ok {
			return "Arc::new(" + inner + ")"
		}
	}
	if m == mParam {
		if st, ok := isSlice(want); ok {
			if cl, ok := e.(*ast.CompositeLit); ok {
				var parts []string
				for _, el := range cl.Elts {
					if isString(st.Elem()) {
						parts = append(parts, g.expr(el, st.Elem(), mParam))
					} else {
						parts = append(parts, g.elem(el, st.Elem()))
					}
				}
				return "&[" + strings.Join(parts, ", ") + "]"
			}
		}
	}
	if tv := g.info.Types[e]; tv.Value != nil && !g.isNamedConstRef(e) {
		t := want
		if isInterface(t) || t == nil {
			t = tv.Type
		}
		lit := g.constLit(tv.Value, t)
		if isString(t) {
			if m == mOwned {
				return lit + ".to_string()"
			}
		}
		return lit
	}
	code, k := g.natural(e)
	return g.coerce(code, k, have, want, m)
}

// elem translates a slice element / interface payload: `*T` elements are plain `T` values.
func (g *gen) elem(e ast.Expr, et types.Type) string {
	if isOptionPtr(et) {
		if inner, ok := g.ptrLiteral(e); ok {
			return inner
		}
		return parenIfNeeded(g.expr(e, et, mOwned)) + ".unwrap()"
	}
	return g.expr(e, et, mOwned)
}

func (g *gen) isNil(e ast.Expr) bool {
	tv, ok := g.info.Types[e]
	return ok && tv.IsNil()
}

func (g *gen) isNamedConstRef(e ast.Expr) bool {
	switch e := e.(type) {
	case *ast.Ident:
		_, ok := g.info.Uses[e].(*types.Const)
		return ok
	case *ast.SelectorExpr:
		_, ok := g.info.Uses[e.Sel].(*types.Const)
		return ok
	}
	return false
}

func (g *gen) nilValue(want types.Type, m mode) string {
	if want == nil {
		fail("nil without type")
	}
	if isAny(want) {
		return "Any::Nil"
	}
	if isNamed(want, cmPath, "Spawner") {
		return "None"
	}
	switch u := types.Unalias(want).Underlying().(type) {
	case *types.Pointer:
		if isOptionPtr(want) {
			return "None"
		}
	case *types.Slice:
		if m == mParam {
			return "&[]"
		}
		return "Vec::new()"
	case *types.Map:
		return "OrderedMap::default()"
	case *types.Signature:
		_ = u
		return "None"
	}
	fail("nil of type %s", types.TypeString(want, nil))
	return ""
}

// ptrLiteral recognizes `&T{...}`, `new(x)`, `new(T)` and elided `{...}` of pointer type and returns the pointee.
func (g *gen) ptrLiteral(e ast.Expr) (string, bool) {
	switch e := e.(type) {
	case *ast.ParenExpr:
		return g.ptrLiteral(e.X)
	case *ast.UnaryExpr:
		if e.Op == token.AND {
			return g.expr(e.X, g.typeOf(e.X), mOwned), true
		}
	case *ast.CallExpr:
		if g.isBuiltin(e.Fun, "new") {
			if tv := g.info.Types[e.Args[0]]; tv.IsType() {
				return rtype(tv.Type, false) + "::default()", true
			}
			pt := types.Unalias(g.typeOf(e)).(*types.Pointer)
			return g.expr(e.Args[0], pt.Elem(), mOwned), true
		}
	case *ast.CompositeLit:
		if p, ok := types.Unalias(g.typeOf(e)).(*types.Pointer); ok {
			return g.compositeLit(e, p.Elem()), true
		}
	}
	return "", false
}

// wrapInterface converts a concrete value to the Rust enum for a Go interface type.
func (g *gen) wrapInterface(e ast.Expr, want types.Type) string {
	have := g.typeOf(e)
	if isNamed(want, fsPath, "MarkerOrRange") {
		switch {
		case isPtrTo(have, fsPath, "Marker"):
			return "fourslash::MarkerOrRange::Marker(" + g.expr(e, have, mOwned) + ")"
		case isPtrTo(have, fsPath, "RangeMarker"):
			return "fourslash::MarkerOrRange::RangeMarker(" + g.expr(e, have, mOwned) + ")"
		}
		fail("MarkerOrRange from %s", types.TypeString(have, nil))
	}
	if isNamed(want, cmPath, "Spawner") {
		return "Some(" + g.expr(e, have, mOwned) + ")"
	}
	if !isAny(want) {
		fail("conversion to interface %s", types.TypeString(want, nil))
	}
	v, payloadT := anyVariant(have)
	if v == "" {
		fail("any from %s", types.TypeString(have, nil))
	}
	g.anyVariants[v]++
	if payloadT == nil {
		return "Any::" + v
	}
	if p, ok := types.Unalias(have).(*types.Pointer); ok && isOptionPtr(have) {
		return "Any::" + v + "(" + g.elem(e, p) + ")"
	}
	return "Any::" + v + "(" + g.expr(e, payloadT, mOwned) + ")"
}

func isPtrTo(t types.Type, pkg, name string) bool {
	p, ok := types.Unalias(t).(*types.Pointer)
	return ok && isNamed(p.Elem(), pkg, name)
}

// anyVariant names the `Any` variant for a Go dynamic type.
func anyVariant(t types.Type) (string, types.Type) {
	t = types.Unalias(t)
	switch {
	case isPlainString(t):
		return "String", types.Typ[types.String]
	case isPtrTo(t, fsPath, "Marker"):
		return "Marker", t
	case isPtrTo(t, fsPath, "RangeMarker"):
		return "RangeMarker", t
	case isPtrTo(t, fsPath, "EditRange"):
		return "EditRange", t
	case isPtrTo(t, lsprotoPath, "CompletionItem"):
		return "CompletionItem", t
	}
	if b, ok := t.(*types.Basic); ok {
		switch b.Kind() {
		case types.Bool, types.UntypedBool:
			return "Bool", types.Typ[types.Bool]
		case types.Int, types.UntypedInt:
			return "Int", types.Typ[types.Int]
		case types.UntypedString:
			return "String", types.Typ[types.String]
		}
	}
	if s, ok := t.(*types.Struct); ok && s.NumFields() == 0 {
		return "Ignored", nil
	}
	if m, ok := t.(*types.Map); ok && isPlainString(m.Key()) && isAny(m.Elem()) {
		return "Map", t
	}
	if s, ok := t.(*types.Slice); ok {
		switch {
		case isPlainString(s.Elem()):
			return "StringSlice", t
		case isPtrTo(s.Elem(), fsPath, "Marker"):
			return "MarkerSlice", t
		}
	}
	return "", nil
}

// coerce adapts a translated expression of kind k to mode m.
func (g *gen) coerce(code string, k kind, have, want types.Type, m mode) string {
	t := want
	if t == nil {
		t = have
	}
	switch m {
	case mPlace, mStmt:
		return code
	case mParam:
		switch {
		case isFourslashTestPtr(t):
			return code
		case isString(t):
			switch k {
			case kStr, kRef:
				return code
			case kOptArc:
				fail("string from Option<Arc>")
			}
			return "&" + parenIfNeeded(code)
		case isStringSlice(t):
			switch k {
			case kStrSlice:
				return code
			case kSlice, kRef:
				return "&go::strs(" + code + ")"
			}
			return "&go::strs(&" + parenIfNeeded(code) + ")"
		case isSliceT(t):
			switch k {
			case kSlice, kRef:
				return code
			}
			return "&" + parenIfNeeded(code)
		}
	}
	// mOwned (and by-value parameters)
	switch k {
	case kOwned:
		return code
	case kPlace:
		if copyType(t) {
			return code
		}
		return parenIfNeeded(code) + ".clone()"
	case kStr:
		return parenIfNeeded(code) + ".to_string()"
	case kSlice:
		return parenIfNeeded(code) + ".to_vec()"
	case kStrSlice:
		return "go::owned_strs(" + code + ")"
	case kRef:
		if copyType(t) {
			return "*" + parenIfNeeded(code)
		}
		return parenIfNeeded(code) + ".clone()"
	case kOptArc:
		if isArcPtr(t) {
			return parenIfNeeded(code) + ".clone().unwrap()"
		}
		return parenIfNeeded(code) + ".clone()"
	}
	fail("coerce kind %d", k)
	return ""
}

// parenIfNeeded wraps code in parentheses unless it is a postfix expression (path, call, field, index, literal).
func parenIfNeeded(code string) string {
	depth := 0
	inStr := false
	for i := 0; i < len(code); i++ {
		c := code[i]
		if inStr {
			if c == '\\' {
				i++
			} else if c == '"' {
				inStr = false
			}
			continue
		}
		switch c {
		case '"':
			if i > 0 && code[i-1] == 'r' || i > 0 && code[i-1] == '#' {
				return "(" + code + ")"
			}
			inStr = true
		case '(', '[', '{':
			depth++
		case ')', ']', '}':
			depth--
		case ' ', '+', '-', '*', '/', '=', '<', '>', '|', '!', '?':
			if depth == 0 {
				if c == '!' && i+1 < len(code) && code[i+1] == '(' && i > 0 {
					continue // macro call
				}
				return "(" + code + ")"
			}
		case '&':
			if depth == 0 {
				return "(" + code + ")"
			}
		}
	}
	return code
}

// natural translates e into its natural Rust form.
func (g *gen) natural(e ast.Expr) (string, kind) {
	switch e := e.(type) {
	case *ast.ParenExpr:
		c, k := g.natural(e.X)
		return "(" + c + ")", k
	case *ast.Ident:
		return g.identExpr(e)
	case *ast.SelectorExpr:
		return g.selectorExpr(e)
	case *ast.CallExpr:
		return g.callExpr(e, mOwned), kOwned
	case *ast.CompositeLit:
		t := g.typeOf(e)
		if p, ok := types.Unalias(t).(*types.Pointer); ok {
			lit := g.compositeLit(e, p.Elem())
			if harnessObject(p.Elem()) {
				return "Arc::new(" + lit + ")", kOwned
			}
			return "Some(" + lit + ")", kOwned
		}
		return g.compositeLit(e, t), kOwned
	case *ast.UnaryExpr:
		switch e.Op {
		case token.NOT:
			return "!" + parenIfNeeded(g.expr(e.X, types.Typ[types.Bool], mOwned)), kOwned
		case token.SUB:
			return "-" + parenIfNeeded(g.expr(e.X, g.typeOf(e.X), mOwned)), kOwned
		case token.AND:
			inner, _ := g.ptrLiteral(e)
			if isArcPtr(g.typeOf(e)) {
				return "Arc::new(" + inner + ")", kOwned
			}
			return "Some(" + inner + ")", kOwned
		}
		fail("unary %s", e.Op)
	case *ast.BinaryExpr:
		return g.binaryExpr(e)
	case *ast.IndexExpr:
		return g.indexExpr(e)
	case *ast.SliceExpr:
		return g.sliceExpr(e)
	case *ast.StarExpr:
		t := g.typeOf(e.X)
		if isOptionPtr(t) {
			c, _ := g.natural(e.X)
			return parenIfNeeded(c) + ".as_ref().unwrap()", kRef
		}
		fail("dereference of %s", types.TypeString(t, nil))
	case *ast.FuncLit:
		return g.funcLit(e, false, false), kOwned
	case *ast.BasicLit:
		tv := g.info.Types[e]
		return g.constLit(tv.Value, tv.Type), kStr
	case *ast.TypeAssertExpr:
		if !isAny(g.typeOf(e.X)) {
			fail("type assertion on %s", types.TypeString(g.typeOf(e.X), nil))
		}
		v, _ := anyVariant(g.typeOf(e))
		if v == "" {
			fail("type assertion to %s", types.TypeString(g.typeOf(e), nil))
		}
		c, _ := g.natural(e.X)
		return parenIfNeeded(c) + ".assert_" + snake(v) + "()", kOwned
	}
	fail("expression %T", e)
	return "", 0
}

func (g *gen) identExpr(e *ast.Ident) (string, kind) {
	obj := g.info.Uses[e]
	if obj == nil {
		fail("unresolved identifier %s", e.Name)
	}
	if vi := g.vars[obj]; vi != nil {
		return vi.name, vi.kind
	}
	switch o := obj.(type) {
	case *types.Const:
		if o.Pkg() != nil && o.Pkg().Path() == g.p.Path {
			// package-level constant of this file
			if isString(o.Type()) {
				return screaming(o.Name()), kStr
			}
			return screaming(o.Name()), kOwned
		}
		if o.Pkg() == nil {
			return g.constLit(o.Val(), o.Type()), kOwned
		}
		return g.qualified(o)
	case *types.Var:
		if o.Pkg() != nil && o.Pkg().Path() == g.p.Path && o.Parent() == o.Pkg().Scope() {
			return screaming(o.Name()), kPlace
		}
		if o.Pkg() != nil && o.Pkg().Path() != g.p.Path {
			return g.qualified(o)
		}
		fail("unknown variable %s", e.Name)
	case *types.Nil:
		fail("nil without context")
	case *types.Func:
		if o.Pkg() != nil && o.Pkg().Path() != g.p.Path {
			return g.qualified(o)
		}
		return ident(o.Name()), kOwned
	}
	fail("identifier %s (%T)", e.Name, obj)
	return "", 0
}

// qualified translates a reference to a package-level object of another package.
func (g *gen) qualified(obj types.Object) (string, kind) {
	path := obj.Pkg().Path()
	mod := pkgModule[path]
	if mod == "" {
		fail("reference to %s.%s", path, obj.Name())
	}
	switch o := obj.(type) {
	case *types.Const:
		if path == corePath {
			switch o.Name() {
			case "TSTrue":
				return "Tristate::True", kOwned
			case "TSFalse":
				return "Tristate::False", kOwned
			case "TSUnknown":
				return "Tristate::Unknown", kOwned
			}
		}
		if n, ok := types.Unalias(o.Type()).(*types.Named); ok && (stringNewtype(n) || rustEnumType(n) || isLsprotoIntEnum(n)) {
			tn := n.Obj().Name()
			if strings.HasPrefix(o.Name(), tn) && len(o.Name()) > len(tn) {
				return mod + "::" + tn + "::" + strings.TrimPrefix(o.Name(), tn), kOwned
			}
		}
		if isString(o.Type()) {
			return mod + "::" + screaming(o.Name()), kStr
		}
		return mod + "::" + screaming(o.Name()), kOwned
	case *types.Var:
		return mod + "::" + screaming(o.Name()), kPlace
	case *types.Func:
		return mod + "::" + ident(o.Name()), kOwned
	}
	fail("reference to %s.%s", path, obj.Name())
	return "", 0
}

func isLsprotoIntEnum(n *types.Named) bool {
	if n.Obj().Pkg() == nil || n.Obj().Pkg().Path() != lsprotoPath {
		return false
	}
	b, ok := n.Underlying().(*types.Basic)
	return ok && b.Info()&types.IsInteger != 0
}

func (g *gen) selectorExpr(e *ast.SelectorExpr) (string, kind) {
	sel := g.info.Selections[e]
	if sel == nil {
		// qualified identifier
		obj := g.info.Uses[e.Sel]
		if obj == nil {
			fail("unresolved selector %s", e.Sel.Name)
		}
		return g.qualified(obj)
	}
	if sel.Kind() != types.FieldVal {
		fail("method value %s", e.Sel.Name)
	}
	base, bk := g.natural(e.X)
	xt := g.typeOf(e.X)
	field := ident(e.Sel.Name)
	if isOptionPtr(xt) {
		if bk == kOptArc {
			fail("field of Option<Arc>")
		}
		base = parenIfNeeded(base) + ".as_ref().unwrap()"
	} else if bk == kOptArc {
		base = parenIfNeeded(base) + ".as_ref().unwrap()"
	} else {
		base = parenIfNeeded(base)
	}
	base += embeddedPath(xt, sel.Index())
	ft := sel.Obj().Type()
	if isArcPtr(ft) {
		return base + "." + field, kOptArc
	}
	return base + "." + field, kPlace
}

func (g *gen) indexExpr(e *ast.IndexExpr) (string, kind) {
	xt := types.Unalias(g.typeOf(e.X))
	switch u := xt.Underlying().(type) {
	case *types.Slice:
		base, _ := g.natural(e.X)
		return parenIfNeeded(base) + "[" + g.index(e.Index) + "]", kPlace
	case *types.Map:
		base, _ := g.natural(e.X)
		return parenIfNeeded(base) + ".get(" + g.expr(e.Index, u.Key(), mParamKey(u.Key())) + ").cloned().unwrap_or_default()", kOwned
	case *types.Pointer:
		if s, ok := u.Elem().Underlying().(*types.Slice); ok {
			_ = s
			base, _ := g.natural(e.X)
			return parenIfNeeded(base) + ".as_ref().unwrap()[" + g.index(e.Index) + "]", kPlace
		}
	}
	fail("index of %s", types.TypeString(xt, nil))
	return "", 0
}

func mParamKey(t types.Type) mode {
	if isString(t) {
		return mParam
	}
	return mOwned
}

func (g *gen) index(e ast.Expr) string {
	if tv := g.info.Types[e]; tv.Value != nil {
		return tv.Value.ExactString()
	}
	return parenIfNeeded(g.expr(e, types.Typ[types.Int], mOwned)) + " as usize"
}

func (g *gen) sliceExpr(e *ast.SliceExpr) (string, kind) {
	if e.Slice3 {
		fail("3-index slice")
	}
	base, _ := g.natural(e.X)
	lo, hi := "", ""
	if e.Low != nil {
		lo = g.index(e.Low)
	}
	if e.High != nil {
		hi = g.index(e.High)
	}
	xt := g.typeOf(e.X)
	if isString(xt) {
		return "go::slice_str(&" + parenIfNeeded(base) + ", " + optIdx(lo) + ", " + optIdx(hi) + ")", kOwned
	}
	return parenIfNeeded(base) + "[" + lo + ".." + hi + "].to_vec()", kOwned
}

func optIdx(s string) string {
	if s == "" {
		return "None"
	}
	return "Some(" + s + ")"
}

func (g *gen) binaryExpr(e *ast.BinaryExpr) (string, kind) {
	lt, rt := g.typeOf(e.X), g.typeOf(e.Y)
	switch e.Op {
	case token.ADD:
		if isString(lt) {
			return g.concat(e), kOwned
		}
	case token.EQL, token.NEQ:
		neg := e.Op == token.NEQ
		if g.isNil(e.Y) || g.isNil(e.X) {
			x := e.X
			if g.isNil(e.X) {
				x = e.Y
			}
			xt := g.typeOf(x)
			c, k := g.natural(x)
			var test string
			switch {
			case isAny(xt):
				test = parenIfNeeded(c) + ".is_nil()"
			case isOptionPtr(xt) || k == kOptArc:
				test = parenIfNeeded(c) + ".is_none()"
			default:
				switch types.Unalias(xt).Underlying().(type) {
				case *types.Slice, *types.Map:
					test = parenIfNeeded(c) + ".is_empty()"
				default:
					fail("comparison with nil of %s", types.TypeString(xt, nil))
				}
			}
			if neg {
				return "!" + test, kOwned
			}
			return test, kOwned
		}
		op := " == "
		if neg {
			op = " != "
		}
		if isString(lt) && isString(rt) {
			return g.strOperand(e.X) + op + g.strOperand(e.Y), kOwned
		}
		return parenIfNeeded(g.cmpOperand(e.X, rt)) + op + parenIfNeeded(g.cmpOperand(e.Y, lt)), kOwned
	case token.LAND, token.LOR:
		return parenIfNeeded(g.expr(e.X, types.Typ[types.Bool], mOwned)) + " " + e.Op.String() + " " + parenIfNeeded(g.expr(e.Y, types.Typ[types.Bool], mOwned)), kOwned
	}
	switch e.Op {
	case token.LSS, token.GTR, token.LEQ, token.GEQ, token.ADD, token.SUB, token.MUL, token.QUO, token.REM:
		if e.Op == token.REM || e.Op == token.QUO || e.Op == token.MUL || e.Op == token.SUB || e.Op == token.ADD {
			t := g.typeOf(e)
			return parenIfNeeded(g.expr(e.X, t, mOwned)) + " " + e.Op.String() + " " + parenIfNeeded(g.expr(e.Y, t, mOwned)), kOwned
		}
		if isString(lt) {
			return g.strOperand(e.X) + " " + e.Op.String() + " " + g.strOperand(e.Y), kOwned
		}
		t := g.typeOf(e)
		return parenIfNeeded(g.expr(e.X, t, mOwned)) + " " + e.Op.String() + " " + parenIfNeeded(g.expr(e.Y, t, mOwned)), kOwned
	}
	fail("binary %s", e.Op)
	return "", 0
}

func (g *gen) cmpOperand(e ast.Expr, other types.Type) string {
	t := g.typeOf(e)
	if tv := g.info.Types[e]; tv.Value != nil && !g.isNamedConstRef(e) {
		ct := t
		if b, ok := types.Unalias(t).(*types.Basic); ok && b.Info()&types.IsUntyped != 0 {
			ct = other
		}
		return g.constLit(tv.Value, ct)
	}
	c, k := g.natural(e)
	if k == kRef {
		return "*" + parenIfNeeded(c)
	}
	return c
}

// strOperand: a string expression as &str for comparisons.
func (g *gen) strOperand(e ast.Expr) string {
	if tv := g.info.Types[e]; tv.Value != nil && !g.isNamedConstRef(e) {
		return g.constLit(tv.Value, types.Typ[types.String])
	}
	c, k := g.natural(e)
	switch k {
	case kStr:
		return c
	case kRef:
		return parenIfNeeded(c) + ".as_str()"
	}
	return parenIfNeeded(c) + ".as_str()"
}

// concat flattens a chain of string `+` into one format!.
func (g *gen) concat(e *ast.BinaryExpr) string {
	var parts []ast.Expr
	var walk func(x ast.Expr)
	walk = func(x ast.Expr) {
		if b, ok := x.(*ast.BinaryExpr); ok && b.Op == token.ADD && isString(g.typeOf(b)) {
			if tv := g.info.Types[b]; tv.Value == nil {
				walk(b.X)
				walk(b.Y)
				return
			}
		}
		if p, ok := x.(*ast.ParenExpr); ok {
			walk(p.X)
			return
		}
		parts = append(parts, x)
	}
	walk(e)
	var f strings.Builder
	var args []string
	for _, p := range parts {
		if tv := g.info.Types[p]; tv.Value != nil {
			s := constant.StringVal(tv.Value)
			s = strings.ReplaceAll(s, "{", "{{")
			s = strings.ReplaceAll(s, "}", "}}")
			f.WriteString(s)
			continue
		}
		f.WriteString("{}")
		args = append(args, g.expr(p, types.Typ[types.String], mParam))
	}
	lit, ok := rustStr(f.String())
	if !ok {
		fail("non-UTF-8 string")
	}
	if len(args) == 0 {
		return lit + ".to_string()"
	}
	return "format!(" + lit + ", " + strings.Join(args, ", ") + ")"
}

// constLit renders a constant of Go type t.
func (g *gen) constLit(v constant.Value, t types.Type) string {
	if t != nil {
		if n, ok := types.Unalias(t).(*types.Named); ok && n.Obj().Pkg() != nil {
			// a named constant with this value, if the package declares one
			if name := findConst(n, v); name != "" {
				obj := n.Obj().Pkg().Scope().Lookup(name)
				c, _ := g.qualified(obj)
				return c
			}
			if stringNewtype(n) || isLsprotoIntEnum(n) {
				return namedPath(n) + "(" + g.constLit(v, n.Underlying()) + ")"
			}
			if isNamed(n, lsPath, "SortText") {
				return g.constLit(v, types.Typ[types.String])
			}
			if !rustEnumType(n) {
				fail("constant of named type %s", n.Obj().Name())
			}
			fail("constant %s of enum %s", v.ExactString(), n.Obj().Name())
		}
	}
	switch v.Kind() {
	case constant.String:
		s, ok := rustStr(constant.StringVal(v))
		if !ok {
			fail("non-UTF-8 string")
		}
		return s
	case constant.Bool:
		return strconv.FormatBool(constant.BoolVal(v))
	case constant.Int:
		return v.ExactString()
	case constant.Float:
		f, _ := constant.Float64Val(v)
		s := strconv.FormatFloat(f, 'g', -1, 64)
		if !strings.ContainsAny(s, ".e") {
			s += ".0"
		}
		return s
	}
	fail("constant %s", v.ExactString())
	return ""
}

func findConst(n *types.Named, v constant.Value) string {
	scope := n.Obj().Pkg().Scope()
	var best string
	for _, name := range scope.Names() {
		c, ok := scope.Lookup(name).(*types.Const)
		if !ok || !c.Exported() || !types.Identical(c.Type(), n) {
			continue
		}
		if constant.Compare(c.Val(), token.EQL, v) {
			if best == "" || strings.HasPrefix(name, n.Obj().Name()) && !strings.HasPrefix(best, n.Obj().Name()) {
				best = name
			}
		}
	}
	return best
}

// compositeLit translates T{...} (t is the literal's type, not a pointer).
func (g *gen) compositeLit(e *ast.CompositeLit, t types.Type) string {
	switch u := types.Unalias(t).Underlying().(type) {
	case *types.Struct:
		if u.NumFields() == 0 {
			return "()"
		}
		path := g.localType(t)
		var fields []string
		set := 0
		for i, el := range e.Elts {
			var fl *types.Var
			var val ast.Expr
			if kv, ok := el.(*ast.KeyValueExpr); ok {
				name := kv.Key.(*ast.Ident).Name
				for j := 0; j < u.NumFields(); j++ {
					if u.Field(j).Name() == name {
						fl = u.Field(j)
					}
				}
				val = kv.Value
			} else {
				fl = u.Field(i)
				val = el
			}
			set++
			fields = append(fields, ident(fl.Name())+": "+g.fieldValue(val, fl.Type()))
		}
		if set < u.NumFields() {
			fields = append(fields, "..Default::default()")
		}
		if len(fields) == 0 {
			return path + "::default()"
		}
		return path + " { " + strings.Join(fields, ", ") + " }"
	case *types.Slice:
		var parts []string
		for _, el := range e.Elts {
			if _, ok := el.(*ast.KeyValueExpr); ok {
				fail("indexed slice literal")
			}
			parts = append(parts, g.elem(el, u.Elem()))
		}
		return "vec![" + strings.Join(parts, ", ") + "]"
	case *types.Map:
		var parts []string
		for _, el := range e.Elts {
			kv := el.(*ast.KeyValueExpr)
			parts = append(parts, "("+g.expr(kv.Key, u.Key(), mOwned)+", "+g.expr(kv.Value, u.Elem(), mOwned)+")")
		}
		return "go::map_of([" + strings.Join(parts, ", ") + "])"
	}
	fail("composite literal of %s", types.TypeString(t, nil))
	return ""
}

// fieldValue: a struct field value. Harness-object pointer fields are Option<Arc<T>>.
func (g *gen) fieldValue(val ast.Expr, ft types.Type) string {
	if isArcPtr(ft) {
		if g.isNil(val) {
			return "None"
		}
		return "Some(" + g.expr(val, ft, mOwned) + ")"
	}
	if _, ok := types.Unalias(ft).(*types.Signature); ok {
		fail("func-valued field")
	}
	return g.expr(val, ft, mOwned)
}

func (g *gen) isBuiltin(fun ast.Expr, name string) bool {
	id, ok := fun.(*ast.Ident)
	if !ok {
		return false
	}
	b, ok := g.info.Uses[id].(*types.Builtin)
	return ok && b.Name() == name
}

// embeddedPath: the Rust path through embedded fields for a promoted field selection (Go index path).
func embeddedPath(t types.Type, index []int) string {
	path := ""
	cur := derefType(t)
	for _, i := range index[:len(index)-1] {
		st, ok := types.Unalias(cur).Underlying().(*types.Struct)
		if !ok {
			fail("promoted field through %s", types.TypeString(cur, nil))
		}
		f := st.Field(i)
		path += "." + ident(f.Name())
		cur = derefType(f.Type())
		if isOptionPtr(f.Type()) {
			fail("promoted field through a pointer")
		}
	}
	return path
}
