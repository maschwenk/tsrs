package main

import (
	"go/ast"
	"go/token"
	"go/types"
	"strings"
)

func (g *gen) stmts(list []ast.Stmt) {
	for i, s := range list {
		if d, ok := s.(*ast.DeferStmt); ok {
			g.deferStmt(d, list[i+1:])
			return
		}
		g.stmt(s)
	}
}

// deferStmt: the generated shape keeps the Go defer ordering on normal return. Release builds abort on panic, so
// go::run does not recover and deferred calls do not run on that path.
func (g *gen) deferStmt(d *ast.DeferStmt, rest []ast.Stmt) {
	if g.inResultFunc {
		fail("defer in a function with results")
	}
	call := d.Call
	// Evaluate the deferred call's arguments now, as Go does.
	isRecover := false
	if sel, ok := call.Fun.(*ast.SelectorExpr); ok {
		if o := g.info.Uses[sel.Sel]; o != nil && o.Pkg() != nil && o.Pkg().Path() == testutilPath && o.Name() == "RecoverAndFail" {
			isRecover = true
		}
	}
	res := g.newTmp("defer")
	var deferred string
	if isRecover {
		deferred = "testutil::recover_and_fail(" + g.expr(call.Args[0], g.info.TypeOf(call.Args[0]), mParam) + ", " + g.expr(call.Args[1], types.Typ[types.String], mParam) + ", " + res + ")"
	} else {
		deferred = g.callExpr(call, mStmt)
	}
	g.line("let %s = go::run(|| {", res)
	g.indent++
	saved := g.saveScope()
	g.stmts(rest)
	g.restoreScope(saved)
	g.indent--
	g.line("});")
	if isRecover {
		g.line("%s;", deferred)
	} else {
		g.line("%s;", deferred)
		g.line("go::resume(%s);", res)
	}
}

type scopeState struct {
	curF, curT string
}

func (g *gen) saveScope() scopeState     { return scopeState{g.curF, g.curT} }
func (g *gen) restoreScope(s scopeState) { g.curF, g.curT = s.curF, s.curT }
func (g *gen) block(list []ast.Stmt) {
	s := g.saveScope()
	g.indent++
	g.stmts(list)
	g.indent--
	g.restoreScope(s)
}

func (g *gen) stmt(s ast.Stmt) {
	switch s := s.(type) {
	case *ast.ExprStmt:
		call, ok := s.X.(*ast.CallExpr)
		if !ok {
			fail("expression statement %T", s.X)
		}
		g.line("%s;", g.callExpr(call, mStmt))
	case *ast.AssignStmt:
		g.assign(s)
	case *ast.IncDecStmt:
		op := "+= 1"
		if s.Tok == token.DEC {
			op = "-= 1"
		}
		g.line("%s %s;", g.lvalue(s.X), op)
	case *ast.DeclStmt:
		gd := s.Decl.(*ast.GenDecl)
		switch gd.Tok {
		case token.CONST:
			for _, sp := range gd.Specs {
				vs := sp.(*ast.ValueSpec)
				for _, n := range vs.Names {
					obj := g.info.Defs[n].(*types.Const)
					name := ident(n.Name)
					k := kOwned
					if isString(obj.Type()) {
						k = kStr
					}
					g.vars[obj] = &varInfo{name: name, kind: k}
					g.line("let %s: %s = %s;", name, constRType(obj.Type()), g.constLit(obj.Val(), obj.Type()))
				}
			}
		case token.VAR:
			for _, sp := range gd.Specs {
				vs := sp.(*ast.ValueSpec)
				for i, n := range vs.Names {
					obj := g.info.Defs[n]
					name := ident(n.Name)
					var val string
					if len(vs.Values) > 0 {
						val = g.expr(vs.Values[i], obj.Type(), mOwned)
					} else {
						val = "Default::default()"
					}
					g.declare(obj, name)
					g.line("let mut %s: %s = %s;", name, g.localType(obj.Type()), val)
				}
			}
		case token.TYPE:
			for _, sp := range gd.Specs {
				code := g.genTypeSpec(sp.(*ast.TypeSpec), "")
				for _, l := range strings.Split(strings.TrimRight(code, "\n"), "\n") {
					g.line("%s", l)
				}
			}
		}
	case *ast.IfStmt:
		g.ifStmt(s, false)
	case *ast.RangeStmt:
		g.rangeStmt(s)
	case *ast.ForStmt:
		g.forStmt(s)
	case *ast.BlockStmt:
		g.line("{")
		g.block(s.List)
		g.line("}")
	case *ast.ReturnStmt:
		switch len(s.Results) {
		case 0:
			g.line("return;")
		case 1:
			if len(g.resultTypes) == 1 {
				g.line("return %s;", g.expr(s.Results[0], g.resultTypes[0], mOwned))
			} else {
				// return f() where f returns several values
				g.line("return %s;", g.expr(s.Results[0], nil, mOwned))
			}
		default:
			var parts []string
			for i, r := range s.Results {
				parts = append(parts, g.expr(r, g.resultTypes[i], mOwned))
			}
			g.line("return (%s);", strings.Join(parts, ", "))
		}
	case *ast.SwitchStmt:
		g.switchStmt(s)
	case *ast.BranchStmt:
		if s.Label != nil {
			fail("labeled %s", s.Tok)
		}
		switch s.Tok {
		case token.BREAK:
			g.line("break;")
		case token.CONTINUE:
			g.line("continue;")
		default:
			fail("branch %s", s.Tok)
		}
	case *ast.EmptyStmt:
	default:
		fail("statement %T", s)
	}
}

