package main

import (
	"go/ast"
	"go/constant"
	"go/token"
	"go/types"
	"regexp"
	"strings"
)

var fixtureFormat = regexp.MustCompile(`\{[a-zA-Z_][a-zA-Z0-9_]*:[0-9]+\}`)

func (g *gen) stmts(list []ast.Stmt) {
	g.stmtsWithTailReturn(list, false)
}

// Only a function or closure's final return can become a Rust tail expression.
// Returns at the end of loop bodies and ordinary nested blocks still exit the function.
func (g *gen) stmtsWithTailReturn(list []ast.Stmt, tail bool) {
	for i, s := range list {
		if decl, ok := s.(*ast.DeclStmt); ok {
			gd := decl.Decl.(*ast.GenDecl)
			if gd.Tok == token.VAR {
				for _, spec := range gd.Specs {
					vs := spec.(*ast.ValueSpec)
					if len(vs.Values) != 0 {
						continue
					}
					for _, name := range vs.Names {
						obj := g.info.Defs[name]
						if end, ok := g.assignedBeforeRead(obj, list[i+1:]); ok {
							g.uninitialized[obj] = true
							g.assigned[obj] = false
							for _, rest := range list[i+2+end:] {
								if g.refersTo(obj, rest) {
									// Later uses may assign again; keep the original mutability when they do.
									ast.Inspect(rest, func(n ast.Node) bool {
										if a, ok := n.(*ast.AssignStmt); ok {
											for _, lhs := range a.Lhs {
												if g.refersTo(obj, lhs) {
													g.assigned[obj] = true
												}
											}
										}
										return true
									})
								}
							}
						}
					}
				}
			}
		}
		if d, ok := s.(*ast.DeferStmt); ok {
			g.deferStmt(d, list[i+1:])
			return
		}
		if g.literalFormattingFixture(s) {
			g.line("#[expect(clippy::literal_string_with_formatting_args, reason = \"These braces are TypeScript fixture syntax, not Rust format placeholders.\")]")
		}
		if ret, ok := s.(*ast.ReturnStmt); ok && tail && i == len(list)-1 {
			g.returnStmt(ret, true)
		} else {
			g.stmt(s)
		}
	}
}

// deferStmt: Go runs the deferred call when the function returns or panics (t.Fatal unwinds through it too).
// The rest of the block runs under go::run (catch_unwind); the deferred call runs afterwards and the panic, if
// any, is resumed.
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

