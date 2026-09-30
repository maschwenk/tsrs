package main

import (
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"path/filepath"
	"sort"
	"strings"
)

// nstate is the three-point lattice used for result positions.
type nstate int

const (
	nsNonNil  nstate = iota // every return path yields a value known to be non-nil
	nsUnknown               // some return path yields a value of unknown nil-ness
	nsNil                   // some return path can yield nil
)

// Slot is one parameter or result position whose Rust type may become Option<…>.
type Slot struct {
	Nilable  bool // the Go type can hold nil (pointer, interface, func, table map)
	Option   bool
	Why      string
	Sub      *SigSlots // nested slots when the position is func-typed (callbacks)
	isResult bool
	State    nstate // results only
	Evidence bool   // results only: some call site compares the result against nil
	EvWhy    string
	// Transparent: Go returns nil early when this parameter is nil; treated as non-nil.
	Transparent bool
	// NilFlow: a literal nil can reach this parameter (rule (a) or forward flow).
	NilFlow     bool
	forced      int // 0: analysis decides, 1: forced non-Option, 2: forced Option (config overrides)
	flowFrom    *flowRec
	cause       *resultSrc
	ucause      *resultSrc // first return that made the state Unknown
	viaEvidence bool
	owner       *Func
}

type SigSlots struct {
	Params   []*Slot
	Results  []*Slot
	Variadic bool
}

// Func is a function/method declaration, or a func-typed struct field called like a method.
type Func struct {
	Obj      types.Object // *types.Func, or *types.Var for fields
	Name     string
	Decl     *ast.FuncDecl
	Sig      *types.Signature
	Slots    *SigSlots
	RecvType *types.TypeName // receiver type (origin), or owning struct for fields
	RecvPtr  bool
	File     string
	Line     int
	IsField  bool
	Pkg      *Package
	External bool // belongs to a result-only package
	Calls    int  // call sites in analyzed main-package code

	// naming (filled by the emitter)
	RustName   string
	Pub        bool
	MergedInto *Func
}

type srcKind int

const (
	srcNonNil  srcKind = iota // composite literal, &T{}, new(T), constants, …
	srcNil                    // literal nil, map miss, zero value
	srcSlot                   // a parameter value or a call result
	srcVar                    // a local variable / parameter (flow-insensitive: all its values)
	srcField                  // a struct field read
	srcAny                    // any of several sources
	srcAll                    // nil only if all sources are nil (core.OrElse, core.Coalesce)
	srcUnknown                // not analyzable (external call, slice element, …)
)

type source struct {
	kind   srcKind
	slot   *Slot
	v      *types.Var
	list   []source
	noInit bool // srcVar: the variable is definitely assigned at this use, ignore its initial value
	soft   bool // srcNil from a map miss or a zero-value helper (core.LastOrNil): not nil provenance
}

type assignRec struct {
	src      source
	callSlot *Slot // set when the value is directly a call result
}

type forwardRec struct {
	pos    token.Pos
	target *Slot
	why    string
}

type flowRec struct {
	target *Slot
	src    source
	why    string
}

type varInfo struct {
	slot         *Slot   // parameter slot, if the var is a parameter with a known slot
	init         *source // initial value (parameter value, or zero value of a `var x *T` / named result)
	initKilled   bool
	zeroBody     *ast.BlockStmt    // function body, for definite-assignment queries on zero-initialized vars
	defExprs     []ast.Expr        // right-hand sides of all plain assignments (for boolean guard variables)
	forcedNonNil bool              // config override: treat as never nil
	da           map[ast.Stmt]bool // definite assignment before each statement (lazily computed)
	assigns      []assignRec
	assignPos    []token.Pos
	comparePos   []token.Pos
	forwards     []forwardRec
}

type fieldInfo struct {
	isNil bool
	why   string
	srcs  []source
}

type edge struct {
	from, to *Slot
	why      string
}

type resultSrc struct {
	slot *Slot
	src  source
	why  string
}

type pendingResult struct {
	slot *Slot
	e    ast.Expr
	why  string
}

type fnCtx struct {
	slots *SigSlots
	named []*types.Var
}

// guardKey identifies a variable or a field path rooted at a variable (links.resolvedType).
type guardKey struct {
	v    *types.Var
	path string
}

type Analyzer struct {
	fset     *token.FileSet
	cfg      *Config
	mainPkg  *types.Package
	nilableF func(t types.Type) bool
	analyzed map[*types.Package]bool

	funcs      map[types.Object]*Func
	order      []*Func // main package, declaration order
	allResults []*Slot

	info        *types.Info
	curExternal bool
	parents     map[ast.Node]ast.Node

	paramSlot map[*types.Var]*Slot
	vars      map[*types.Var]*varInfo
	fields    map[*types.Var]*fieldInfo
	edges     []edge
	results   []resultSrc
	flows     []flowRec
	handled   map[*ast.FuncLit]bool
	zeroGen   map[string]bool
	coalesce  map[string]bool
	implCache map[*types.Func][]*Func
	ifElse    map[string]bool
	funcPass  map[string]bool
	cbCond    map[string]bool
	// return expressions are resolved after the whole package is walked, so callback
	// literals nested inside them already have their own slots
	pendingResults []pendingResult
	litSlots       map[*ast.FuncLit]*SigSlots
	localLit       map[*types.Var]*ast.FuncLit
	pendingLits    []*ast.FuncLit
	depth          int
	boolDepth      int
	// readTolerant reports whether a parameter is never written through (nil maps read fine in Go)
	readTolerant func(v *types.Var) bool

	// nil-transparent parameters: `if p == nil … { return nil/p }` early exits
	skipCompare map[token.Pos]bool
	skipReturn  map[*ast.ReturnStmt]bool
}

func newAnalyzer(fset *token.FileSet, cfg *Config, main *types.Package, nilable func(types.Type) bool) *Analyzer {
	a := &Analyzer{
		fset: fset, cfg: cfg, mainPkg: main, nilableF: nilable,
		analyzed:  map[*types.Package]bool{},
		funcs:     map[types.Object]*Func{},
		parents:   map[ast.Node]ast.Node{},
		paramSlot: map[*types.Var]*Slot{},
		vars:      map[*types.Var]*varInfo{},
		fields:    map[*types.Var]*fieldInfo{},
		handled:   map[*ast.FuncLit]bool{},
		zeroGen:   map[string]bool{},
		coalesce:  map[string]bool{},
		implCache: map[*types.Func][]*Func{},
		ifElse:    map[string]bool{"core.IfElse": true},
		funcPass:  map[string]bool{},
		cbCond:    map[string]bool{},
		litSlots:  map[*ast.FuncLit]*SigSlots{},
		localLit:  map[*types.Var]*ast.FuncLit{},

		skipCompare: map[token.Pos]bool{},
		skipReturn:  map[*ast.ReturnStmt]bool{},
	}
	for _, z := range cfg.ZeroValueGenerics {
		a.zeroGen[z] = true
	}
	for _, z := range cfg.CoalesceGenerics {
		a.coalesce[z] = true
	}
	for _, z := range cfg.FuncPassthrough {
		a.funcPass[z] = true
	}
	for _, z := range cfg.CallbackConditionalNil {
		a.cbCond[z] = true
	}
	return a
}

// newSigSlots creates slots for sig. resultState is the initial state of result slots:
// nsNonNil when a body will be analyzed, nsUnknown for bodiless positions (fields, callbacks).
func (a *Analyzer) newSigSlots(sig *types.Signature, resultState nstate) *SigSlots {
	s := &SigSlots{Variadic: sig.Variadic()}
	n := sig.Params().Len()
	for i := 0; i < n; i++ {
		t := sig.Params().At(i).Type()
		sl := &Slot{}
		if !(s.Variadic && i == n-1) {
			sl.Nilable = a.nilableF(t)
			if _, isMap := t.Underlying().(*types.Map); isMap {
				sl.Nilable = true // nil map arguments: Option<&FxHashMap<…>> when passed/compared
			}
			if fs, ok := t.Underlying().(*types.Signature); ok {
				// callbacks: nil-ness comes from the literals / function values passed
				sl.Sub = a.newSigSlots(fs, nsNonNil)
			}
		}
		s.Params = append(s.Params, sl)
	}
	for i := 0; i < sig.Results().Len(); i++ {
		t := sig.Results().At(i).Type()
		sl := &Slot{Nilable: a.nilableF(t), isResult: true, State: resultState}
		if fs, ok := t.Underlying().(*types.Signature); ok {
			sl.Sub = a.newSigSlots(fs, nsUnknown)
		}
		s.Results = append(s.Results, sl)
		a.allResults = append(a.allResults, sl)
	}
	return s
}

func ownerName(s *Slot) string {
	if s.owner == nil {
		return "(callback)"
	}
	return s.owner.Name
}

func setOwner(s *SigSlots, fn *Func) {
	for _, x := range s.Params {
		x.owner = fn
	}
	for _, x := range s.Results {
		x.owner = fn
	}
}

func (a *Analyzer) pos(p token.Pos) string {
	pp := a.fset.Position(p)
	return fmt.Sprintf("%s:%d", filepath.Base(pp.Filename), pp.Line)
}

// collect registers every function declaration and func-typed struct field of pkg.
func (a *Analyzer) collect(pkg *Package, external bool) {
	a.analyzed[pkg.Types] = true
	for fi, f := range pkg.Files {
		fname := pkg.Names[fi]
		for _, d := range f.Decls {
			switch d := d.(type) {
			case *ast.FuncDecl:
				obj, _ := pkg.Info.Defs[d.Name].(*types.Func)
				if obj == nil {
					continue
				}
				sig := obj.Type().(*types.Signature)
				st := nsNonNil
				if d.Body == nil {
					st = nsUnknown
				}
				fn := &Func{Obj: obj, Name: d.Name.Name, Decl: d, Sig: sig, Slots: a.newSigSlots(sig, st),
					File: fname, Line: a.fset.Position(d.Pos()).Line, Pkg: pkg, External: external}
				if r := sig.Recv(); r != nil {
					t := r.Type()
					if p, ok := t.(*types.Pointer); ok {
						fn.RecvPtr = true
						t = p.Elem()
					}
					if n, ok := types.Unalias(t).(*types.Named); ok {
						fn.RecvType = n.Origin().Obj()
					}
				}
				a.funcs[obj] = fn
				setOwner(fn.Slots, fn)
				if !external {
					a.order = append(a.order, fn)
				}
			case *ast.GenDecl:
				if d.Tok != token.TYPE {
					continue
				}
				for _, sp := range d.Specs {
					ts := sp.(*ast.TypeSpec)
					tn, _ := pkg.Info.Defs[ts.Name].(*types.TypeName)
					if tn == nil {
						continue
					}
					st, ok := tn.Type().Underlying().(*types.Struct)
					if !ok {
						continue
					}
					for i := 0; i < st.NumFields(); i++ {
						fv := st.Field(i)
						sig, ok := fv.Type().Underlying().(*types.Signature)
						if !ok {
							continue
						}
						st := nsNonNil
						if tn.Name() == a.cfg.CheckerType {
							st = nsUnknown // no body: call-site evidence decides
						}
						fn := &Func{Obj: fv, Name: fv.Name(), Sig: sig, Slots: a.newSigSlots(sig, st), RecvType: tn,
							RecvPtr: true, File: fname, Line: a.fset.Position(fv.Pos()).Line, IsField: true,
							Pkg: pkg, External: external}
						a.funcs[fv] = fn
						setOwner(fn.Slots, fn)
						if !external {
							a.order = append(a.order, fn)
						}
					}
				}
			}
		}
	}
}