func (g *gen) localType(t types.Type) string {
	if n, ok := types.Unalias(t).(*types.Named); ok {
		if _, pkgLevel := objModule[n.Obj()]; pkgLevel {
			return rtype(t, false)
		}
		if name, ok := g.localTypes[n.Obj()]; ok {
			return name
		}
	}
	return rtype(t, false)
}

// declare registers a new local variable.
func (g *gen) declare(obj types.Object, name string) *varInfo {
	vi := &varInfo{name: name, kind: kPlace}
	g.vars[obj] = vi
	if isTestingTPtr(obj.Type()) {
		g.curT = name
	}
	return vi
}

func (g *gen) assign(s *ast.AssignStmt) {
	switch s.Tok {
	case token.DEFINE:
		if len(s.Rhs) == 1 && len(s.Lhs) > 1 {
			// a, b := f()
			var names []string
			var objs []types.Object
			for _, l := range s.Lhs {
				id := l.(*ast.Ident)
				if id.Name == "_" {
					names = append(names, "_")
					objs = append(objs, nil)
					continue
				}
				obj := g.info.Defs[id]
				if obj == nil {
					fail(":= redeclaring an existing variable")
				}
				names = append(names, "mut "+ident(id.Name))
				objs = append(objs, obj)
			}
			g.line("let (%s) = %s;", strings.Join(names, ", "), g.expr(s.Rhs[0], nil, mOwned))
			var fName string
			for _, obj := range objs {
				if obj != nil && isFourslashTestPtr(obj.Type()) {
					fName = ident(obj.Name())
				}
			}
			for _, obj := range objs {
				if obj == nil {
					continue
				}
				vi := g.declare(obj, ident(obj.Name()))
				if isFourslashTestPtr(obj.Type()) {
					g.line("let %s = &mut %s;", vi.name, vi.name)
					g.curF = vi.name
				}
				if _, ok := types.Unalias(obj.Type()).(*types.Signature); ok {
					vi.fclosure = true
					vi.fOwner = fName
				}
			}
			return
		}
		if len(s.Lhs) != len(s.Rhs) {
			fail("assignment count mismatch")
		}
		for i, l := range s.Lhs {
			id := l.(*ast.Ident)
			obj := g.info.Defs[id]
			if id.Name == "_" {
				g.line("let _ = %s;", g.expr(s.Rhs[i], g.info.TypeOf(s.Rhs[i]), mOwned))
				continue
			}
			if obj == nil {
				fail(":= redeclaring an existing variable")
			}
			if fl, ok := s.Rhs[i].(*ast.FuncLit); ok {
				name := ident(id.Name)
				takesF := g.refersToF(fl)
				code := g.funcLit(fl, takesF, false)
				vi := g.declare(obj, name)
				vi.takesF = takesF
				g.line("let mut %s = %s;", name, code)
				continue
			}
			name := ident(id.Name)
			if isOptionPtr(obj.Type()) {
				if c, k := g.naturalIfPlace(s.Rhs[i]); k == kDeref {
					vi := g.declare(obj, name)
					vi.kind = kDeref
					g.line("let mut %s = %s.clone();", name, parenIfNeeded(c))
					continue
				}
			}
			val := g.expr(s.Rhs[i], obj.Type(), mOwned)
			vi := g.declare(obj, name)
			if _, ok := types.Unalias(obj.Type()).(*types.Signature); ok {
				vi.fclosure = true
				vi.fOwner = g.curF
			}
			g.line("let mut %s = %s;", name, val)
			if isFourslashTestPtr(obj.Type()) {
				g.line("let %s = &mut %s;", name, name)
				g.curF = name
			}
		}
	case token.ASSIGN:
		if len(s.Lhs) != len(s.Rhs) {
			fail("tuple assignment")
		}
		for i, l := range s.Lhs {
			if id, ok := l.(*ast.Ident); ok && id.Name == "_" {
				g.line("let _ = %s;", g.expr(s.Rhs[i], g.info.TypeOf(s.Rhs[i]), mOwned))
				continue
			}
			lt := g.info.TypeOf(l)
			if ix, ok := l.(*ast.IndexExpr); ok {
				if _, isMap := types.Unalias(g.info.TypeOf(ix.X)).Underlying().(*types.Map); isMap {
					mt := types.Unalias(g.info.TypeOf(ix.X)).Underlying().(*types.Map)
					g.line("%s.insert(%s, %s);", g.lvalue(ix.X), g.expr(ix.Index, mt.Key(), mOwned), g.expr(s.Rhs[i], mt.Elem(), mOwned))
					continue
				}
			}
			// x = append(x, ...)
			if call, ok := s.Rhs[i].(*ast.CallExpr); ok && g.isBuiltin(call.Fun, "append") && sameExpr(call.Args[0], l) {
				g.line("%s", g.appendInPlace(l, call))
				continue
			}
			if sel, ok := l.(*ast.SelectorExpr); ok && isOptionSliceField(g.typeOf(sel.X), sel.Sel.Name) {
				g.line("%s = Some(%s);", g.lvalue(l), g.expr(s.Rhs[i], lt, mOwned))
				continue
			}
			g.line("%s = %s;", g.lvalue(l), g.expr(s.Rhs[i], lt, mOwned))
		}
	case token.ADD_ASSIGN, token.SUB_ASSIGN, token.MUL_ASSIGN, token.QUO_ASSIGN, token.REM_ASSIGN:
		lt := g.info.TypeOf(s.Lhs[0])
		if isString(lt) && s.Tok == token.ADD_ASSIGN {
			g.line("%s.push_str(%s);", g.lvalue(s.Lhs[0]), g.expr(s.Rhs[0], lt, mParam))
			return
		}
		g.line("%s %s %s;", g.lvalue(s.Lhs[0]), s.Tok.String(), g.expr(s.Rhs[0], lt, mOwned))
	default:
		fail("assignment %s", s.Tok)
	}
}