// canonicalFourslashTest emits the overwhelmingly common test shape through one hand-written Rust helper:
//
//	t.Parallel()
//	defer testutil.RecoverAndFail(t, "Panic on fourslash test")
//	const content = "..."
//	f, done := fourslash.NewFourslash(t, nil, content)
//	defer done()
//	// test actions
//
// The helper takes a function pointer, rather than a generic callback, so its defer scaffolding is compiled once.
// Return false for any variation; the general defer lowering above preserves its exact Go semantics.
func (g *gen) canonicalFourslashTest(stmts []ast.Stmt) bool {
	recoverAt := -1
	var recoverCall *ast.CallExpr
	for i, stmt := range stmts {
		if d, ok := stmt.(*ast.DeferStmt); ok {
			recoverAt = i
			recoverCall = d.Call
			break
		}
	}
	if recoverAt < 0 || len(recoverCall.Args) != 2 {
		return false
	}
	recoverSel, ok := recoverCall.Fun.(*ast.SelectorExpr)
	if !ok {
		return false
	}
	recoverFn, ok := g.info.Uses[recoverSel.Sel].(*types.Func)
	if !ok || recoverFn.Pkg() == nil || recoverFn.Pkg().Path() != testutilPath || recoverFn.Name() != "RecoverAndFail" {
		return false
	}
	rest := stmts[recoverAt+1:]
	if len(rest) < 3 {
		return false
	}
	msg := g.info.Types[recoverCall.Args[1]]
	if msg.Value == nil || msg.Value.Kind() != constant.String || constant.StringVal(msg.Value) != "Panic on fourslash test" {
		return false
	}
	t, ok := recoverCall.Args[0].(*ast.Ident)
	if !ok || !isTestingTPtr(g.info.TypeOf(t)) {
		return false
	}
	if recoverAt != 1 {
		return false
	}
	parallelCall, ok := isCallStmt(stmts[0])
	if !ok || len(parallelCall.Args) != 0 {
		return false
	}
	parallelSel, ok := parallelCall.Fun.(*ast.SelectorExpr)
	if !ok || parallelSel.Sel.Name != "Parallel" || !sameExpr(t, parallelSel.X) {
		return false
	}

	decl, ok := rest[0].(*ast.DeclStmt)
	if !ok {
		return false
	}
	gd, ok := decl.Decl.(*ast.GenDecl)
	if !ok || gd.Tok != token.CONST || len(gd.Specs) != 1 {
		return false
	}
	vs, ok := gd.Specs[0].(*ast.ValueSpec)
	if !ok || len(vs.Names) != 1 {
		return false
	}
	contentObj, ok := g.info.Defs[vs.Names[0]].(*types.Const)
	if !ok || !isString(contentObj.Type()) {
		return false
	}
	// The one fixture built at runtime can panic while allocating, so leave it inside the general recovery boundary.
	if _, compressed := rustCompressedString(constant.StringVal(contentObj.Val())); compressed {
		return false
	}

	assign, ok := rest[1].(*ast.AssignStmt)
	if !ok || assign.Tok != token.DEFINE || len(assign.Lhs) != 2 || len(assign.Rhs) != 1 {
		return false
	}
	fID, ok := assign.Lhs[0].(*ast.Ident)
	if !ok {
		return false
	}
	doneID, ok := assign.Lhs[1].(*ast.Ident)
	if !ok {
		return false
	}
	fObj := g.info.Defs[fID]
	doneObj := g.info.Defs[doneID]
	if fObj == nil || doneObj == nil || !isFourslashTestPtr(fObj.Type()) {
		return false
	}
	newCall, ok := assign.Rhs[0].(*ast.CallExpr)
	if !ok || len(newCall.Args) != 3 || !sameExpr(t, newCall.Args[0]) || !g.isNil(newCall.Args[1]) {
		return false
	}
	sel, ok := newCall.Fun.(*ast.SelectorExpr)
	if !ok {
		return false
	}
	newFn, ok := g.info.Uses[sel.Sel].(*types.Func)
	if !ok || newFn.Pkg() == nil || newFn.Pkg().Path() != fsPath || newFn.Name() != "NewFourslash" {
		return false
	}
	contentID, ok := newCall.Args[2].(*ast.Ident)
	if !ok || g.info.Uses[contentID] != contentObj {
		return false
	}

	doneDefer, ok := rest[2].(*ast.DeferStmt)
	if !ok || len(doneDefer.Call.Args) != 0 {
		return false
	}
	doneUse, ok := doneDefer.Call.Fun.(*ast.Ident)
	if !ok || g.info.Uses[doneUse] != doneObj {
		return false
	}
	for _, action := range rest[3:] {
		if g.refersTo(contentObj, action) || g.refersTo(doneObj, action) {
			return false
		}
	}

	// Register the FourslashTest callback parameter as if NewFourslash had declared it. The constant is emitted
	// directly as the helper argument; the checks above and below prove it has no other use.
	fName := ident(fObj.Name())
	g.vars[fObj] = &varInfo{name: fName, kind: kPlace}
	g.curF = fName
	tName := ident(t.Name)
	content := g.constLit(contentObj.Val(), contentObj.Type())
	if g.literalFormattingFixture(rest[0]) {
		g.line("#[expect(clippy::literal_string_with_formatting_args, reason = \"These braces are TypeScript fixture syntax, not Rust format placeholders.\")]")
	}
	g.line("fourslash::run_generated_test(%s, %s, |%s, %s| {", tName, content, fName, tName)
	g.indent++
	g.stmts(rest[3:])
	g.indent--
	g.line("});")
	return true
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
					if isString(obj.Type()) {
						if value, ok := rustCompressedString(constant.StringVal(obj.Val())); ok {
							g.vars[obj] = &varInfo{name: name, kind: kPlace}
							g.line("let %s: String = %s;", name, value)
							continue
						}
					}
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
						val = rustDefault(g.localType(obj.Type()))
					}
					g.declare(obj, name)
					if g.uninitialized[obj] {
						g.line("let %s: %s;", g.binding(name, obj), g.localType(obj.Type()))
					} else {
						g.line("let %s: %s = %s;", g.binding(name, obj), g.localType(obj.Type()), val)
					}
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
		g.returnStmt(s, false)
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