// walkPackage analyzes every function body of pkg except those in skipped files.
func (a *Analyzer) walkPackage(pkg *Package, external bool, skip func(string) bool) {
	a.info = pkg.Info
	a.curExternal = external
	defer a.resolvePending()
	for fi, f := range pkg.Files {
		if skip != nil && skip(pkg.Names[fi]) {
			continue
		}
		var stack []ast.Node
		ast.Inspect(f, func(n ast.Node) bool {
			if n == nil {
				stack = stack[:len(stack)-1]
				return true
			}
			if len(stack) > 0 {
				a.parents[n] = stack[len(stack)-1]
			}
			stack = append(stack, n)
			return true
		})
		for _, d := range f.Decls {
			switch d := d.(type) {
			case *ast.FuncDecl:
				obj, _ := pkg.Info.Defs[d.Name].(*types.Func)
				fn := a.funcs[obj]
				if fn == nil {
					continue
				}
				if a.cfg.NilTransparent && !external {
					a.detectTransparent(fn)
				}
				a.walkFunc(d.Type, d.Body, fn.Slots)
			case *ast.GenDecl:
				if d.Tok == token.VAR {
					for _, sp := range d.Specs {
						for _, v := range sp.(*ast.ValueSpec).Values {
							a.walk(v, &fnCtx{})
						}
					}
				}
			}
		}
	}
}

// detectTransparent finds nil-in/nil-out parameters: a leading
// `if p == nil [|| …] { return nil }` (or `return p`) in a function with a single pointer
// result, where that condition holds the only nil comparisons of p. Such p is treated as
// non-nil (callers holding an Option map over the call); the early exit is ignored.
func (a *Analyzer) detectTransparent(fn *Func) {
	d := fn.Decl
	if d.Body == nil || len(fn.Slots.Results) != 1 || !fn.Slots.Results[0].Nilable {
		return
	}
	params := map[*types.Var]int{}
	for i := 0; i < fn.Sig.Params().Len(); i++ {
		params[fn.Sig.Params().At(i)] = i
	}
	// Positive form: `if p != nil && … { … }` … `return p` as the last statement.
	if n := len(d.Body.List); n > 0 {
		if ret, ok := d.Body.List[n-1].(*ast.ReturnStmt); ok && len(ret.Results) == 1 {
			if id, ok := unparen(ret.Results[0]).(*ast.Ident); ok {
				if v, ok := a.info.Uses[id].(*types.Var); ok {
					if idx, isParam := params[v]; isParam && fn.Slots.Params[idx].Nilable && a.onlyPositiveGuards(d.Body, v) {
						fn.Slots.Params[idx].Transparent = true
						a.markSkipCompares(d.Body, v)
					}
				}
			}
		}
	}
	for _, st := range d.Body.List {
		ifs, ok := st.(*ast.IfStmt)
		if !ok {
			break
		}
		if ifs.Init != nil || ifs.Else != nil || len(ifs.Body.List) != 1 {
			continue
		}
		ret, ok := ifs.Body.List[0].(*ast.ReturnStmt)
		if !ok || len(ret.Results) != 1 {
			continue
		}
		var retVar *types.Var
		retNil := a.isNilLit(ret.Results[0])
		if id, ok := unparen(ret.Results[0]).(*ast.Ident); ok {
			retVar, _ = a.info.Uses[id].(*types.Var)
		}
		for _, term := range a.disjuncts(ifs.Cond) {
			b, ok := unparen(term).(*ast.BinaryExpr)
			if !ok || b.Op != token.EQL || !a.isNilLit(b.Y) {
				continue
			}
			id, ok := unparen(b.X).(*ast.Ident)
			if !ok {
				continue
			}
			v, _ := a.info.Uses[id].(*types.Var)
			idx, isParam := params[v]
			if !isParam || !fn.Slots.Params[idx].Nilable || !(retNil || retVar == v) {
				continue
			}
			if a.countNilCompares(d.Body, v) != a.countNilCompares(ifs.Cond, v) {
				continue
			}
			fn.Slots.Params[idx].Transparent = true
			a.markSkipCompares(ifs.Cond, v)
			if retNil {
				a.skipReturn[ret] = true
			}
		}
	}
}

// onlyPositiveGuards: every nil comparison of v is a `v != nil` conjunct of the condition of
// a top-level if statement in body, and v is never assigned.
func (a *Analyzer) onlyPositiveGuards(body *ast.BlockStmt, v *types.Var) bool {
	total := a.countNilCompares(body, v)
	if total == 0 {
		return false
	}
	guarded := 0
	for _, st := range body.List {
		ifs, ok := st.(*ast.IfStmt)
		if !ok {
			continue
		}
		for _, c := range a.conjuncts(ifs.Cond) {
			if b, ok := unparen(c).(*ast.BinaryExpr); ok && b.Op == token.NEQ && a.isNilLit(b.Y) {
				if id, ok := unparen(b.X).(*ast.Ident); ok && a.info.Uses[id] == v {
					guarded++
				}
			}
		}
	}
	if guarded != total {
		return false
	}
	assigned := false
	ast.Inspect(body, func(x ast.Node) bool {
		switch s := x.(type) {
		case *ast.AssignStmt:
			for _, l := range s.Lhs {
				if id, ok := unparen(l).(*ast.Ident); ok && a.info.Uses[id] == v {
					assigned = true
				}
			}
		case *ast.UnaryExpr:
			if id, ok := unparen(s.X).(*ast.Ident); ok && s.Op == token.AND && a.info.Uses[id] == v {
				assigned = true
			}
		}
		return !assigned
	})
	return !assigned
}

func (a *Analyzer) conjuncts(e ast.Expr) []ast.Expr {
	if b, ok := unparen(e).(*ast.BinaryExpr); ok && b.Op == token.LAND {
		return append(a.conjuncts(b.X), a.conjuncts(b.Y)...)
	}
	return []ast.Expr{e}
}

func (a *Analyzer) disjuncts(e ast.Expr) []ast.Expr {
	if b, ok := unparen(e).(*ast.BinaryExpr); ok && b.Op == token.LOR {
		return append(a.disjuncts(b.X), a.disjuncts(b.Y)...)
	}
	return []ast.Expr{e}
}

func (a *Analyzer) nilCompareIdents(n ast.Node, v *types.Var, f func(*ast.Ident)) {
	ast.Inspect(n, func(x ast.Node) bool {
		b, ok := x.(*ast.BinaryExpr)
		if !ok || (b.Op != token.EQL && b.Op != token.NEQ) {
			return true
		}
		for _, pair := range [][2]ast.Expr{{b.X, b.Y}, {b.Y, b.X}} {
			if id, ok := unparen(pair[0]).(*ast.Ident); ok && a.isNilLit(pair[1]) && a.info.Uses[id] == v {
				f(id)
			}
		}
		return true
	})
}

func (a *Analyzer) countNilCompares(n ast.Node, v *types.Var) int {
	c := 0
	a.nilCompareIdents(n, v, func(*ast.Ident) { c++ })
	return c
}

func (a *Analyzer) markSkipCompares(n ast.Node, v *types.Var) {
	a.nilCompareIdents(n, v, func(id *ast.Ident) { a.skipCompare[id.Pos()] = true })
}

func (a *Analyzer) varInfo(v *types.Var) *varInfo {
	vi := a.vars[v]
	if vi == nil {
		vi = &varInfo{}
		a.vars[v] = vi
	}
	return vi
}

func (a *Analyzer) fieldInfo(v *types.Var) *fieldInfo {
	fi := a.fields[v]
	if fi == nil {
		fi = &fieldInfo{}
		a.fields[v] = fi
	}
	return fi
}

// mark makes a slot Option directly (parameters, callback positions).
func (a *Analyzer) mark(s *Slot, why string) {
	if s == nil || !s.Nilable || s.Option || s.forced == 1 {
		return
	}
	s.Option = true
	s.Why = why
	if s.isResult {
		s.State = nsNil
	}
}

// evidence records that a call site compares a result against nil.
func (a *Analyzer) evidence(s *Slot, why string) {
	if s != nil && s.Nilable && !s.Evidence {
		s.Evidence = true
		s.EvWhy = why
	}
}

func (a *Analyzer) walkFunc(ftype *ast.FuncType, body *ast.BlockStmt, slots *SigSlots) {
	if body == nil {
		return
	}
	ctx := &fnCtx{slots: slots}
	idx := 0
	for _, field := range ftype.Params.List {
		if len(field.Names) == 0 {
			idx++
			continue
		}
		for _, nm := range field.Names {
			if v, ok := a.info.Defs[nm].(*types.Var); ok && v != nil && slots != nil && idx < len(slots.Params) {
				vi := a.varInfo(v)
				vi.slot = slots.Params[idx]
				vi.init = &source{kind: srcSlot, slot: slots.Params[idx]}
				a.paramSlot[v] = slots.Params[idx]
			}
			idx++
		}
	}
	if ftype.Results != nil {
		for _, field := range ftype.Results.List {
			if len(field.Names) == 0 {
				ctx.named = append(ctx.named, nil)
				continue
			}
			for _, nm := range field.Names {
				v, _ := a.info.Defs[nm].(*types.Var)
				if v == nil || nm.Name == "_" {
					ctx.named = append(ctx.named, nil)
					continue
				}
				ctx.named = append(ctx.named, v)
				vi := a.varInfo(v)
				vi.init = &source{kind: srcNil}
				vi.initKilled = a.firstUseIsAssign(v, body.List)
				vi.zeroBody = body
			}
		}
	}
	a.walk(body, ctx)
}

func unparen(e ast.Expr) ast.Expr {
	for {
		p, ok := e.(*ast.ParenExpr)
		if !ok {
			return e
		}
		e = p.X
	}
}

func (a *Analyzer) isNilLit(e ast.Expr) bool {
	tv, ok := a.info.Types[unparen(e)]
	return ok && tv.IsNil()
}

func (a *Analyzer) walk(n ast.Node, ctx *fnCtx) {
	ast.Inspect(n, func(x ast.Node) bool {
		switch x := x.(type) {
		case *ast.FuncLit:
			if a.handled[x] {
				return false
			}
			if v := a.litVar(x); v != nil {
				// `f := func(…) {…}`: analyzed when f is passed as a callback (or at the end)
				a.localLit[v] = x
				a.pendingLits = append(a.pendingLits, x)
				return false
			}
			a.walkFunc(x.Type, x.Body, nil)
			return false
		case *ast.BlockStmt:
			a.stmtList(x.List)
		case *ast.CaseClause:
			a.stmtList(x.Body)
		case *ast.CommClause:
			a.stmtList(x.Body)
		case *ast.CompositeLit:
			a.compositeLit(x)
		case *ast.CallExpr:
			a.call(x)
		case *ast.ReturnStmt:
			a.ret(x, ctx)
		case *ast.AssignStmt:
			a.assign(x)
		case *ast.ValueSpec:
			a.valueSpec(x)
		case *ast.IncDecStmt:
			a.touch(x.X)
		case *ast.RangeStmt:
			if x.Tok == token.ASSIGN {
				if x.Key != nil {
					a.touch(x.Key)
				}
				if x.Value != nil {
					a.touch(x.Value)
				}
			}
		case *ast.UnaryExpr:
			if x.Op == token.AND {
				a.touch(x.X)
			}
		case *ast.BinaryExpr:
			if x.Op == token.EQL || x.Op == token.NEQ {
				if a.isNilLit(x.Y) {
					a.comparedToNil(x.X)
				} else if a.isNilLit(x.X) {
					a.comparedToNil(x.Y)
				}
			}
		case *ast.SwitchStmt:
			if x.Tag != nil {
				for _, st := range x.Body.List {
					for _, e := range st.(*ast.CaseClause).List {
						if a.isNilLit(e) {
							a.comparedToNil(x.Tag)
						}
					}
				}
			}
		}
		return true
	})
}