func sameExpr(a, b ast.Expr) bool {
	ai, ok1 := a.(*ast.Ident)
	bi, ok2 := b.(*ast.Ident)
	return ok1 && ok2 && ai.Name == bi.Name
}

func (g *gen) appendInPlace(l ast.Expr, call *ast.CallExpr) string {
	st, _ := isSlice(g.info.TypeOf(l))
	target := g.lvalue(l)
	if call.Ellipsis.IsValid() {
		return target + ".extend(" + g.expr(call.Args[1], g.info.TypeOf(call.Args[1]), mOwned) + ");"
	}
	var parts []string
	for _, a := range call.Args[1:] {
		parts = append(parts, target+".push("+g.elem(a, st.Elem())+");")
	}
	return strings.Join(parts, " ")
}

func (g *gen) ifStmt(s *ast.IfStmt, isElse bool) {
	if s.Init != nil {
		if isElse {
			fail("else if with init")
		}
		g.line("{")
		g.indent++
		g.stmt(s.Init)
	}
	cond := g.expr(s.Cond, types.Typ[types.Bool], mOwned)
	if isElse {
		g.buf.WriteString("if " + cond + " {\n")
	} else {
		g.line("if %s {", cond)
	}
	g.block(s.Body.List)
	switch e := s.Else.(type) {
	case nil:
		g.line("}")
	case *ast.BlockStmt:
		g.line("} else {")
		g.block(e.List)
		g.line("}")
	case *ast.IfStmt:
		g.buf.WriteString(strings.Repeat("    ", g.indent) + "} else ")
		g.ifStmt(e, true)
	}
	if s.Init != nil {
		g.indent--
		g.line("}")
	}
}