func (g *gen) refersTo(obj types.Object, n ast.Node) bool {
	if n == nil {
		return false
	}
	found := false
	ast.Inspect(n, func(n ast.Node) bool {
		if id, ok := n.(*ast.Ident); ok && g.info.Uses[id] == obj {
			found = true
		}
		return !found
	})
	return found
}

// Omit Go's zero initialization only when an assignment, or both branches of an if, replace it before a read.
func (g *gen) assignedBeforeRead(obj types.Object, list []ast.Stmt) (int, bool) {
	for i, s := range list {
		if !g.refersTo(obj, s) {
			continue
		}
		switch s := s.(type) {
		case *ast.AssignStmt:
			if s.Tok != token.ASSIGN {
				return 0, false
			}
			for _, rhs := range s.Rhs {
				if g.refersTo(obj, rhs) {
					return 0, false
				}
			}
			for _, lhs := range s.Lhs {
				if id, ok := lhs.(*ast.Ident); ok && g.info.Uses[id] == obj {
					return i, true
				}
			}
		case *ast.IfStmt:
			if s.Init != nil && g.refersTo(obj, s.Init) || g.refersTo(obj, s.Cond) {
				return 0, false
			}
			if _, ok := g.assignedBeforeRead(obj, s.Body.List); !ok {
				return 0, false
			}
			var other []ast.Stmt
			switch e := s.Else.(type) {
			case *ast.BlockStmt:
				other = e.List
			case *ast.IfStmt:
				other = []ast.Stmt{e}
			}
			if _, ok := g.assignedBeforeRead(obj, other); ok {
				return i, true
			}
		}
		return 0, false
	}
	return 0, false
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
				names = append(names, g.binding(ident(id.Name), obj))
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
				g.line("let %s = %s;", g.binding(name, obj), code)
				continue
			}
			name := ident(id.Name)
			if isOptionPtr(obj.Type()) {
				if !g.assigned[obj] {
					if inner, ok := g.ptrLiteral(s.Rhs[i]); ok {
						vi := g.declare(obj, name)
						vi.kind = kDeref
						g.line("let %s = %s;", name, inner)
						continue
					}
				}
				if c, k := g.naturalIfPlace(s.Rhs[i]); k == kDeref {
					vi := g.declare(obj, name)
					vi.kind = kDeref
					g.line("let %s = %s.clone();", g.binding(name, obj), parenIfNeeded(c))
					continue
				}
			}
			val := g.expr(s.Rhs[i], obj.Type(), mOwned)
			vi := g.declare(obj, name)
			if _, ok := types.Unalias(obj.Type()).(*types.Signature); ok {
				vi.fclosure = true
				vi.fOwner = g.curF
			}
			g.line("let %s = %s;", g.binding(name, obj), val)
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