// stmtList applies the "zero value killed by an immediate assignment" heuristic to
// `var x *T` declarations: the zero value only counts as a nil source if the first
// statement mentioning x is not a plain top-level `x = …` in the same block.
func (a *Analyzer) stmtList(list []ast.Stmt) {
	for i, st := range list {
		ds, ok := st.(*ast.DeclStmt)
		if !ok {
			continue
		}
		gd, ok := ds.Decl.(*ast.GenDecl)
		if !ok || gd.Tok != token.VAR {
			continue
		}
		for _, sp := range gd.Specs {
			vs := sp.(*ast.ValueSpec)
			if len(vs.Values) != 0 {
				continue
			}
			for _, nm := range vs.Names {
				if v, ok := a.info.Defs[nm].(*types.Var); ok && v != nil {
					if a.firstUseIsAssign(v, list[i+1:]) {
						a.varInfo(v).initKilled = true
					}
				}
			}
		}
	}
}

func (a *Analyzer) mentions(n ast.Node, v *types.Var) bool {
	found := false
	ast.Inspect(n, func(x ast.Node) bool {
		if found {
			return false
		}
		if id, ok := x.(*ast.Ident); ok && a.info.Uses[id] == v {
			found = true
		}
		return true
	})
	return found
}

func (a *Analyzer) firstUseIsAssign(v *types.Var, stmts []ast.Stmt) bool {
	for _, st := range stmts {
		if !a.mentions(st, v) {
			continue
		}
		switch st.(type) {
		case *ast.SwitchStmt, *ast.TypeSwitchStmt, *ast.IfStmt:
			ok, _ := a.establishes([]ast.Stmt{st}, guardKey{v: v})
			return ok
		}
		as, ok := st.(*ast.AssignStmt)
		if !ok || as.Tok != token.ASSIGN {
			return false
		}
		for _, r := range as.Rhs {
			if a.mentions(r, v) {
				return false
			}
		}
		for _, l := range as.Lhs {
			if id, ok := unparen(l).(*ast.Ident); ok && a.info.Uses[id] == v {
				return true
			}
		}
		return false
	}
	return false
}

func (a *Analyzer) touch(e ast.Expr) {
	if id, ok := unparen(e).(*ast.Ident); ok {
		if v, ok := a.info.Uses[id].(*types.Var); ok {
			vi := a.varInfo(v)
			vi.assignPos = append(vi.assignPos, id.Pos())
		}
	}
}

func (a *Analyzer) comparedToNil(e ast.Expr) {
	switch o := unparen(e).(type) {
	case *ast.Ident:
		if v, ok := a.info.Uses[o].(*types.Var); ok && !v.IsField() && !a.skipCompare[o.Pos()] {
			vi := a.varInfo(v)
			vi.comparePos = append(vi.comparePos, o.Pos())
		}
	case *ast.CallExpr:
		if slots, _, _ := a.callee(o); slots != nil && len(slots.Results) >= 1 {
			a.evidence(slots.Results[0], "call result compared to nil at "+a.pos(o.Pos()))
		}
	case *ast.SelectorExpr:
		if sel := a.info.Selections[o]; sel != nil && sel.Kind() == types.FieldVal {
			fi := a.fieldInfo(sel.Obj().(*types.Var).Origin())
			if !fi.isNil {
				fi.isNil = true
				fi.why = "field compared to nil at " + a.pos(o.Pos())
			}
		}
	}
}

// callee resolves the called function's slots. recvOffset is 1 for method expressions.
func (a *Analyzer) callee(call *ast.CallExpr) (*SigSlots, *Func, int) {
	fun := unparen(call.Fun)
	switch f := fun.(type) {
	case *ast.IndexExpr:
		fun = unparen(f.X)
	case *ast.IndexListExpr:
		fun = unparen(f.X)
	}
	switch f := fun.(type) {
	case *ast.Ident:
		switch o := a.info.Uses[f].(type) {
		case *types.Func:
			if fn := a.funcs[o.Origin()]; fn != nil {
				return fn.Slots, fn, 0
			}
		case *types.Var:
			if s := a.paramSlot[o]; s != nil && s.Sub != nil {
				return s.Sub, nil, 0
			}
		}
	case *ast.SelectorExpr:
		if sel := a.info.Selections[f]; sel != nil {
			switch o := sel.Obj().(type) {
			case *types.Func:
				if fn := a.funcs[o.Origin()]; fn != nil {
					off := 0
					if sel.Kind() == types.MethodExpr {
						off = 1
					}
					return fn.Slots, fn, off
				}
			case *types.Var:
				if fn := a.funcs[o.Origin()]; fn != nil {
					return fn.Slots, fn, 0
				}
			}
		} else if o, ok := a.info.Uses[f.Sel].(*types.Func); ok {
			if fn := a.funcs[o.Origin()]; fn != nil {
				return fn.Slots, fn, 0
			}
		}
	}
	return nil, nil, 0
}

// implementations resolves a call of an interface method declared in an analyzed package
// to the analyzed concrete methods that can be behind it.
func (a *Analyzer) implementations(call *ast.CallExpr) []*Func {
	sel, ok := unparen(call.Fun).(*ast.SelectorExpr)
	if !ok {
		return nil
	}
	s := a.info.Selections[sel]
	if s == nil || s.Kind() != types.MethodVal {
		return nil
	}
	m, _ := s.Obj().(*types.Func)
	if m == nil {
		return nil
	}
	recv := m.Type().(*types.Signature).Recv()
	if recv == nil {
		return nil
	}
	iface, ok := recv.Type().Underlying().(*types.Interface)
	if !ok || m.Pkg() == nil || !a.analyzed[m.Pkg()] {
		return nil
	}
	if impls, ok := a.implCache[m]; ok {
		return impls
	}
	var impls []*Func
	for pkg := range a.analyzed {
		for _, name := range pkg.Scope().Names() {
			tn, ok := pkg.Scope().Lookup(name).(*types.TypeName)
			if !ok || tn.IsAlias() {
				continue
			}
			if _, isIface := tn.Type().Underlying().(*types.Interface); isIface {
				continue
			}
			ptr := types.NewPointer(tn.Type())
			if !types.Implements(ptr, iface) {
				continue
			}
			obj, _, _ := types.LookupFieldOrMethod(ptr, true, m.Pkg(), m.Name())
			if f, ok := obj.(*types.Func); ok {
				if fn := a.funcs[f.Origin()]; fn != nil {
					impls = append(impls, fn)
				}
			}
		}
	}
	sort.Slice(impls, func(i, j int) bool { return impls[i].Obj.Pos() < impls[j].Obj.Pos() })
	a.implCache[m] = impls
	return impls
}

func (a *Analyzer) isInterfaceCall(call *ast.CallExpr) bool {
	sel, ok := unparen(call.Fun).(*ast.SelectorExpr)
	if !ok {
		return false
	}
	s := a.info.Selections[sel]
	return s != nil && s.Kind() == types.MethodVal && types.IsInterface(s.Recv())
}

// calledFunc returns the static *types.Func of a call, analyzed or not.
func (a *Analyzer) calledFunc(call *ast.CallExpr) *types.Func {
	fun := unparen(call.Fun)
	switch f := fun.(type) {
	case *ast.IndexExpr:
		fun = unparen(f.X)
	case *ast.IndexListExpr:
		fun = unparen(f.X)
	}
	var id *ast.Ident
	switch f := fun.(type) {
	case *ast.Ident:
		id = f
	case *ast.SelectorExpr:
		id = f.Sel
	}
	if id == nil {
		return nil
	}
	fn, _ := a.info.Uses[id].(*types.Func)
	if fn != nil {
		fn = fn.Origin()
	}
	return fn
}

func funcKey(f *types.Func) string {
	if f.Pkg() == nil {
		return f.Name()
	}
	return f.Pkg().Name() + "." + f.Name()
}

// genericPassthrough returns the arguments whose parameter type is the type parameter
// that is also the (single) result type, e.g. core.IfElse(c, a, b) -> [a, b].
func (a *Analyzer) genericPassthrough(call *ast.CallExpr, f *types.Func) []ast.Expr {
	sig := f.Type().(*types.Signature)
	if sig.Results().Len() != 1 {
		return nil
	}
	tp, ok := sig.Results().At(0).Type().(*types.TypeParam)
	if !ok {
		return nil
	}
	var out []ast.Expr
	for i, arg := range call.Args {
		if i >= sig.Params().Len() {
			break
		}
		if pt, ok := sig.Params().At(i).Type().(*types.TypeParam); ok && pt == tp {
			out = append(out, arg)
		}
	}
	return out
}

func (a *Analyzer) isNilish(e ast.Expr) bool {
	e = unparen(e)
	if a.isNilLit(e) {
		return true
	}
	if call, ok := e.(*ast.CallExpr); ok {
		if f := a.calledFunc(call); f != nil {
			for _, arg := range a.genericPassthrough(call, f) {
				if a.isNilish(arg) {
					return true
				}
			}
		}
	}
	return false
}

func (a *Analyzer) exprSource(e ast.Expr) source { return a.exprSourceAt(e, nil) }

// exprSourceAt describes where the value of e comes from. When at is non-nil, nil guards
// that dominate `at` (if x != nil {…}, if x == nil { return }, …) are honored.
func (a *Analyzer) exprSourceAt(e ast.Expr, at ast.Node) source {
	a.depth++
	defer func() { a.depth-- }()
	if a.depth > 12 {
		at = nil
	}
	e = unparen(e)
	if a.isNilLit(e) {
		return source{kind: srcNil}
	}
	if at != nil {
		if k, ok := a.keyOf(e); ok {
			if guarded, extras := a.guard(k, at); guarded {
				return a.sourcesOf(extras)
			}
		}
	}
	tv := a.info.Types[e]
	if tv.Type != nil && !a.nilableF(tv.Type) {
		return source{kind: srcNonNil}
	}
	switch x := e.(type) {
	case *ast.Ident:
		if v, ok := a.info.Uses[x].(*types.Var); ok && !v.IsField() {
			if v.Parent() != nil && v.Pkg() != nil && v.Parent() == v.Pkg().Scope() {
				return source{kind: srcUnknown}
			}
			return source{kind: srcVar, v: v, noInit: a.definitelyAssigned(v, x)}
		}
		return source{kind: srcNonNil}
	case *ast.CallExpr:
		if ftv, ok := a.info.Types[x.Fun]; ok && ftv.IsType() {
			if len(x.Args) == 1 {
				return a.exprSourceAt(x.Args[0], at)
			}
			return source{kind: srcUnknown}
		}
		if ftv, ok := a.info.Types[x.Fun]; ok && ftv.IsBuiltin() {
			return source{kind: srcNonNil} // new(T), append, …
		}
		var list []source
		f := a.calledFunc(x)
		genericResult := false
		if f != nil {
			if sig := f.Type().(*types.Signature); sig.Results().Len() > 0 {
				_, genericResult = sig.Results().At(0).Type().(*types.TypeParam)
			}
		}
		if slots, fn, _ := a.callee(x); slots != nil && len(slots.Results) >= 1 && !genericResult {
			if cb, ok := a.callbackResult(x, fn); ok {
				list = append(list, cb)
			} else {
				list = append(list, source{kind: srcSlot, slot: slots.Results[0]})
			}
		} else if f == nil && slots == nil && !a.isInterfaceCall(x) {
			// dynamic call of a func value (thunk slices, locals): non-nil by convention
			list = append(list, source{kind: srcNonNil})
		} else if impls := a.implementations(x); len(impls) > 0 {
			for _, fn := range impls {
				if len(fn.Slots.Results) >= 1 {
					list = append(list, source{kind: srcSlot, slot: fn.Slots.Results[0]})
				}
			}
		}
		if f != nil {
			if a.zeroGen[funcKey(f)] {
				return source{kind: srcNil, soft: true}
			}
			var pass []source
			if a.ifElse[funcKey(f)] && len(x.Args) == 3 {
				for i, arg := range x.Args[1:] {
					if k, ok := a.keyOf(arg); ok && a.condImplies(x.Args[0], k, i == 0) {
						pass = append(pass, source{kind: srcNonNil})
					} else {
						pass = append(pass, a.exprSourceAt(arg, at))
					}
				}
			} else {
				for _, arg := range a.genericPassthrough(x, f) {
					pass = append(pass, a.exprSourceAt(arg, at))
				}
				if genericResult && len(pass) == 0 {
					pass = append(pass, source{kind: srcUnknown})
				}
			}
			if a.coalesce[funcKey(f)] && len(pass) > 0 {
				list = append(list, source{kind: srcAll, list: pass})
			} else {
				list = append(list, pass...)
			}
		}
		switch len(list) {
		case 0:
			return source{kind: srcUnknown}
		case 1:
			return list[0]
		}
		return source{kind: srcAny, list: list}
	case *ast.SelectorExpr:
		if sel := a.info.Selections[x]; sel != nil {
			if sel.Kind() == types.FieldVal {
				return source{kind: srcField, v: sel.Obj().(*types.Var).Origin()}
			}
			return source{kind: srcNonNil} // method value
		}
		if v, ok := a.info.Uses[x.Sel].(*types.Var); ok && v != nil {
			return source{kind: srcUnknown} // package-level var of another package
		}
		return source{kind: srcNonNil}
	case *ast.IndexExpr:
		if tv, ok := a.info.Types[x.X]; ok {
			if _, isMap := tv.Type.Underlying().(*types.Map); isMap {
				return source{kind: srcNil, soft: true}
			}
		}
		// Slice/array elements: []*T maps to &[P<T>], elements are non-nil by convention.
		return source{kind: srcNonNil}
	case *ast.CompositeLit, *ast.FuncLit, *ast.BasicLit:
		return source{kind: srcNonNil}
	case *ast.UnaryExpr:
		if x.Op == token.AND {
			return source{kind: srcNonNil}
		}
	case *ast.TypeAssertExpr:
		return a.exprSourceAt(x.X, at)
	}
	return source{kind: srcUnknown}
}