func (g *gen) rangeStmt(s *ast.RangeStmt) {
	xt := types.Unalias(g.info.TypeOf(s.X))
	name := func(e ast.Expr) (string, types.Object) {
		if e == nil {
			return "_", nil
		}
		id := e.(*ast.Ident)
		if id.Name == "_" {
			return "_", nil
		}
		obj := g.info.Defs[id]
		if obj == nil {
			fail("range assigning to existing variables")
		}
		return ident(id.Name), obj
	}
	kName, kObj := name(s.Key)
	vName, vObj := name(s.Value)
	saved := g.saveScope()
	switch u := xt.Underlying().(type) {
	case *types.Basic:
		// for i := range n
		g.line("for %s in 0..%s {", kName, g.expr(s.X, xt, mOwned))
		if kObj != nil {
			g.declare(kObj, kName)
		}
	case *types.Slice:
		src := g.expr(s.X, xt, mOwned)
		if vObj == nil {
			if kObj == nil {
				g.line("for _ in 0..%s.len() {", parenIfNeeded(src))
			} else {
				g.line("for %s in 0..%s.len() as i32 {", kName, parenIfNeeded(src))
				g.declare(kObj, kName)
			}
		} else if kObj == nil {
			g.line("for %s in %s {", vName, src)
			g.declare(vObj, vName)
		} else {
			g.line("for (%s, %s) in %s.into_iter().enumerate() {", kName, vName, parenIfNeeded(src))
			g.indent++
			g.line("let %s = %s as i32;", kName, kName)
			g.indent--
			g.declare(kObj, kName)
			g.declare(vObj, vName)
		}
		if vObj != nil && isOptionPtr(u.Elem()) {
			g.vars[vObj].kind = kDeref
		}
		_ = u
	case *types.Map:
		g.line("for (%s, %s) in %s {", kName, vName, g.expr(s.X, xt, mOwned))
		if kObj != nil {
			g.declare(kObj, kName)
		}
		if vObj != nil {
			g.declare(vObj, vName)
		}
	default:
		fail("range over %s", xt)
	}
	g.block(s.Body.List)
	g.restoreScope(saved)
	g.line("}")
}