func (g *gen) literalFormattingFixture(n ast.Node) bool {
	found := false
	ast.Inspect(n, func(n ast.Node) bool {
		if found {
			return false
		}
		if _, ok := n.(*ast.FuncLit); ok {
			return false
		}
		if e, ok := n.(ast.Expr); ok {
			if tv := g.info.Types[e]; tv.Value != nil && tv.Value.Kind() == constant.String && !g.isNamedConstRef(e) {
				found = fixtureFormat.MatchString(constant.StringVal(tv.Value))
			}
		}
		return !found
	})
	return found
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

func (g *gen) returnStmt(s *ast.ReturnStmt, tail bool) {
	var value string
	switch len(s.Results) {
	case 0:
		if !tail {
			g.line("return;")
		}
		return
	case 1:
		var want types.Type
		if len(g.resultTypes) == 1 {
			want = g.resultTypes[0]
		}
		value = g.expr(s.Results[0], want, mOwned)
	default:
		var parts []string
		for i, r := range s.Results {
			parts = append(parts, g.expr(r, g.resultTypes[i], mOwned))
		}
		value = "(" + strings.Join(parts, ", ") + ")"
	}
	if tail {
		g.line("%s", value)
	} else {
		g.line("return %s;", value)
	}
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
	bindings := len(g.nonNil)
	cond := g.ifCondition(s)
	if isElse {
		g.buf.WriteString("if " + cond + " {\n")
	} else {
		g.line("if %s {", cond)
	}
	g.block(s.Body.List)
	g.nonNil = g.nonNil[:bindings]
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

// Borrow a read-only local pointer after its nil guard. Keep the original condition for mutable places and
// expressions with side effects: evaluating those once or keeping a borrow could change their meaning.
func (g *gen) ifCondition(s *ast.IfStmt) string {
	guard := s.Cond
	var rest ast.Expr
	if and, ok := guard.(*ast.BinaryExpr); ok && and.Op == token.LAND {
		guard, rest = and.X, and.Y
	}
	if cmp, ok := guard.(*ast.BinaryExpr); ok && cmp.Op == token.NEQ {
		place := cmp.X
		if g.isNil(cmp.X) {
			place = cmp.Y
		}
		if (g.isNil(cmp.X) || g.isNil(cmp.Y)) && isOptionPtr(g.typeOf(place)) && g.readOnlyPlace(place) && g.borrowedPointer(place) == "" {
			used := false
			check := func(n ast.Node) bool {
				switch n := n.(type) {
				case *ast.SelectorExpr:
					used = used || g.samePlace(place, n.X)
				case *ast.StarExpr:
					used = used || g.samePlace(place, n.X)
				}
				return true
			}
			ast.Inspect(s.Body, check)
			if rest != nil {
				ast.Inspect(rest, check)
			}
			if used {
				value := g.expr(place, nil, mPlace)
				name := g.newTmp("present")
				g.nonNil = append(g.nonNil, nonNilBinding{place, name})
				cond := "let Some(" + name + ") = " + parenIfNeeded(value) + ".as_ref()"
				if rest != nil {
					cond += " && " + g.expr(rest, types.Typ[types.Bool], mOwned)
				}
				return cond
			}
		}
	}
	return g.expr(s.Cond, types.Typ[types.Bool], mOwned)
}

func (g *gen) readOnlyPlace(e ast.Expr) bool {
	switch e := e.(type) {
	case *ast.Ident:
		obj := g.info.Uses[e]
		return g.vars[obj] != nil && !g.assigned[obj]
	case *ast.SelectorExpr:
		return g.info.Selections[e] != nil && g.info.Selections[e].Kind() == types.FieldVal && g.readOnlyPlace(e.X)
	}
	return false
}

func (g *gen) samePlace(a, b ast.Expr) bool {
	switch a := a.(type) {
	case *ast.Ident:
		b, ok := b.(*ast.Ident)
		return ok && g.info.Uses[a] != nil && g.info.Uses[a] == g.info.Uses[b]
	case *ast.SelectorExpr:
		b, ok := b.(*ast.SelectorExpr)
		return ok && a.Sel.Name == b.Sel.Name && g.samePlace(a.X, b.X)
	}
	return false
}

func (g *gen) borrowedPointer(e ast.Expr) string {
	for i := len(g.nonNil) - 1; i >= 0; i-- {
		if g.samePlace(e, g.nonNil[i].expr) {
			return g.nonNil[i].name
		}
	}
	return ""
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
		borrowed := false
		if id, ok := s.X.(*ast.Ident); ok {
			if vi := g.vars[g.info.Uses[id]]; vi != nil && vi.kind == kSlice && vObj != nil {
				src = vi.name + ".iter().cloned()"
				borrowed = true
			}
		}
		if lit, ok := s.X.(*ast.CompositeLit); ok && vObj != nil {
			var parts []string
			for _, el := range lit.Elts {
				parts = append(parts, g.elem(el, u.Elem()))
			}
			src = "[" + strings.Join(parts, ", ") + "]"
		}
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
			iter := src
			if !borrowed {
				iter = parenIfNeeded(src) + ".into_iter()"
			}
			g.line("for (%s, %s) in %s.enumerate() {", kName, vName, iter)
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