// keyOf returns the guard key of a variable or a field path rooted at a variable.
func (a *Analyzer) keyOf(e ast.Expr) (guardKey, bool) {
	switch x := unparen(e).(type) {
	case *ast.Ident:
		if v, ok := a.info.Uses[x].(*types.Var); ok && v != nil {
			return guardKey{v: v}, true
		}
	case *ast.SelectorExpr:
		sel := a.info.Selections[x]
		if sel == nil || sel.Kind() != types.FieldVal {
			return guardKey{}, false
		}
		k, ok := a.keyOf(x.X)
		if !ok {
			return guardKey{}, false
		}
		k.path += "." + x.Sel.Name
		return k, true
	}
	return guardKey{}, false
}

func (a *Analyzer) matchKey(e ast.Expr, k guardKey) bool {
	k2, ok := a.keyOf(e)
	return ok && k2 == k
}

// clobbers reports whether an assignment target e overwrites k (same key or a prefix of it).
func (a *Analyzer) clobbers(e ast.Expr, k guardKey) bool {
	k2, ok := a.keyOf(e)
	return ok && k2.v == k.v && (k2.path == k.path || strings.HasPrefix(k.path, k2.path+"."))
}

// condImplies reports whether cond (when it evaluates to `when`) implies k != nil.
func (a *Analyzer) condImplies(cond ast.Expr, k guardKey, when bool) bool {
	switch c := unparen(cond).(type) {
	case *ast.BinaryExpr:
		switch c.Op {
		case token.LAND:
			if when {
				return a.condImplies(c.X, k, true) || a.condImplies(c.Y, k, true)
			}
		case token.LOR:
			if !when {
				return a.condImplies(c.X, k, false) || a.condImplies(c.Y, k, false)
			}
		case token.NEQ, token.EQL:
			var other ast.Expr
			if a.isNilLit(c.Y) {
				other = c.X
			} else if a.isNilLit(c.X) {
				other = c.Y
			}
			if other == nil || !a.matchKey(other, k) {
				return false
			}
			return (c.Op == token.NEQ) == when
		}
	case *ast.UnaryExpr:
		if c.Op == token.NOT {
			return a.condImplies(c.X, k, !when)
		}
	case *ast.Ident:
		// `ok := k != nil && …; if ok { … }`: expand a boolean variable assigned once
		if v, isVar := a.info.Uses[c].(*types.Var); isVar && a.boolDepth < 4 {
			if vi := a.vars[v]; vi != nil && len(vi.defExprs) == 1 && vi.defExprs[0] != nil && len(vi.assignPos) <= 1 {
				a.boolDepth++
				defer func() { a.boolDepth-- }()
				return a.condImplies(vi.defExprs[0], k, when)
			}
		}
	}
	return false
}

func (a *Analyzer) terminates(s ast.Stmt) bool {
	switch s := s.(type) {
	case *ast.ReturnStmt, *ast.BranchStmt:
		return true
	case *ast.ExprStmt:
		if call, ok := s.X.(*ast.CallExpr); ok {
			if id, ok := unparen(call.Fun).(*ast.Ident); ok && id.Name == "panic" {
				return true
			}
		}
	case *ast.BlockStmt:
		return len(s.List) > 0 && a.terminates(s.List[len(s.List)-1])
	case *ast.IfStmt:
		return s.Else != nil && a.terminates(s.Body) && a.terminates(s.Else)
	}
	return false
}

// assignedBetween reports whether k is (re)assigned in n at a position in (from, to).
func (a *Analyzer) assignedBetween(n ast.Node, k guardKey, from, to token.Pos) bool {
	if k.path == "" {
		if vi := a.vars[k.v]; vi != nil {
			for _, p := range vi.assignPos {
				if p > from && p < to {
					return true
				}
			}
		}
	}
	found := false
	ast.Inspect(n, func(x ast.Node) bool {
		if found || x == nil {
			return false
		}
		if x.End() <= from || x.Pos() >= to {
			return false
		}
		switch s := x.(type) {
		case *ast.AssignStmt:
			for _, l := range s.Lhs {
				if l.Pos() > from && l.Pos() < to && a.clobbers(l, k) {
					found = true
				}
			}
		case *ast.UnaryExpr:
			if s.Op == token.AND && s.Pos() > from && s.Pos() < to && a.clobbers(s.X, k) {
				found = true
			}
		}
		return true
	})
	return found
}

// guard reports whether k is known non-nil at node `at`. extras lists values k may hold
// instead (from `if k == nil { k = v }` patterns); nil extras means plainly non-nil.
func (a *Analyzer) guard(k guardKey, at ast.Node) (bool, []ast.Expr) {
	child := at
	for p := a.parents[at]; p != nil; child, p = p, a.parents[p] {
		switch p := p.(type) {
		case *ast.FuncDecl:
			return false, nil
		case *ast.FuncLit:
			// Guards outside a closure still hold inside it (callbacks run synchronously)
			// unless the closure itself assigns k.
			if ok, _ := a.valuesBetween(p.Body, k, p.Body.Pos(), p.Body.End()); !ok || len(a.assignedValues(p.Body, k)) > 0 {
				return false, nil
			}
		case *ast.IfStmt:
			if child == p.Body && a.condImplies(p.Cond, k, true) {
				return a.valuesBetween(p.Body, k, p.Cond.End(), at.Pos())
			}
			if child == p.Else && a.condImplies(p.Cond, k, false) {
				return a.valuesBetween(p.Else, k, p.Else.Pos(), at.Pos())
			}
		case *ast.ForStmt:
			if child == p.Body && p.Cond != nil && a.condImplies(p.Cond, k, true) {
				return a.valuesBetween(p.Body, k, p.Body.Pos(), at.Pos())
			}
		case *ast.BinaryExpr:
			if child == p.Y && p.Op == token.LAND && a.condImplies(p.X, k, true) {
				return true, nil
			}
			if child == p.Y && p.Op == token.LOR && a.condImplies(p.X, k, false) {
				return true, nil
			}
		case *ast.CaseClause:
			if sw, ok := a.parents[a.parents[p]].(*ast.SwitchStmt); ok && sw.Tag == nil {
				if len(p.List) == 1 && a.condImplies(p.List[0], k, true) {
					return a.valuesBetween(p, k, p.Colon, at.Pos())
				}
				// earlier cases of a tagless switch are known false here
				for _, cl := range sw.Body.List {
					if cl == ast.Stmt(p) {
						break
					}
					cc := cl.(*ast.CaseClause)
					for _, e := range cc.List {
						if a.condImplies(e, k, false) {
							return a.valuesBetween(p, k, p.Colon, at.Pos())
						}
					}
				}
			}
			if ok, extras := a.guardInList(p.Body, child, k, at); ok {
				return true, extras
			}
		case *ast.BlockStmt:
			if ok, extras := a.guardInList(p.List, child, k, at); ok {
				return true, extras
			}
		case *ast.CommClause:
			if ok, extras := a.guardInList(p.Body, child, k, at); ok {
				return true, extras
			}
		}
	}
	return false, nil
}

// establishes reports whether running stmts leaves k non-nil (or leaves the function):
// the list terminates, or its last statement mentioning k assigns it (extras = the value),
// or is an if/else whose branches both establish k.
func (a *Analyzer) establishes(stmts []ast.Stmt, k guardKey) (bool, []ast.Expr) {
	if len(stmts) == 0 {
		return false, nil
	}
	if a.terminates(stmts[len(stmts)-1]) {
		return true, nil
	}
	for j := len(stmts) - 1; j >= 0; j-- {
		st := stmts[j]
		if !a.mentionsKey(st, k) {
			continue
		}
		if ifs, ok := st.(*ast.IfStmt); ok && ifs.Else == nil && a.condImplies(ifs.Cond, k, false) {
			// `if k == nil { …establish… }`: non-nil afterwards either way
			if ok, ex := a.establishes(ifs.Body.List, k); ok {
				return true, ex
			}
			return false, nil
		}
		if !a.assignedBetween(st, k, st.Pos()-1, st.End()+1) {
			continue // only reads k
		}
		switch st := st.(type) {
		case *ast.AssignStmt:
			if st.Tok != token.ASSIGN || len(st.Lhs) != len(st.Rhs) {
				return false, nil
			}
			for li, l := range st.Lhs {
				if a.matchKey(l, k) {
					return true, []ast.Expr{st.Rhs[li]}
				}
			}
		case *ast.SwitchStmt, *ast.TypeSwitchStmt:
			var body *ast.BlockStmt
			if sw, ok := st.(*ast.SwitchStmt); ok {
				body = sw.Body
			} else {
				body = st.(*ast.TypeSwitchStmt).Body
			}
			hasDefault := false
			var extras []ast.Expr
			for _, cl := range body.List {
				cc := cl.(*ast.CaseClause)
				if cc.List == nil {
					hasDefault = true
				}
				ok, ex := a.establishes(cc.Body, k)
				if !ok {
					return false, nil
				}
				extras = append(extras, ex...)
			}
			return hasDefault, extras
		case *ast.IfStmt:
			if st.Else == nil {
				if a.condImplies(st.Cond, k, false) {
					// `if k == nil { …establish… }` after an earlier value: non-nil either way
					if ok, ex := a.establishes(st.Body.List, k); ok {
						return true, ex
					}
				}
				return false, nil
			}
			ok1, ex1 := a.establishes(st.Body.List, k)
			var ok2 bool
			var ex2 []ast.Expr
			switch e := st.Else.(type) {
			case *ast.BlockStmt:
				ok2, ex2 = a.establishes(e.List, k)
			case *ast.IfStmt:
				ok2, ex2 = a.establishes([]ast.Stmt{e}, k)
			}
			if ok1 && ok2 {
				return true, append(ex1, ex2...)
			}
		}
		return false, nil
	}
	return false, nil
}