func (g *gen) forStmt(s *ast.ForStmt) {
	hasContinue := false
	ast.Inspect(s.Body, func(n ast.Node) bool {
		if b, ok := n.(*ast.BranchStmt); ok && b.Tok == token.CONTINUE {
			hasContinue = true
		}
		_, isLit := n.(*ast.FuncLit)
		return !isLit
	})
	if hasContinue && s.Post != nil {
		fail("for loop with continue and post statement")
	}
	g.line("{")
	g.indent++
	if s.Init != nil {
		g.stmt(s.Init)
	}
	cond := "true"
	if s.Cond != nil {
		cond = g.expr(s.Cond, types.Typ[types.Bool], mOwned)
	}
	g.line("while %s {", cond)
	g.indent++
	g.stmts(s.Body.List)
	if s.Post != nil {
		g.stmt(s.Post)
	}
	g.indent--
	g.line("}")
	g.indent--
	g.line("}")
}

func (g *gen) switchStmt(s *ast.SwitchStmt) {
	if s.Init != nil {
		fail("switch with init")
	}
	tag := ""
	var tagT types.Type
	if s.Tag != nil {
		tagT = g.info.TypeOf(s.Tag)
		tag = g.newTmp("tag")
		g.line("let %s = %s;", tag, g.expr(s.Tag, tagT, mOwned))
	}
	first := true
	var def *ast.CaseClause
	for _, c := range s.Body.List {
		cc := c.(*ast.CaseClause)
		if cc.List == nil {
			def = cc
			continue
		}
		var conds []string
		for _, e := range cc.List {
			if s.Tag == nil {
				conds = append(conds, g.expr(e, types.Typ[types.Bool], mOwned))
			} else if isString(tagT) {
				conds = append(conds, tag+" == "+g.expr(e, tagT, mParam))
			} else {
				conds = append(conds, tag+" == "+g.expr(e, tagT, mOwned))
			}
		}
		for _, st := range cc.Body {
			if b, ok := st.(*ast.BranchStmt); ok && b.Tok == token.FALLTHROUGH {
				fail("fallthrough")
			}
		}
		kw := "} else if"
		if first {
			kw = "if"
		}
		first = false
		g.line("%s %s {", kw, strings.Join(conds, " || "))
		g.block(cc.Body)
	}
	if def != nil {
		if first {
			g.line("{")
		} else {
			g.line("} else {")
		}
		g.block(def.Body)
		g.line("}")
	} else if !first {
		g.line("}")
	}
}

// refersToF: whether a function literal uses the enclosing FourslashTest variable (it then receives it as a
// parameter instead of capturing it, so the caller can keep using it between calls).
func (g *gen) refersToF(fl *ast.FuncLit) bool {
	found := false
	ast.Inspect(fl.Body, func(n ast.Node) bool {
		if id, ok := n.(*ast.Ident); ok {
			if o := g.info.Uses[id]; o != nil {
				if _, isVar := o.(*types.Var); isVar && isFourslashTestPtr(o.Type()) {
					found = true
				}
				if vi := g.vars[o]; vi != nil && (vi.takesF || vi.fclosure) {
					found = true
				}
			}
		}
		return !found
	})
	return found
}

// lvalue translates an assignment target.
func (g *gen) lvalue(e ast.Expr) string {
	switch e := e.(type) {
	case *ast.Ident:
		obj := g.info.Uses[e]
		if vi := g.vars[obj]; vi != nil {
			if vi.kind != kPlace {
				fail("assignment to a borrowed variable")
			}
			return vi.name
		}
		fail("assignment to %s", e.Name)
	case *ast.SelectorExpr:
		sel := g.info.Selections[e]
		if sel == nil || sel.Kind() != types.FieldVal {
			fail("assignment to selector")
		}
		base := g.lvalue(e.X)
		if isOptionPtr(g.typeOf(e.X)) {
			base += ".as_mut().unwrap()"
		}
		return base + embeddedPath(g.typeOf(e.X), sel.Index()) + "." + ident(e.Sel.Name)
	case *ast.IndexExpr:
		if _, ok := isSlice(g.typeOf(e.X)); ok {
			return g.lvalue(e.X) + "[" + g.index(e.Index) + "]"
		}
	case *ast.ParenExpr:
		return g.lvalue(e.X)
	}
	fail("assignment target %T", e)
	return ""
}