// assignedValues returns the right-hand sides of all assignments to k inside n (not in closures).
func (a *Analyzer) assignedValues(n ast.Node, k guardKey) []ast.Expr {
	var out []ast.Expr
	ast.Inspect(n, func(x ast.Node) bool {
		switch x := x.(type) {
		case *ast.FuncLit:
			return false
		case *ast.AssignStmt:
			if x.Tok == token.ASSIGN && len(x.Lhs) == len(x.Rhs) {
				for i, l := range x.Lhs {
					if a.matchKey(l, k) {
						if ok, _ := a.guardAfterAssignKey(x, k); ok {
							continue // the following nil guard's own assignments are collected instead
						}
						out = append(out, x.Rhs[i])
					}
				}
			}
		}
		return true
	})
	return out
}

func (a *Analyzer) mentionsKey(n ast.Node, k guardKey) bool {
	found := false
	ast.Inspect(n, func(x ast.Node) bool {
		if found {
			return false
		}
		if e, ok := x.(ast.Expr); ok && a.matchKey(e, k) {
			found = true
		}
		return true
	})
	return found
}

// nilGuard reports whether statement s (appearing before the point of interest) makes k
// non-nil afterwards: `if k == nil { return … }`, `if k == nil { …; k = v }`,
// `debug.Assert(k != nil)`.
func (a *Analyzer) nilGuard(s ast.Stmt, k guardKey) (bool, []ast.Expr) {
	switch s := s.(type) {
	case *ast.IfStmt:
		if s.Else != nil {
			if a.condImplies(s.Cond, k, false) {
				// `if k == nil { …establish… } else { … }`: k is already non-nil in the else branch
				ok, ex := a.establishes(s.Body.List, k)
				if !ok {
					ex = a.assignedValues(s.Body, k)
					ok = len(ex) > 0
				}
				if ok {
					return true, append(ex, a.assignedValues(s.Else, k)...)
				}
				return false, nil
			}
			// if/else whose branches both establish k (possibly via nested nil guards)
			return a.establishes([]ast.Stmt{s}, k)
		}
		if !a.condImplies(s.Cond, k, false) {
			return false, nil
		}
		if ok, ex := a.establishes(s.Body.List, k); ok {
			return true, ex
		}
		// Lazy-initialization idiom `if k == nil { …; k = v; … }`: assume every path assigns.
		if vals := a.assignedValues(s.Body, k); len(vals) > 0 {
			return true, vals
		}
		return false, nil
	case *ast.SwitchStmt, *ast.TypeSwitchStmt:
		return a.establishes([]ast.Stmt{s}, k)
	case *ast.AssignStmt:
		// the nearest preceding plain assignment decides k's value
		return a.establishes([]ast.Stmt{s}, k)
	case *ast.ExprStmt:
		if call, isCall := s.X.(*ast.CallExpr); isCall {
			if f := a.calledFunc(call); f != nil && funcKey(f) == "debug.Assert" && len(call.Args) > 0 && a.condImplies(call.Args[0], k, true) {
				return true, nil
			}
		}
	}
	return false, nil
}

// guardInList looks for a statement before child in list that establishes k != nil.
func (a *Analyzer) guardInList(list []ast.Stmt, child ast.Node, k guardKey, at ast.Node) (bool, []ast.Expr) {
	idx := -1
	for i, s := range list {
		if ast.Node(s) == child {
			idx = i
			break
		}
	}
	for i := idx - 1; i >= 0; i-- {
		s := list[i]
		if ok, extras := a.nilGuard(s, k); ok {
			ok2, more := a.valuesBetween(&ast.BlockStmt{List: list}, k, s.End(), at.Pos())
			if !ok2 {
				return false, nil
			}
			return true, append(extras, more...)
		}
	}
	return false, nil
}

// valuesBetween: k stays known non-nil from `from` to `to` except for the values assigned
// to it in between (returned). Fails if k's address is taken or it is modified otherwise.
func (a *Analyzer) valuesBetween(n ast.Node, k guardKey, from, to token.Pos) (bool, []ast.Expr) {
	ok := true
	var vals []ast.Expr
	ast.Inspect(n, func(x ast.Node) bool {
		if !ok || x == nil || x.End() <= from || x.Pos() >= to {
			return false
		}
		switch s := x.(type) {
		case *ast.AssignStmt:
			for i, l := range s.Lhs {
				if l.Pos() <= from || l.Pos() >= to || !a.clobbers(l, k) {
					continue
				}
				if s.Tok == token.ASSIGN && len(s.Lhs) == len(s.Rhs) && a.matchKey(l, k) {
					vals = append(vals, s.Rhs[i])
				} else {
					ok = false
				}
			}
		case *ast.UnaryExpr:
			if s.Op == token.AND && a.clobbers(s.X, k) {
				ok = false
			}
		case *ast.IncDecStmt:
			if a.clobbers(s.X, k) {
				ok = false
			}
		}
		return true
	})
	return ok, vals
}

// guardAfterAssign: `x = f(); if x == nil { x = d }` — the first later statement in the
// same list that mentions x is a nil guard, so the assigned value never escapes as nil.
func (a *Analyzer) guardAfterAssign(stmt ast.Stmt, v *types.Var) (bool, []ast.Expr) {
	return a.guardAfterAssignKey(stmt, guardKey{v: v})
}

func (a *Analyzer) guardAfterAssignKey(stmt ast.Stmt, k guardKey) (bool, []ast.Expr) {
	var list []ast.Stmt
	switch p := a.parents[stmt].(type) {
	case *ast.BlockStmt:
		list = p.List
	case *ast.CaseClause:
		list = p.Body
	case *ast.CommClause:
		list = p.Body
	default:
		return false, nil
	}
	for i, s := range list {
		if s != stmt {
			continue
		}
		for _, next := range list[i+1:] {
			if !a.mentionsKey(next, k) {
				continue
			}
			return a.nilGuard(next, k)
		}
		break
	}
	return false, nil
}

func (a *Analyzer) call(x *ast.CallExpr) {
	slots, fn, off := a.callee(x)
	if slots == nil {
		return
	}
	if fn != nil && !a.curExternal {
		fn.Calls++
	}
	n := len(slots.Params)
	if len(x.Args) == 1 && n > 1 {
		return // f(g()) with a multi-value g
	}
	targetMain := (fn != nil && !fn.External) || (fn == nil && !a.curExternal)
	for i, arg := range x.Args {
		pi := i - off
		if pi < 0 || pi >= n {
			continue
		}
		if slots.Variadic && pi >= n-1 {
			continue
		}
		target := slots.Params[pi]
		arg = unparen(arg)
		if a.isNilish(arg) {
			a.mark(target, "nil passed at "+a.pos(arg.Pos()))
			target.NilFlow = target.Nilable
			continue
		}
		if targetMain && target.Nilable {
			if id, ok := arg.(*ast.Ident); ok {
				if v, ok := a.info.Uses[id].(*types.Var); ok && !v.IsField() {
					if guarded, _ := a.guard(guardKey{v: v}, x); !guarded {
						a.flows = append(a.flows, flowRec{target, source{kind: srcVar, v: v, noInit: a.definitelyAssigned(v, id)}, "nil-able " + v.Name() + " passed at " + a.pos(id.Pos())})
					}
				}
			}
		}
		switch e := arg.(type) {
		case *ast.Ident:
			switch o := a.info.Uses[e].(type) {
			case *types.Var:
				if lit := a.localLit[o]; lit != nil && target.Sub != nil {
					if !a.handled[lit] {
						a.walkCallbackLit(lit, target.Sub)
					} else if own := a.litSlots[lit]; own != nil {
						a.callbackForwardEdges(own, target.Sub, a.pos(e.Pos()))
					}
				}
				if vi := a.vars[o]; vi != nil && vi.slot != nil {
					if targetMain {
						vi.forwards = append(vi.forwards, forwardRec{e.Pos(), target, "forwarded at " + a.pos(e.Pos())})
					}
					if vi.slot.Sub != nil && target.Sub != nil {
						a.callbackForwardEdges(vi.slot.Sub, target.Sub, a.pos(e.Pos()))
					}
				}
			case *types.Func:
				if target.Sub != nil {
					a.funcValueEdges(o, target.Sub, a.pos(e.Pos()))
				}
			}
		case *ast.SelectorExpr:
			if target.Sub == nil {
				break
			}
			if f := a.funcValue(e); f != nil {
				a.funcValueEdges(f, target.Sub, a.pos(e.Pos()))
			}
		case *ast.FuncLit:
			if target.Sub != nil {
				a.walkCallbackLit(e, target.Sub)
			}
		}
	}
}

// walkCallbackLit analyzes a function literal passed where callback slots `shared` are
// expected. The literal gets its own slots (so call sites can be judged individually),
// linked to the shared ones.
func (a *Analyzer) walkCallbackLit(lit *ast.FuncLit, shared *SigSlots) {
	a.handled[lit] = true
	a.walkFunc(lit.Type, lit.Body, a.ownSlots(lit, shared))
}

// ownSlots returns (creating on first use) the literal's own slots linked to `shared`.
func (a *Analyzer) ownSlots(lit *ast.FuncLit, shared *SigSlots) *SigSlots {
	if own := a.litSlots[lit]; own != nil {
		return own
	}
	own := &SigSlots{Variadic: shared.Variadic}
	for _, p := range shared.Params {
		own.Params = append(own.Params, &Slot{Nilable: p.Nilable, Sub: p.Sub, owner: p.owner})
	}
	for _, r := range shared.Results {
		sl := &Slot{Nilable: r.Nilable, Sub: r.Sub, isResult: true, owner: r.owner}
		own.Results = append(own.Results, sl)
		a.allResults = append(a.allResults, sl)
	}
	for k := range own.Params {
		a.edges = append(a.edges, edge{shared.Params[k], own.Params[k], "callback parameter"})
		a.edges = append(a.edges, edge{own.Params[k], shared.Params[k], "callback literal compares its parameter"})
	}
	for j := range own.Results {
		a.edges = append(a.edges, edge{own.Results[j], shared.Results[j], "callback literal at " + a.pos(lit.Pos()) + " can return nil"})
	}
	a.litSlots[lit] = own
	return own
}

// litVar returns the local variable a function literal is directly assigned to.
func (a *Analyzer) litVar(lit *ast.FuncLit) *types.Var {
	var lhs []ast.Expr
	var rhs []ast.Expr
	switch p := a.parents[lit].(type) {
	case *ast.AssignStmt:
		lhs, rhs = p.Lhs, p.Rhs
	case *ast.ValueSpec:
		for _, n := range p.Names {
			lhs = append(lhs, n)
		}
		rhs = p.Values
	default:
		return nil
	}
	if len(lhs) != len(rhs) {
		return nil
	}
	for i, r := range rhs {
		if r != ast.Expr(lit) {
			continue
		}
		id, ok := lhs[i].(*ast.Ident)
		if !ok {
			return nil
		}
		obj := a.info.Defs[id]
		if obj == nil {
			obj = a.info.Uses[id]
		}
		if v, ok := obj.(*types.Var); ok && !v.IsField() && v.Parent() != v.Pkg().Scope() {
			return v
		}
	}
	return nil
}

// callbackResult: for a call of a callback-conditional function (nil only when its
// callback returns nil), the source describing this call site's callback result.
func (a *Analyzer) callbackResult(call *ast.CallExpr, fn *Func) (source, bool) {
	if fn == nil || !a.cbCond[fn.Name] {
		return source{}, false
	}
	for i, p := range fn.Slots.Params {
		if p.Sub == nil || len(p.Sub.Results) == 0 || !p.Sub.Results[0].Nilable || i >= len(call.Args) {
			continue
		}
		arg := unparen(call.Args[i])
		if lit, ok := arg.(*ast.FuncLit); ok {
			own := a.ownSlots(lit, p.Sub)
			return source{kind: srcSlot, slot: own.Results[0]}, true
		}
		if f := a.funcValue(arg); f != nil {
			if cf := a.funcs[f.Origin()]; cf != nil && len(cf.Slots.Results) > 0 {
				return source{kind: srcSlot, slot: cf.Slots.Results[0]}, true
			}
		}
		if id, ok := arg.(*ast.Ident); ok {
			if v, ok := a.info.Uses[id].(*types.Var); ok {
				if ps := a.paramSlot[v]; ps != nil && ps.Sub != nil && len(ps.Sub.Results) > 0 {
					return source{kind: srcSlot, slot: ps.Sub.Results[0]}, true
				}
				if lit := a.localLit[v]; lit != nil {
					return source{kind: srcSlot, slot: a.ownSlots(lit, p.Sub).Results[0]}, true
				}
			}
		}
		return source{}, false
	}
	return source{}, false
}

// callbackForwardEdges: a callback parameter `own` is passed on as callback `target`.
func (a *Analyzer) callbackForwardEdges(own, target *SigSlots, at string) {
	for k := 0; k < len(own.Params) && k < len(target.Params); k++ {
		a.edges = append(a.edges, edge{target.Params[k], own.Params[k], "callback forwarded at " + at})
	}
	for j := 0; j < len(own.Results) && j < len(target.Results); j++ {
		a.edges = append(a.edges, edge{own.Results[j], target.Results[j], "callback forwarded at " + at})
	}
}

// funcValueEdges: named function f is used as a value where callback `target` is expected.
func (a *Analyzer) funcValueEdges(f *types.Func, target *SigSlots, at string) {
	fn := a.funcs[f.Origin()]
	if fn == nil {
		return
	}
	off := len(target.Params) - len(fn.Slots.Params)
	if off < 0 {
		off = 0
	}
	for j := 0; j < len(fn.Slots.Results) && j < len(target.Results); j++ {
		a.edges = append(a.edges, edge{fn.Slots.Results[j], target.Results[j], "function value " + f.Name() + " at " + at})
	}
	if !fn.External {
		for k := 0; k < len(fn.Slots.Params) && k+off < len(target.Params); k++ {
			a.edges = append(a.edges, edge{target.Params[k+off], fn.Slots.Params[k], "used as callback at " + at})
		}
	}
}

func (a *Analyzer) funcValue(e ast.Expr) *types.Func {
	switch x := unparen(e).(type) {
	case *ast.Ident:
		f, _ := a.info.Uses[x].(*types.Func)
		return f
	case *ast.SelectorExpr:
		if sel := a.info.Selections[x]; sel != nil {
			if sel.Kind() == types.MethodVal || sel.Kind() == types.MethodExpr {
				f, _ := sel.Obj().(*types.Func)
				return f
			}
			return nil
		}
		f, _ := a.info.Uses[x.Sel].(*types.Func)
		return f
	}
	return nil
}

func (a *Analyzer) ret(x *ast.ReturnStmt, ctx *fnCtx) {
	if ctx.slots == nil || a.skipReturn[x] {
		return
	}
	rs := ctx.slots.Results
	why := "returned at " + a.pos(x.Pos())
	if len(x.Results) == 0 {
		for j, v := range ctx.named {
			if v != nil && j < len(rs) && rs[j].Nilable {
				a.results = append(a.results, resultSrc{rs[j], source{kind: srcVar, v: v}, "named result at bare return " + a.pos(x.Pos())})
			}
		}
		return
	}
	if len(x.Results) == 1 && len(rs) > 1 {
		if call, ok := unparen(x.Results[0]).(*ast.CallExpr); ok {
			slots, _, _ := a.callee(call)
			for j := 0; j < len(rs); j++ {
				src := source{kind: srcUnknown}
				if slots != nil && j < len(slots.Results) {
					src = source{kind: srcSlot, slot: slots.Results[j]}
				}
				a.results = append(a.results, resultSrc{rs[j], src, why})
			}
		}
		return
	}
	for j, e := range x.Results {
		if j >= len(rs) {
			break
		}
		if rs[j].Sub != nil {
			re := unparen(e)
			// `return core.Memoize(func() *T {…})`: the wrapped literal is what callers get
			if call, ok := re.(*ast.CallExpr); ok && len(call.Args) == 1 {
				if f := a.calledFunc(call); f != nil && a.funcPass[funcKey(f)] {
					re = unparen(call.Args[0])
				}
			}
			if lit, ok := re.(*ast.FuncLit); ok {
				a.handled[lit] = true
				a.walkFunc(lit.Type, lit.Body, rs[j].Sub)
			} else if f := a.funcValue(re); f != nil {
				a.funcValueEdges(f, rs[j].Sub, a.pos(e.Pos()))
			}
		}
		if !rs[j].Nilable {
			continue
		}
		es := types.ExprString(e)
		if len(es) > 60 {
			es = es[:60] + "…"
		}
		a.pendingResults = append(a.pendingResults, pendingResult{rs[j], e, why + " (" + es + ")"})
	}
}

func (a *Analyzer) rhsSources(lhsCount int, rhs []ast.Expr) ([]source, []*Slot) {
	srcs := make([]source, lhsCount)
	calls := make([]*Slot, lhsCount)
	if len(rhs) == lhsCount {
		for i, r := range rhs {
			srcs[i] = a.exprSourceAt(r, r)
			if call, ok := unparen(r).(*ast.CallExpr); ok {
				if slots, _, _ := a.callee(call); slots != nil && len(slots.Results) >= 1 {
					calls[i] = slots.Results[0]
				}
			}
		}
	} else if len(rhs) == 1 {
		for i := range srcs {
			srcs[i] = source{kind: srcNonNil} // comma-ok forms: the value is checked via ok
		}
		if call, ok := unparen(rhs[0]).(*ast.CallExpr); ok {
			slots, _, _ := a.callee(call)
			for i := 0; i < lhsCount; i++ {
				if slots != nil && i < len(slots.Results) {
					srcs[i] = source{kind: srcSlot, slot: slots.Results[i]}
					calls[i] = slots.Results[i]
				} else {
					srcs[i] = source{kind: srcUnknown}
				}
			}
		}
	}
	return srcs, calls
}

func (a *Analyzer) assign(x *ast.AssignStmt) {
	plain := x.Tok == token.ASSIGN || x.Tok == token.DEFINE
	var srcs []source
	var calls []*Slot
	if plain {
		srcs, calls = a.rhsSources(len(x.Lhs), x.Rhs)
	}
	for i, lhs := range x.Lhs {
		switch l := unparen(lhs).(type) {
		case *ast.Ident:
			if l.Name == "_" {
				continue
			}
			obj := a.info.Defs[l]
			if obj == nil {
				obj = a.info.Uses[l]
			}
			v, _ := obj.(*types.Var)
			if v == nil {
				continue
			}
			vi := a.varInfo(v)
			vi.assignPos = append(vi.assignPos, l.Pos())
			if plain && len(x.Lhs) == len(x.Rhs) {
				vi.defExprs = append(vi.defExprs, x.Rhs[i])
			} else {
				vi.defExprs = append(vi.defExprs, nil)
			}
			if plain {
				src := srcs[i]
				if ok, extras := a.guardAfterAssign(x, v); ok {
					src = a.sourcesOf(extras)
				}
				vi.assigns = append(vi.assigns, assignRec{src, calls[i]})
			}
		case *ast.SelectorExpr:
			sel := a.info.Selections[l]
			if sel == nil || sel.Kind() != types.FieldVal {
				continue
			}
			fv := sel.Obj().(*types.Var).Origin()
			if plain {
				fi := a.fieldInfo(fv)
				src := srcs[i]
				if k, ok := a.keyOf(l); ok {
					if ok, extras := a.guardAfterAssignKey(x, k); ok {
						src = a.sourcesOf(extras)
					}
				}
				fi.srcs = append(fi.srcs, src)
				if fn := a.funcs[fv]; fn != nil && fn.IsField && len(x.Lhs) == len(x.Rhs) {
					if f := a.funcValue(x.Rhs[i]); f != nil {
						a.funcValueEdges(f, fn.Slots, a.pos(l.Pos()))
					}
				}
			}
		}
	}
}

// compositeLit links func values stored into func-typed struct fields (`&M{fn: f}`), and
// records nil stored into pointer fields.
func (a *Analyzer) compositeLit(x *ast.CompositeLit) {
	for _, el := range x.Elts {
		kv, ok := el.(*ast.KeyValueExpr)
		if !ok {
			continue
		}
		key, ok := kv.Key.(*ast.Ident)
		if !ok {
			continue
		}
		fv, ok := a.info.Uses[key].(*types.Var)
		if !ok || !fv.IsField() {
			continue
		}
		fv = fv.Origin()
		fi := a.fieldInfo(fv)
		fi.srcs = append(fi.srcs, a.exprSourceAt(kv.Value, kv.Value))
		fn := a.funcs[fv]
		if fn == nil || !fn.IsField {
			continue
		}
		if lit, ok := unparen(kv.Value).(*ast.FuncLit); ok {
			a.walkCallbackLit(lit, fn.Slots)
		} else if f := a.funcValue(kv.Value); f != nil {
			a.funcValueEdges(f, fn.Slots, a.pos(kv.Pos()))
		} else if id, ok := unparen(kv.Value).(*ast.Ident); ok {
			if v, ok := a.info.Uses[id].(*types.Var); ok {
				if ps := a.paramSlot[v]; ps != nil && ps.Sub != nil {
					a.callbackForwardEdges(ps.Sub, fn.Slots, a.pos(kv.Pos()))
				}
			}
		}
	}
}

func (a *Analyzer) valueSpec(vs *ast.ValueSpec) {
	var srcs []source
	var calls []*Slot
	if len(vs.Values) > 0 {
		srcs, calls = a.rhsSources(len(vs.Names), vs.Values)
	}
	for i, nm := range vs.Names {
		v, _ := a.info.Defs[nm].(*types.Var)
		if v == nil || nm.Name == "_" {
			continue
		}
		vi := a.varInfo(v)
		if len(vs.Values) == 0 {
			if a.nilableF(v.Type()) {
				vi.init = &source{kind: srcNil}
				vi.zeroBody = a.enclosingBody(vs)
			}
			continue
		}
		if len(vs.Values) == len(vs.Names) {
			vi.defExprs = append(vi.defExprs, vs.Values[i])
		}
		src := srcs[i]
		if ds, ok := a.parents[a.parents[vs]].(*ast.DeclStmt); ok {
			if ok, extras := a.guardAfterAssign(ds, v); ok {
				src = a.sourcesOf(extras)
			}
		}
		vi.assigns = append(vi.assigns, assignRec{src, calls[i]})
	}
}

func (a *Analyzer) sourcesOf(extras []ast.Expr) source {
	if len(extras) == 0 {
		return source{kind: srcNonNil}
	}
	var list []source
	for _, x := range extras {
		list = append(list, a.exprSourceAt(x, x))
	}
	return source{kind: srcAny, list: list}
}

func anyBefore(list []token.Pos, p token.Pos) bool {
	for _, x := range list {
		if x < p {
			return true
		}
	}
	return false
}

// resolvePending turns deferred return expressions into sources. It must run while
// a.info belongs to the package the expressions come from.
func (a *Analyzer) resolvePending() {
	for _, lit := range a.pendingLits {
		if !a.handled[lit] {
			a.handled[lit] = true
			a.walkFunc(lit.Type, lit.Body, nil)
		}
	}
	a.pendingLits = nil
	for _, p := range a.pendingResults {
		a.results = append(a.results, resultSrc{p.slot, a.exprSourceAt(p.e, p.e), p.why})
	}
	a.pendingResults = nil
}

// applyOverrides applies config overrides before the fixpoint:
//
//	"Recv.name.p0" / "name.r0" -> "P" | "Option";  "Recv.name#local" -> "nonnil".
func (a *Analyzer) applyOverrides() []string {
	var unknown []string
	for key, val := range a.cfg.Overrides {
		ok := false
		for _, fn := range a.order {
			fk := fn.Name
			if fn.RecvType != nil {
				fk = fn.RecvType.Name() + "." + fn.Name
			}
			if i := strings.Index(key, "#"); i >= 0 {
				if key[:i] != fk || fn.Decl == nil {
					continue
				}
				ast.Inspect(fn.Decl, func(n ast.Node) bool {
					if id, isID := n.(*ast.Ident); isID && id.Name == key[i+1:] {
						if v, isVar := fn.Pkg.Info.Defs[id].(*types.Var); isVar {
							a.varInfo(v).initKilled = true
							a.varInfo(v).forcedNonNil = true
							ok = true
						}
					}
					return true
				})
				continue
			}
			var slot *Slot
			var idx int
			if strings.HasPrefix(key, fk+".p") {
				if _, err := fmt.Sscanf(key[len(fk)+2:], "%d", &idx); err == nil && idx < len(fn.Slots.Params) {
					slot = fn.Slots.Params[idx]
				}
			} else if strings.HasPrefix(key, fk+".r") {
				if _, err := fmt.Sscanf(key[len(fk)+2:], "%d", &idx); err == nil && idx < len(fn.Slots.Results) {
					slot = fn.Slots.Results[idx]
				}
			}
			if slot == nil {
				continue
			}
			ok = true
			if val == "P" {
				slot.forced = 1
			} else {
				slot.forced = 2
				slot.Option = slot.Nilable
				slot.Why = "config override"
				if slot.isResult {
					slot.State = nsNil
				} else {
					slot.NilFlow = true
				}
			}
		}
		if !ok {
			unknown = append(unknown, key)
		}
	}
	sort.Strings(unknown)
	return unknown
}

// markNilTolerant makes read-only parameters of nil-tolerant types (a nil Go map reads as
// empty; ast.SymbolTable) Option.
func (a *Analyzer) markNilTolerant() {
	if len(a.cfg.NilTolerantParams) == 0 {
		return
	}
	for _, fn := range a.order {
		if fn.External || fn.IsField {
			continue
		}
		for i := 0; i < fn.Sig.Params().Len() && i < len(fn.Slots.Params); i++ {
			v := fn.Sig.Params().At(i)
			if !contains(a.cfg.NilTolerantParams, types.TypeString(v.Type(), func(p *types.Package) string { return p.Name() })) {
				continue
			}
			if a.readTolerant != nil && !a.readTolerant(v) {
				continue
			}
			a.mark(fn.Slots.Params[i], "read-only "+types.TypeString(v.Type(), func(p *types.Package) string { return p.Name() })+" parameter (a nil table reads as empty in Go)")
			fn.Slots.Params[i].NilFlow = fn.Slots.Params[i].Option
		}
	}
}

// finish applies the per-variable rules and runs the fixpoint.
func (a *Analyzer) finish() {
	a.markNilTolerant()
	mode := a.cfg.ParamForwarding
	vars := make([]*types.Var, 0, len(a.vars))
	for v := range a.vars {
		vars = append(vars, v)
	}
	sort.Slice(vars, func(i, j int) bool { return vars[i].Pos() < vars[j].Pos() })
	for _, v := range vars {
		vi := a.vars[v]
		if vi.slot != nil {
			// A comparison or forward only speaks about the parameter's incoming value if no
			// assignment to it textually precedes it.
			for _, cp := range vi.comparePos {
				if !anyBefore(vi.assignPos, cp) {
					a.mark(vi.slot, "compared to nil at "+a.pos(cp))
					break
				}
			}
			if mode == "reverse" || mode == "both" {
				for _, fw := range vi.forwards {
					if !anyBefore(vi.assignPos, fw.pos) {
						a.edges = append(a.edges, edge{fw.target, vi.slot, fw.why})
					}
				}
			}
		} else if len(vi.comparePos) > 0 && len(vi.assigns) == 1 && vi.assigns[0].callSlot != nil && (vi.init == nil || vi.initKilled) {
			a.evidence(vi.assigns[0].callSlot, fmt.Sprintf("result stored in %s, compared to nil at %s", v.Name(), a.pos(vi.comparePos[0])))
		}
	}
	forward := mode == "" || mode == "forward" || mode == "both"
	for changed := true; changed; {
		changed = false
		for _, e := range a.edges {
			if e.from.Option && e.to.Nilable && !e.to.Option && e.to.forced != 1 {
				a.mark(e.to, e.why)
				changed = true
			}
		}
		for i := range a.results {
			r := &a.results[i]
			if s := a.eval(r.src, map[*types.Var]bool{}); s > r.slot.State && r.slot.forced != 1 {
				r.slot.State = s
				if s == nsNil {
					r.slot.Why = r.why
					r.slot.cause = r
				} else if r.slot.ucause == nil {
					r.slot.ucause = r
				}
				changed = true
			}
		}
		for _, s := range a.allResults {
			if s.Nilable && !s.Option && s.forced != 1 && (s.State == nsNil || (s.State == nsUnknown && s.Evidence)) {
				s.Option = true
				if s.State != nsNil {
					s.Why = s.EvWhy
					s.viaEvidence = true
				}
				changed = true
			}
		}
		for _, fi := range a.fields {
			if fi.isNil {
				continue
			}
			for _, s := range fi.srcs {
				if a.eval(s, map[*types.Var]bool{}) == nsNil {
					fi.isNil = true
					fi.why = "field assigned a nil-able value"
					changed = true
					break
				}
			}
		}
		if forward {
			for i := range a.flows {
				f := &a.flows[i]
				if !f.target.NilFlow && f.target.Nilable && f.target.forced != 1 && !f.target.Transparent && a.carriesNil(f.src, map[*types.Var]bool{}) {
					a.mark(f.target, f.why)
					f.target.NilFlow = true
					f.target.flowFrom = f
					changed = true
				}
			}
		}
	}
}

func maxState(a, b nstate) nstate {
	if a > b {
		return a
	}
	return b
}

func (a *Analyzer) eval(s source, visiting map[*types.Var]bool) nstate {
	switch s.kind {
	case srcNil:
		return nsNil
	case srcUnknown:
		return nsUnknown
	case srcSlot:
		if s.slot.Option {
			return nsNil
		}
		if s.slot.isResult {
			return s.slot.State
		}
		return nsNonNil
	case srcField:
		if fi := a.fields[s.v]; fi != nil && fi.isNil {
			return nsNil
		}
		if s.v.Pkg() != nil && !a.analyzed[s.v.Pkg()] {
			return nsUnknown
		}
		return nsNonNil
	case srcAny:
		st := nsNonNil
		for _, x := range s.list {
			st = maxState(st, a.eval(x, visiting))
		}
		return st
	case srcAll:
		st := nsNil
		for _, x := range s.list {
			if y := a.eval(x, visiting); y < st {
				st = y
			}
		}
		return st
	case srcVar:
		if visiting[s.v] {
			return nsNonNil
		}
		visiting[s.v] = true
		defer delete(visiting, s.v)
		vi := a.vars[s.v]
		if vi == nil || vi.forcedNonNil {
			return nsNonNil
		}
		st := nsNonNil
		if vi.init != nil && !vi.initKilled && !s.noInit {
			st = a.eval(*vi.init, visiting)
		}
		for _, as := range vi.assigns {
			st = maxState(st, a.eval(as.src, visiting))
		}
		if vi.init == nil && len(vi.assigns) == 0 {
			return nsNonNil // range variables over slices, type-switch bindings
		}
		return st
	}
	return nsNonNil
}

// carriesNil: nil provenance through variables only — a literal nil / zero value or a
// nil-able parameter can flow into the value. Call results and fields do not count.
func (a *Analyzer) carriesNil(s source, visiting map[*types.Var]bool) bool {
	switch s.kind {
	case srcNil:
		return !s.soft
	case srcSlot:
		return !s.slot.isResult && s.slot.NilFlow
	case srcAny:
		for _, x := range s.list {
			if a.carriesNil(x, visiting) {
				return true
			}
		}
	case srcAll:
		for _, x := range s.list {
			if !a.carriesNil(x, visiting) {
				return false
			}
		}
		return len(s.list) > 0
	case srcVar:
		if visiting[s.v] {
			return false
		}
		visiting[s.v] = true
		defer delete(visiting, s.v)
		vi := a.vars[s.v]
		if vi == nil || vi.forcedNonNil {
			return false
		}
		if vi.init != nil && !vi.initKilled && !s.noInit && a.carriesNil(*vi.init, visiting) {
			return true
		}
		for _, as := range vi.assigns {
			if a.carriesNil(as.src, visiting) {
				return true
			}
		}
	}
	return false
}

func describeSlots(prefix string, s *SigSlots, out *[]string) {
	for i, p := range s.Params {
		if p.Option {
			*out = append(*out, fmt.Sprintf("%s.p%d: %s", prefix, i, p.Why))
		}
		if p.Sub != nil {
			describeSlots(fmt.Sprintf("%s.p%d", prefix, i), p.Sub, out)
		}
	}
	for i, r := range s.Results {
		if r.Option {
			*out = append(*out, fmt.Sprintf("%s.r%d: %s", prefix, i, r.Why))
		} else if r.Nilable && r.State == nsUnknown {
			*out = append(*out, fmt.Sprintf("%s.r%d: (unknown nil-ness, no call site checks -> not Option)", prefix, i))
		}
		if r.Sub != nil {
			describeSlots(fmt.Sprintf("%s.r%d", prefix, i), r.Sub, out)
		}
	}
}

func (f *Func) whyLines() string {
	var out []string
	describeSlots(f.RustName, f.Slots, &out)
	return strings.Join(out, "\n")
}

// trace explains why a result slot is Option by following nil sources to their roots.
func (a *Analyzer) trace(s *Slot, depth int, seen map[*Slot]bool, out *[]string) {
	ind := strings.Repeat("  ", depth)
	name := "?"
	if s.owner != nil {
		name = s.owner.Name
	}
	if seen[s] || depth > 25 {
		*out = append(*out, ind+name+": (cycle)")
		return
	}
	seen[s] = true
	if s.viaEvidence {
		*out = append(*out, ind+name+": Option by call-site evidence: "+s.EvWhy)
		if s.ucause != nil {
			*out = append(*out, ind+"  unknown because "+s.ucause.why)
		}
		return
	}
	if s.cause == nil {
		*out = append(*out, ind+name+": "+s.Why)
		return
	}
	*out = append(*out, ind+name+": "+s.cause.why)
	a.traceSrc(s.cause.src, depth+1, seen, map[*types.Var]bool{}, out)
}

func (a *Analyzer) traceSrc(src source, depth int, seen map[*Slot]bool, vis map[*types.Var]bool, out *[]string) {
	ind := strings.Repeat("  ", depth)
	switch src.kind {
	case srcNil:
		*out = append(*out, ind+"nil")
	case srcSlot:
		if src.slot.isResult && src.slot.cause == nil && src.slot.Option {
			*out = append(*out, ind+ownerName(src.slot)+": "+src.slot.Why)
		} else if src.slot.isResult {
			a.trace(src.slot, depth, seen, out)
		} else {
			*out = append(*out, ind+"param: "+src.slot.Why)
		}
	case srcField:
		why := ""
		if fi := a.fields[src.v]; fi != nil {
			why = fi.why
		}
		*out = append(*out, ind+"field "+src.v.Name()+": "+why)
	case srcAny, srcAll:
		for _, x := range src.list {
			if a.eval(x, map[*types.Var]bool{}) != nsNil {
				continue
			}
			if x.kind == srcSlot && seen[x.slot] {
				name := "?"
				if x.slot.owner != nil {
					name = x.slot.owner.Name
				}
				*out = append(*out, ind+"(back to "+name+")")
				continue
			}
			a.traceSrc(x, depth, seen, vis, out)
		}
	case srcVar:
		if vis[src.v] {
			return
		}
		vis[src.v] = true
		vi := a.vars[src.v]
		if vi == nil {
			return
		}
		if vi.init != nil && !vi.initKilled && a.eval(*vi.init, map[*types.Var]bool{}) == nsNil {
			*out = append(*out, ind+"var "+src.v.Name()+" initial value ("+a.pos(src.v.Pos())+")")
			a.traceSrc(*vi.init, depth+1, seen, vis, out)
			return
		}
		for _, as := range vi.assigns {
			if a.eval(as.src, map[*types.Var]bool{}) == nsNil {
				*out = append(*out, ind+"var "+src.v.Name()+" assigned ("+a.pos(src.v.Pos())+")")
				a.traceSrc(as.src, depth+1, seen, vis, out)
				return
			}
		}
	}
}

// root returns a description of the root cause of a result slot's Option-ness.
func (a *Analyzer) root(s *Slot, seen map[*Slot]bool) string {
	name := "?"
	if s.owner != nil {
		name = s.owner.Name
	}
	if seen[s] {
		return "cycle@" + name
	}
	seen[s] = true
	if s.viaEvidence {
		return "evidence<" + a.unknownRoot(s, map[*Slot]bool{}, 0) + ">"
	}
	if s.cause == nil {
		return "direct@" + name + ": " + s.Why
	}
	return a.rootSrc(s.cause.src, name, seen, map[*types.Var]bool{})
}

func (a *Analyzer) rootSrc(src source, name string, seen map[*Slot]bool, vis map[*types.Var]bool) string {
	switch src.kind {
	case srcNil:
		return "nil@" + name
	case srcSlot:
		if src.slot.isResult {
			return a.root(src.slot, seen)
		}
		return "param@" + name
	case srcField:
		return "field " + src.v.Name()
	case srcAny, srcAll:
		for _, x := range src.list {
			if a.eval(x, map[*types.Var]bool{}) == nsNil && !(x.kind == srcSlot && seen[x.slot]) {
				return a.rootSrc(x, name, seen, vis)
			}
		}
		return "cycle@" + name
	case srcVar:
		if vis[src.v] {
			return "varcycle@" + name
		}
		vis[src.v] = true
		vi := a.vars[src.v]
		if vi.init != nil && !vi.initKilled && a.eval(*vi.init, map[*types.Var]bool{}) == nsNil {
			if vi.init.kind == srcNil {
				return "zero-init " + src.v.Name() + "@" + name
			}
			return a.rootSrc(*vi.init, name, seen, vis)
		}
		for _, as := range vi.assigns {
			if a.eval(as.src, map[*types.Var]bool{}) == nsNil {
				return a.rootSrc(as.src, name, seen, vis)
			}
		}
	}
	return "?@" + name
}

func (a *Analyzer) enclosingBody(n ast.Node) *ast.BlockStmt {
	for p := a.parents[n]; p != nil; p = a.parents[p] {
		switch f := p.(type) {
		case *ast.FuncDecl:
			return f.Body
		case *ast.FuncLit:
			return f.Body
		}
	}
	return nil
}

// definitelyAssigned reports whether zero-initialized variable v has been assigned on
// every path reaching the statement that contains use.
func (a *Analyzer) definitelyAssigned(v *types.Var, use ast.Node) bool {
	vi := a.vars[v]
	if vi == nil || vi.zeroBody == nil {
		return false
	}
	// The innermost function body containing the use: the declaring body, or a closure
	// (analyzed from "unassigned" at its entry).
	body := vi.zeroBody
	for n := ast.Node(use); n != nil; n = a.parents[n] {
		if n == ast.Node(vi.zeroBody) {
			break
		}
		if lit, ok := n.(*ast.FuncLit); ok {
			body = lit.Body
			break
		}
	}
	if vi.da == nil {
		vi.da = map[ast.Stmt]bool{}
	}
	if _, done := vi.da[body]; !done {
		w := &daWalker{a: a, v: v, at: vi.da}
		w.stmt(body, false, "")
	}
	for n := ast.Node(use); n != nil; n = a.parents[n] {
		if st, ok := n.(ast.Stmt); ok {
			if in, ok := vi.da[st]; ok {
				return in
			}
		}
		if n == ast.Node(body) {
			break
		}
	}
	return false
}

// daWalker computes definite assignment of one variable over structured Go code.
// "true" after a statement that cannot complete normally (return, panic, break, …).
type daWalker struct {
	a      *Analyzer
	v      *types.Var
	at     map[ast.Stmt]bool
	breaks []*daTarget
}

type daTarget struct {
	label string
	node  ast.Stmt
	outs  []bool
}

func (w *daWalker) stmts(list []ast.Stmt, in bool) bool {
	for _, s := range list {
		in = w.stmt(s, in, "")
	}
	return in
}

func (w *daWalker) assigns(lhs []ast.Expr) bool {
	for _, l := range lhs {
		if id, ok := unparen(l).(*ast.Ident); ok {
			if w.a.info.Uses[id] == w.v || w.a.info.Defs[id] == w.v {
				return true
			}
		}
	}
	return false
}

func (w *daWalker) push(label string, node ast.Stmt) *daTarget {
	t := &daTarget{label: label, node: node}
	w.breaks = append(w.breaks, t)
	return t
}

func (w *daWalker) pop() { w.breaks = w.breaks[:len(w.breaks)-1] }

func and(in bool, outs []bool) bool {
	for _, o := range outs {
		in = in && o
	}
	return in
}

func (w *daWalker) stmt(s ast.Stmt, in bool, label string) bool {
	if s == nil {
		return in
	}
	w.at[s] = in
	switch s := s.(type) {
	case *ast.AssignStmt:
		if (s.Tok == token.ASSIGN || s.Tok == token.DEFINE) && w.assigns(s.Lhs) {
			return true
		}
	case *ast.DeclStmt:
		if gd, ok := s.Decl.(*ast.GenDecl); ok {
			for _, sp := range gd.Specs {
				if vs, ok := sp.(*ast.ValueSpec); ok {
					for _, n := range vs.Names {
						if w.a.info.Defs[n] == w.v {
							return len(vs.Values) > 0
						}
					}
				}
			}
		}
	case *ast.ReturnStmt:
		return true
	case *ast.ExprStmt:
		if call, ok := s.X.(*ast.CallExpr); ok {
			if id, ok := unparen(call.Fun).(*ast.Ident); ok && id.Name == "panic" {
				return true
			}
		}
	case *ast.BranchStmt:
		switch s.Tok {
		case token.BREAK:
			for i := len(w.breaks) - 1; i >= 0; i-- {
				t := w.breaks[i]
				if s.Label == nil || t.label == s.Label.Name {
					t.outs = append(t.outs, in)
					break
				}
			}
		case token.GOTO:
			return in
		}
		return true
	case *ast.BlockStmt:
		return w.stmts(s.List, in)
	case *ast.LabeledStmt:
		return w.stmt(s.Stmt, in, s.Label.Name)
	case *ast.IfStmt:
		in = w.stmt(s.Init, in, "")
		b := w.stmts(s.Body.List, in)
		e := in
		if s.Else != nil {
			e = w.stmt(s.Else, in, "")
		}
		return b && e
	case *ast.ForStmt:
		in = w.stmt(s.Init, in, "")
		t := w.push(label, s)
		w.stmts(s.Body.List, in)
		w.pop()
		if s.Cond == nil {
			return and(true, t.outs)
		}
		return and(in, t.outs)
	case *ast.RangeStmt:
		t := w.push(label, s)
		w.stmts(s.Body.List, in)
		w.pop()
		return and(in, t.outs)
	case *ast.SwitchStmt, *ast.TypeSwitchStmt, *ast.SelectStmt:
		var body *ast.BlockStmt
		switch x := s.(type) {
		case *ast.SwitchStmt:
			in = w.stmt(x.Init, in, "")
			body = x.Body
		case *ast.TypeSwitchStmt:
			in = w.stmt(x.Init, in, "")
			body = x.Body
		case *ast.SelectStmt:
			body = x.Body
		}
		t := w.push(label, s)
		out := true
		hasDefault := false
		for _, cl := range body.List {
			switch c := cl.(type) {
			case *ast.CaseClause:
				if c.List == nil {
					hasDefault = true
				}
				out = w.stmts(c.Body, in) && out
			case *ast.CommClause:
				if c.Comm == nil {
					hasDefault = true
				}
				out = w.stmts(c.Body, in) && out
			}
		}
		w.pop()
		if _, isSelect := s.(*ast.SelectStmt); !hasDefault && !isSelect {
			out = out && in
		}
		return and(out, t.outs)
	}
	return in
}

// unknownRoot follows the chain of Unknown states to the first unanalyzable source.
func (a *Analyzer) unknownRoot(s *Slot, seen map[*Slot]bool, depth int) string {
	if seen[s] || depth > 40 {
		return "cycle@" + ownerName(s)
	}
	seen[s] = true
	if s.ucause == nil {
		if s.owner != nil && s.owner.IsField {
			return "bodiless field func " + s.owner.Name
		}
		return "no-cause@" + ownerName(s)
	}
	return a.unknownSrc(s.ucause.src, ownerName(s)+" "+s.ucause.why, seen, map[*types.Var]bool{}, depth)
}

func (a *Analyzer) unknownSrc(src source, ctx string, seen map[*Slot]bool, vis map[*types.Var]bool, depth int) string {
	switch src.kind {
	case srcUnknown:
		return "unknown source: " + ctx
	case srcSlot:
		if src.slot.isResult {
			return a.unknownRoot(src.slot, seen, depth+1)
		}
	case srcField:
		return "field of unanalyzed package " + src.v.Name()
	case srcAny, srcAll:
		for _, x := range src.list {
			if a.eval(x, map[*types.Var]bool{}) == nsUnknown && !(x.kind == srcSlot && seen[x.slot]) {
				return a.unknownSrc(x, ctx, seen, vis, depth)
			}
		}
	case srcVar:
		if vis[src.v] {
			return "varcycle"
		}
		vis[src.v] = true
		vi := a.vars[src.v]
		if vi.init != nil && !vi.initKilled && !src.noInit && a.eval(*vi.init, map[*types.Var]bool{}) == nsUnknown {
			return a.unknownSrc(*vi.init, ctx, seen, vis, depth)
		}
		for _, as := range vi.assigns {
			if a.eval(as.src, map[*types.Var]bool{}) == nsUnknown {
				return a.unknownSrc(as.src, ctx, seen, vis, depth)
			}
		}
	}
	return "?: " + ctx
}

// flowRoot follows nil provenance of a parameter back to where the nil literal enters.
func (a *Analyzer) flowRoot(s *Slot, depth int) string {
	if s.flowFrom == nil || depth > 50 {
		return ownerName(s) + ": " + s.Why
	}
	return a.flowRootSrc(s.flowFrom.src, s.flowFrom.why, map[*types.Var]bool{}, depth)
}

func (a *Analyzer) flowRootSrc(src source, ctx string, vis map[*types.Var]bool, depth int) string {
	switch src.kind {
	case srcNil:
		return "nil literal/zero value: " + ctx
	case srcSlot:
		if !src.slot.isResult && src.slot.NilFlow {
			return a.flowRoot(src.slot, depth+1)
		}
	case srcAny, srcAll:
		for _, x := range src.list {
			if a.carriesNil(x, map[*types.Var]bool{}) {
				return a.flowRootSrc(x, ctx, vis, depth)
			}
		}
	case srcVar:
		if vis[src.v] {
			return "cycle " + ctx
		}
		vis[src.v] = true
		vi := a.vars[src.v]
		if vi.init != nil && !vi.initKilled && !src.noInit && a.carriesNil(*vi.init, map[*types.Var]bool{}) {
			if vi.init.kind == srcNil {
				return "zero value of " + src.v.Name() + " (" + a.pos(src.v.Pos()) + ")"
			}
			return a.flowRootSrc(*vi.init, ctx, vis, depth)
		}
		for _, as := range vi.assigns {
			if a.carriesNil(as.src, map[*types.Var]bool{}) {
				if as.src.kind == srcNil {
					return "nil assigned to " + src.v.Name() + " (" + a.pos(src.v.Pos()) + ")"
				}
				return a.flowRootSrc(as.src, ctx, vis, depth)
			}
		}
	}
	return "? " + ctx
}
