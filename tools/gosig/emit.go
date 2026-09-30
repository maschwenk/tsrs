package main

import (
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

type Gen struct {
	handWritten map[string]string // "Recv.rust_name" or ".rust_name" -> hand-written file defining it
	skippedHand []*Func
	cfg         *Config
	l           *Loaded
	a           *Analyzer
	tm          *TypeMapper
	namer       *namer

	emitFile   map[*Func]string // Rust output file for emitted funcs
	advisory   []*Func          // ported elsewhere (hand-ported / later), not emitted
	mutRecv    map[*Func]bool
	needC      map[*Func]bool
	mutParam   map[*types.Var]bool
	collisions []string
	merges     []string
}

var (
	implRe = regexp.MustCompile(`^impl(?:<[^>]*>)?\s+(?:[A-Za-z_][A-Za-z0-9_:<>, ']*\s+for\s+)?([A-Za-z_][A-Za-z0-9_]*)`)
	fnRe   = regexp.MustCompile(`^(\s*)(?:pub(?:\([a-z]+\))?\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)`)

	traitImplRe = regexp.MustCompile(`^impl(?:<[^>]*>)?\s+[A-Za-z_][A-Za-z0-9_:<>, ']*\s+for\s+`)
)

const traitImpl = "\x00trait"

// scanHandWritten records functions defined in the output directory's hand-written files
// (files without the generated-stub marker), so stubs never duplicate them.
func (g *Gen) scanHandWritten() {
	g.handWritten = map[string]string{}
	entries, err := os.ReadDir(g.cfg.OutDir)
	if err != nil {
		return
	}
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".rs") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(g.cfg.OutDir, e.Name()))
		if err != nil || strings.Contains(string(data), stubMarker) {
			continue
		}
		impl := ""
		for _, line := range strings.Split(string(data), "\n") {
			if m := implRe.FindStringSubmatch(line); m != nil {
				impl = m[1]
				if traitImplRe.MatchString(line) {
					impl = traitImpl // trait methods do not stand in for inherent (Go) methods
				}
				continue
			}
			if strings.HasPrefix(line, "}") {
				impl = ""
				continue
			}
			if m := fnRe.FindStringSubmatch(line); m != nil && !(impl == traitImpl && m[1] != "") {
				recv := impl
				if m[1] == "" {
					recv = ""
				}
				if _, ok := g.handWritten[recv+"."+m[2]]; !ok {
					g.handWritten[recv+"."+m[2]] = e.Name()
				}
			}
		}
	}
}

func (g *Gen) handFile(fn *Func) string {
	recv := ""
	if fn.RecvType != nil {
		recv = fn.RecvType.Name()
	}
	return g.handWritten[recv+"."+fn.RustName]
}

func (g *Gen) notPorted(file string) bool {
	if strings.HasSuffix(file, "_test.go") {
		return true
	}
	for _, p := range g.cfg.NotPorted {
		if ok, _ := filepath.Match(p, file); ok {
			return true
		}
	}
	return false
}

func (g *Gen) funcKey(fn *Func) string {
	if fn.RecvType != nil {
		return fn.RecvType.Name() + "." + fn.Name
	}
	return fn.Name
}

func (g *Gen) fileRank(file string) int {
	for i, r := range g.cfg.Files {
		if r.Go == file {
			return i
		}
	}
	for i, n := range g.l.Target.Names {
		if n == file {
			return 1000 + i
		}
	}
	return 10000
}

func (g *Gen) plan() {
	g.emitFile = map[*Func]string{}
	for _, fn := range g.a.order {
		if g.notPorted(fn.File) {
			continue
		}
		if fn.IsField {
			if fn.RecvType == g.tm.checker && g.cfg.FieldsFile != "" && !contains(g.cfg.SkipFuncs, g.funcKey(fn)) {
				g.emitFile[fn] = g.cfg.FieldsFile
			}
			continue
		}
		if fn.Name == "init" || fn.Name == "_" {
			continue
		}
		rf, rule := g.cfg.rustFileFor(fn.File, fn.Line)
		if rule == nil || contains(g.cfg.SkipFuncs, g.funcKey(fn)) {
			g.advisory = append(g.advisory, fn)
			continue
		}
		if rf == "" {
			fmt.Fprintf(os.Stderr, "warning: %s:%d not covered by any chunk\n", fn.File, fn.Line)
			continue
		}
		g.emitFile[fn] = rf
	}
}

// computeMutation decides &self vs &mut self for plain helper types, and which slice/map
// parameters are written through.
func (g *Gen) computeMutation() {
	info := g.l.Target.Info
	g.mutRecv = map[*Func]bool{}
	g.mutParam = map[*types.Var]bool{}
	deps := map[*Func][]*Func{}
	rootVar := func(e ast.Expr) (*types.Var, bool) {
		bare := true
		for {
			switch x := e.(type) {
			case *ast.ParenExpr:
				e = x.X
				continue
			case *ast.SelectorExpr:
				e, bare = x.X, false
				continue
			case *ast.IndexExpr:
				e, bare = x.X, false
				continue
			case *ast.StarExpr:
				e, bare = x.X, false
				continue
			case *ast.Ident:
				v, _ := info.Uses[x].(*types.Var)
				return v, bare
			}
			return nil, bare
		}
	}
	mutatingCalls := map[string]int{"delete": 0, "clear": 0, "slices.Sort": 0, "slices.SortFunc": 0,
		"slices.SortStableFunc": 0, "slices.Reverse": 0, "sort.Slice": 0, "sort.SliceStable": 0}
	for _, fn := range g.a.order {
		if fn.Decl == nil || fn.Decl.Body == nil || fn.External {
			continue
		}
		var recv *types.Var
		if fn.Decl.Recv != nil && len(fn.Decl.Recv.List) > 0 && len(fn.Decl.Recv.List[0].Names) > 0 {
			recv, _ = info.Defs[fn.Decl.Recv.List[0].Names[0]].(*types.Var)
		}
		params := map[*types.Var]bool{}
		for i := 0; i < fn.Sig.Params().Len(); i++ {
			params[fn.Sig.Params().At(i)] = true
		}
		writeTo := func(lhs ast.Expr) {
			v, bare := rootVar(lhs)
			if v == nil || bare {
				return
			}
			if recv != nil && v == recv {
				g.mutRecv[fn] = true
			}
			if ix, ok := unparen(lhs).(*ast.IndexExpr); ok {
				if id, ok := unparen(ix.X).(*ast.Ident); ok {
					if pv, ok := info.Uses[id].(*types.Var); ok && params[pv] {
						g.mutParam[pv] = true
					}
				}
			}
		}
		ast.Inspect(fn.Decl.Body, func(x ast.Node) bool {
			switch x := x.(type) {
			case *ast.AssignStmt:
				for _, l := range x.Lhs {
					writeTo(l)
				}
			case *ast.IncDecStmt:
				writeTo(x.X)
			case *ast.UnaryExpr:
				if x.Op == token.AND {
					if v, bare := rootVar(x.X); v != nil && recv != nil && v == recv && !bare {
						g.mutRecv[fn] = true
					}
				}
			case *ast.CallExpr:
				if f := g.a.calledFunc(x); f != nil {
					if _, ok := mutatingCalls[funcKey(f)]; ok && len(x.Args) > 0 {
						if id, ok := unparen(x.Args[0]).(*ast.Ident); ok {
							if pv, ok := info.Uses[id].(*types.Var); ok && params[pv] {
								g.mutParam[pv] = true
							}
						}
					}
				} else if id, ok := unparen(x.Fun).(*ast.Ident); ok && (id.Name == "delete" || id.Name == "clear") && len(x.Args) > 0 {
					if aid, ok := unparen(x.Args[0]).(*ast.Ident); ok {
						if pv, ok := info.Uses[aid].(*types.Var); ok && params[pv] {
							g.mutParam[pv] = true
						}
					}
					if v, bare := rootVar(x.Args[0]); v != nil && recv != nil && v == recv && !bare {
						g.mutRecv[fn] = true
					}
				}
				sel, ok := unparen(x.Fun).(*ast.SelectorExpr)
				if !ok || recv == nil {
					break
				}
				s := info.Selections[sel]
				if s == nil || s.Kind() != types.MethodVal {
					break
				}
				v, bare := rootVar(sel.X)
				if v != recv {
					break
				}
				m, _ := s.Obj().(*types.Func)
				if bare {
					if callee := g.a.funcs[m.Origin()]; callee != nil {
						deps[fn] = append(deps[fn], callee)
					}
				} else if _, isPtr := m.Type().(*types.Signature).Recv().Type().(*types.Pointer); isPtr {
					if tv, ok := info.Types[sel.X]; ok {
						if _, xPtr := tv.Type.Underlying().(*types.Pointer); !xPtr {
							g.mutRecv[fn] = true
						}
					}
				}
			}
			return true
		})
	}
	for changed := true; changed; {
		changed = false
		for fn, ds := range deps {
			if g.mutRecv[fn] {
				continue
			}
			for _, d := range ds {
				if g.mutRecv[d] {
					g.mutRecv[fn] = true
					changed = true
					break
				}
			}
		}
	}
}

// computeNeedsChecker finds non-Checker functions that use a dropped back-pointer field
// (`t.checker`), directly or through such callees.
func (g *Gen) computeNeedsChecker() {
	info := g.l.Target.Info
	g.needC = map[*Func]bool{}
	calls := map[*Func][]*Func{}
	for _, fn := range g.a.order {
		if fn.Decl == nil || fn.Decl.Body == nil || (fn.RecvType != nil && fn.RecvType == g.tm.checker) {
			continue
		}
		ast.Inspect(fn.Decl.Body, func(x ast.Node) bool {
			switch x := x.(type) {
			case *ast.SelectorExpr:
				if sel := info.Selections[x]; sel != nil && sel.Kind() == types.FieldVal {
					recv := sel.Recv()
					if p, ok := recv.(*types.Pointer); ok {
						recv = p.Elem()
					}
					if n, ok := types.Unalias(recv).(*types.Named); ok && contains(g.cfg.DroppedCheckerFields, n.Obj().Name()+"."+x.Sel.Name) {
						g.needC[fn] = true
					}
				}
			case *ast.CallExpr:
				if _, callee, _ := g.a.callee(x); callee != nil && !callee.External && !callee.IsField {
					calls[fn] = append(calls[fn], callee)
				}
			}
			return true
		})
	}
	for changed := true; changed; {
		changed = false
		for fn, cs := range calls {
			if g.needC[fn] {
				continue
			}
			for _, c := range cs {
				if g.needC[c] && (c.RecvType == nil || c.RecvType != g.tm.checker) {
					g.needC[fn] = true
					changed = true
					break
				}
			}
		}
	}
}

func (g *Gen) isArena(tn *types.TypeName) bool { return contains(g.cfg.ArenaTypes, tn.Name()) }

func (g *Gen) recvKind(fn *Func) string {
	if fn.IsField {
		return "&mut self"
	}
	n := fn.RecvType.Type().(*types.Named)
	switch {
	case fn.RecvType == g.tm.checker:
		return "&mut self"
	case g.tm.holdsChecker(n):
		if g.tm.paramStyle() && g.isArena(fn.RecvType) {
			return "&self"
		}
		return "&mut self"
	case g.isArena(fn.RecvType):
		return "&self" // also for Go value receivers (NodeBuilder.SymbolToParameterDeclaration): arena objects are never moved
	case !fn.RecvPtr:
		return "self"
	case g.mutRecv[fn]:
		return "&mut self"
	}
	return "&self"
}

// checkerParam reports whether the Rust signature gets an explicit `c: &mut Checker`.
func (g *Gen) checkerParam(fn *Func) bool {
	if fn.IsField || (fn.RecvType != nil && fn.RecvType == g.tm.checker) {
		return false
	}
	for i := 0; i < fn.Sig.Params().Len(); i++ {
		if g.tm.isChecker(fn.Sig.Params().At(i).Type()) {
			return false // Go already passes the checker explicitly
		}
	}
	if fn.RecvType != nil && g.tm.holdsChecker(fn.RecvType.Type().(*types.Named)) && g.tm.paramStyle() {
		return true
	}
	return g.needC[fn]
}

func (g *Gen) cbFor(fn *Func) string {
	if g.checkerParam(fn) {
		return "&mut Checker"
	}
	if fn.RecvType != nil {
		if fn.RecvType == g.tm.checker || g.tm.holdsChecker(fn.RecvType.Type().(*types.Named)) {
			return "&mut Checker"
		}
	}
	for i := 0; i < fn.Sig.Params().Len(); i++ {
		if g.tm.isChecker(fn.Sig.Params().At(i).Type()) {
			return "&mut Checker"
		}
	}
	return ""
}

func (g *Gen) implHeader(tn *types.TypeName) string {
	n := tn.Type().(*types.Named)
	name := tn.Name()
	if tn == g.tm.checker {
		return "impl " + name
	}
	if g.tm.holdsChecker(n) {
		if h := g.cfg.CheckerHolders[name]; h != "" {
			return h
		}
		if g.tm.paramStyle() {
			return "impl " + name
		}
		return "impl<'c> " + name + "<'c>"
	}
	tps := n.TypeParams()
	if tps.Len() == 0 {
		return "impl " + name
	}
	var names []string
	for i := 0; i < tps.Len(); i++ {
		names = append(names, tps.At(i).Obj().Name())
	}
	return "impl" + g.tm.typeParams(tps) + " " + name + "<" + strings.Join(names, ", ") + ">"
}

func (g *Gen) paramName(name string, i int, used map[string]bool) string {
	var s string
	switch name {
	case "_":
		return "_"
	case "":
		s = fmt.Sprintf("arg%d", i)
	default:
		s = g.namer.ident(name)
	}
	base := s
	for k := 2; used[s]; k++ {
		s = fmt.Sprintf("%s%d", base, k)
	}
	used[s] = true
	return s
}

// signature renders `vis fn name<generics>(params) -> results`.
func (g *Gen) signature(fn *Func, name, vis string) string {
	cb := g.cbFor(fn)
	var params []string
	if fn.RecvType != nil {
		params = append(params, g.recvKind(fn))
	}
	used := map[string]bool{"self": true}
	if g.checkerParam(fn) {
		params = append(params, "c: &mut Checker")
		used["c"] = true
	}
	n := fn.Sig.Params().Len()
	for i := 0; i < n; i++ {
		pv := fn.Sig.Params().At(i)
		pn := g.paramName(pv.Name(), i, used)
		var ts string
		if fn.Sig.Variadic() && i == n-1 {
			ts = g.tm.variadic(pv.Type().(*types.Slice).Elem(), cb)
		} else if pt, ok := g.cfg.ParamTypes[g.funcKey(fn)+"."+pv.Name()]; ok {
			ts = pt
			if fn.Slots.Params[i].Option {
				ts = wrapOption(ts)
			}
		} else {
			slot := fn.Slots.Params[i]
			ts = g.tm.rust(pv.Type(), posParam, cb, slot.Sub)
			if g.mutParam[pv] && (strings.HasPrefix(ts, "&[") || strings.HasPrefix(ts, "&FxHashMap")) {
				ts = "&mut " + ts[1:]
			}
			if slot.Option {
				ts = wrapOption(ts)
			}
		}
		params = append(params, pn+": "+ts)
	}
	generics := g.tm.typeParams(fn.Sig.TypeParams())
	res := g.tm.results(fn.Sig, cb, fn.Slots)
	if rt, ok := g.cfg.ParamTypes[g.funcKey(fn)+".r0"]; ok && fn.Sig.Results().Len() == 1 {
		res = " -> " + rt // paramTypes "Recv.goName.r0": the Rust result type, as written
	}
	return fmt.Sprintf("%s fn %s%s(%s)%s", vis, name, generics, strings.Join(params, ", "), res)
}

func (g *Gen) vis(fn *Func) string {
	if fn.Pub || ast.IsExported(fn.Name) {
		return "pub"
	}
	return "pub(crate)"
}

// forwardTarget returns the function fn trivially forwards to (same args, same order).
func (g *Gen) forwardTarget(fn *Func) types.Object {
	if fn.Decl == nil || fn.Decl.Body == nil || len(fn.Decl.Body.List) != 1 {
		return nil
	}
	info := g.l.Target.Info
	var call *ast.CallExpr
	switch s := fn.Decl.Body.List[0].(type) {
	case *ast.ReturnStmt:
		if len(s.Results) == 1 {
			call, _ = unparen(s.Results[0]).(*ast.CallExpr)
		}
	case *ast.ExprStmt:
		call, _ = unparen(s.X).(*ast.CallExpr)
	}
	if call == nil || len(call.Args) != fn.Sig.Params().Len() {
		return nil
	}
	for i, arg := range call.Args {
		id, ok := unparen(arg).(*ast.Ident)
		if !ok || info.Uses[id] != fn.Sig.Params().At(i) {
			return nil
		}
	}
	_, callee, _ := g.a.callee(call)
	if callee == nil {
		return nil
	}
	return callee.Obj
}

func (g *Gen) assignNames() {
	type member struct {
		fn   *Func
		rank int
	}
	spaces := map[string][]*Func{}
	for _, fn := range g.a.order {
		if g.notPorted(fn.File) {
			continue
		}
		if fn.IsField && g.emitFile[fn] == "" {
			continue
		}
		if !fn.IsField && contains(g.cfg.SkipFuncs, g.funcKey(fn)) {
			continue // skipped functions reserve no name and are never merge targets/sources
		}
		ns := ""
		if fn.RecvType != nil {
			ns = fn.RecvType.Name()
		}
		spaces[ns] = append(spaces[ns], fn)
	}
	nsNames := make([]string, 0, len(spaces))
	for ns := range spaces {
		nsNames = append(nsNames, ns)
	}
	sort.Strings(nsNames)
	for _, ns := range nsNames {
		fns := spaces[ns]
		sort.SliceStable(fns, func(i, j int) bool {
			ei, ej := ast.IsExported(fns[i].Name), ast.IsExported(fns[j].Name)
			if ei != ej {
				return !ei
			}
			ri, rj := g.fileRank(fns[i].File), g.fileRank(fns[j].File)
			if ri != rj {
				return ri < rj
			}
			return fns[i].Line < fns[j].Line
		})
		taken := map[string]*Func{}
		for _, fn := range fns {
			nm := g.namer.ident(fn.Name)
			winner := taken[nm]
			if winner == nil {
				fn.RustName = nm
				taken[nm] = fn
				continue
			}
			where := fmt.Sprintf("%s:%d", fn.File, fn.Line)
			if ast.IsExported(fn.Name) && g.forwardTarget(fn) == winner.Obj &&
				g.signature(fn, "x", "") == g.signature(winner, "x", "") {
				fn.MergedInto = winner
				fn.RustName = nm
				winner.Pub = true
				g.merges = append(g.merges, fmt.Sprintf("%s (%s) merged into %s (now pub)", g.funcKey(fn), where, g.funcKey(winner)))
				continue
			}
			suffix := "_2"
			if ast.IsExported(fn.Name) {
				suffix = "_exported"
			}
			alt := nm + suffix
			for k := 3; taken[alt] != nil; k++ {
				alt = fmt.Sprintf("%s_%d", nm, k)
			}
			fn.RustName = alt
			taken[alt] = fn
			g.collisions = append(g.collisions, fmt.Sprintf("%s (%s) collides with %s -> %s", g.funcKey(fn), where, g.funcKey(winner), alt))
		}
	}
}

func (g *Gen) recvName(fn *Func) string {
	if fn.RecvType == nil {
		return "-"
	}
	return fn.RecvType.Name()
}

type sigLine struct{ name, recv, text string }

func wrapList(prefix string, items []string, width int) []string {
	var lines []string
	cur := prefix
	for i, it := range items {
		sep := ", "
		if i == 0 {
			sep = ""
		}
		if len(cur)+len(sep)+len(it) > width && cur != prefix {
			lines = append(lines, cur+",")
			cur = "//     " + it
			continue
		}
		cur += sep + it
	}
	return append(lines, cur)
}

// declComment lists non-function declarations (and skipped functions) of a Go range.
func (g *Gen) declComment(goFile string, from, to int, note string) string {
	var af *ast.File
	for i, n := range g.l.Target.Names {
		if n == goFile {
			af = g.l.Target.Files[i]
		}
	}
	if af == nil {
		return ""
	}
	var lines []string
	for _, d := range af.Decls {
		line := g.l.Fset.Position(d.Pos()).Line
		if line < from || (to > 0 && line > to) {
			continue
		}
		switch d := d.(type) {
		case *ast.GenDecl:
			var names []string
			for _, sp := range d.Specs {
				switch sp := sp.(type) {
				case *ast.TypeSpec:
					names = append(names, sp.Name.Name)
				case *ast.ValueSpec:
					for _, n := range sp.Names {
						names = append(names, n.Name)
					}
				}
			}
			if len(names) == 0 {
				continue
			}
			if len(names) == 1 {
				lines = append(lines, fmt.Sprintf("//   %s %s (%s:%d)", d.Tok, names[0], goFile, line))
			} else {
				lines = append(lines, wrapList(fmt.Sprintf("//   %s (%s:%d): ", d.Tok, goFile, line), names, 110)...)
			}
		case *ast.FuncDecl:
			fn := g.a.funcs[g.l.Target.Info.Defs[d.Name]]
			if fn != nil && contains(g.cfg.SkipFuncs, g.funcKey(fn)) {
				lines = append(lines, fmt.Sprintf("//   func %s %s:%d (not generated)", g.funcKey(fn), goFile, line))
			}
		}
	}
	if len(lines) == 0 {
		return ""
	}
	rng := fmt.Sprintf("%s:%d-%d", goFile, from, to)
	if to <= 0 {
		rng = goFile
	} else if to >= 99999 {
		rng = fmt.Sprintf("%s:%d-end", goFile, from)
	}
	head := fmt.Sprintf("// Non-function declarations in %s", rng)
	if note != "" {
		head += " (" + note + ")"
	}
	return head + ":\n" + strings.Join(lines, "\n") + "\n"
}

const stubMarker = "// Stubs generated by tools/gosig. Keep the signatures; replace the todo!() bodies."

type Stats struct {
	funcs, fields, ptrParams, optParams, ptrResults, optResults int
}

func (g *Gen) emit(dry bool) (Stats, error) {
	var st Stats
	g.tm.counting = true
	outs := map[string]*strings.Builder{}
	var outOrder []string
	get := func(name string) *strings.Builder {
		b := outs[name]
		if b == nil {
			b = &strings.Builder{}
			b.WriteString(strings.TrimRight(g.cfg.Prelude, "\n") + "\n\n")
			b.WriteString(stubMarker + "\n")
			outs[name] = b
			outOrder = append(outOrder, name)
		}
		return b
	}
	var sigs []sigLine

	countSlots := func(fn *Func) {
		for i, p := range fn.Slots.Params {
			if fn.Sig.Variadic() && i == len(fn.Slots.Params)-1 {
				continue
			}
			if p.Nilable {
				st.ptrParams++
				if p.Option {
					st.optParams++
				}
			}
		}
		for _, r := range fn.Slots.Results {
			if r.Nilable {
				st.ptrResults++
				if r.Option {
					st.optResults++
				}
			}
		}
	}

	writeFuncs := func(b *strings.Builder, fns []*Func) {
		var kept []*Func
		var hand []string
		for _, fn := range fns {
			if hf := g.handFile(fn); hf != "" {
				g.skippedHand = append(g.skippedHand, fn)
				hand = append(hand, fmt.Sprintf("%s (%s)", fn.RustName, hf))
				sig := g.signature(fn, fn.RustName, g.vis(fn))
				sigs = append(sigs, sigLine{fn.RustName, g.recvName(fn), fmt.Sprintf("%s | %s | %s:%d | %s", fn.RustName, sig, fn.File, fn.Line, g.recvName(fn))})
				continue
			}
			kept = append(kept, fn)
		}
		if len(hand) > 0 {
			b.WriteString("\n")
			b.WriteString(strings.Join(wrapList("// Already defined by hand, not generated: ", hand, 110), "\n") + "\n")
		}
		fns = kept
		var curImpl *types.TypeName
		open := false
		closeImpl := func() {
			if open {
				b.WriteString("}\n")
				open = false
			}
		}
		for _, fn := range fns {
			sig := g.signature(fn, fn.RustName, g.vis(fn))
			sigs = append(sigs, sigLine{fn.RustName, g.recvName(fn), fmt.Sprintf("%s | %s | %s:%d | %s", fn.RustName, sig, fn.File, fn.Line, g.recvName(fn))})
			countSlots(fn)
			if fn.IsField {
				st.fields++
			} else {
				st.funcs++
			}
			indent := ""
			if fn.RecvType != nil {
				if !open || curImpl != fn.RecvType {
					closeImpl()
					b.WriteString("\n" + g.implHeader(fn.RecvType) + " {\n")
					open, curImpl = true, fn.RecvType
				} else {
					b.WriteString("\n")
				}
				indent = "    "
			} else {
				closeImpl()
				b.WriteString("\n")
			}
			fmt.Fprintf(b, "%s// %s:%d\n%s%s {\n%s    todo!()\n%s}\n", indent, fn.File, fn.Line, indent, sig, indent, indent)
		}
		closeImpl()
	}

	byFile := map[string][]*Func{}
	for _, fn := range g.a.order {
		if g.emitFile[fn] != "" && fn.MergedInto == nil && !fn.IsField {
			byFile[fn.File] = append(byFile[fn.File], fn)
		}
	}
	seenRust := map[string]bool{}
	for _, rule := range g.cfg.Files {
		chunks := rule.Chunks
		if len(chunks) == 0 {
			chunks = []Chunk{{Rust: rule.Rust, From: 1, To: 0}}
		}
		fns := byFile[rule.Go]
		sort.SliceStable(fns, func(i, j int) bool { return fns[i].Line < fns[j].Line })
		for _, ch := range chunks {
			b := get(ch.Rust)
			if seenRust[ch.Rust] {
				fmt.Fprintf(b, "\n// ---- %s ----\n", rule.Go)
			}
			seenRust[ch.Rust] = true
			if dc := g.declComment(rule.Go, ch.From, ch.To, rule.DeclsNote); dc != "" {
				b.WriteString("\n" + dc)
			}
			var in []*Func
			for _, fn := range fns {
				if g.emitFile[fn] == ch.Rust {
					in = append(in, fn)
				}
			}
			writeFuncs(b, in)
		}
	}
	if g.cfg.FieldsFile != "" {
		var fields []*Func
		for _, fn := range g.a.order {
			if fn.IsField && g.emitFile[fn] != "" {
				fields = append(fields, fn)
			}
		}
		var kept []*Func
		for _, fn := range fields {
			if g.handFile(fn) == "" {
				kept = append(kept, fn)
			} else {
				g.skippedHand = append(g.skippedHand, fn)
				sig := g.signature(fn, fn.RustName, g.vis(fn))
				sigs = append(sigs, sigLine{fn.RustName, g.recvName(fn), fmt.Sprintf("%s | %s | %s:%d | %s", fn.RustName, sig, fn.File, fn.Line, g.recvName(fn))})
			}
		}
		if len(kept) > 0 {
			b := get(g.cfg.FieldsFile)
			b.WriteString("\n// Function-valued fields of Checker, called like methods.\n")
			writeFuncs(b, kept)
		}
	}

	sort.Slice(sigs, func(i, j int) bool {
		if sigs[i].name != sigs[j].name {
			return sigs[i].name < sigs[j].name
		}
		return sigs[i].recv < sigs[j].recv
	})
	g.tm.counting = false
	var adv []sigLine
	for _, fn := range g.advisory {
		if fn.RustName == "" || fn.MergedInto != nil {
			continue
		}
		sig := g.signature(fn, fn.RustName, g.vis(fn))
		adv = append(adv, sigLine{fn.RustName, g.recvName(fn), fmt.Sprintf("%s | %s | %s:%d | %s", fn.RustName, sig, fn.File, fn.Line, g.recvName(fn))})
	}
	sort.Slice(adv, func(i, j int) bool {
		if adv[i].name != adv[j].name {
			return adv[i].name < adv[j].name
		}
		return adv[i].recv < adv[j].recv
	})
	if dry {
		return st, nil
	}
	protected := map[string]bool{"lib.rs": true, "types.rs": true, "links.rs": true, "mapper.rs": true, "checker.rs": true, "program.rs": true}
	for _, name := range outOrder {
		if protected[name] {
			return st, fmt.Errorf("refusing to write protected file %s", name)
		}
		if err := os.WriteFile(filepath.Join(g.cfg.OutDir, name), []byte(outs[name].String()), 0o644); err != nil {
			return st, err
		}
	}
	writeSigs := func(path, header string, lines []sigLine) error {
		if path == "" {
			return nil
		}
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return err
		}
		var b strings.Builder
		b.WriteString(header)
		for _, s := range lines {
			b.WriteString(s.text + "\n")
		}
		return os.WriteFile(path, []byte(b.String()), 0o644)
	}
	if err := writeSigs(g.cfg.SigsFile, "# rust_name | signature | go_file:line | receiver  (generated by tools/gosig)\n", sigs); err != nil {
		return st, err
	}
	if err := writeSigs(g.cfg.AdvisorySigsFile, "# Advisory signatures for functions that are hand-ported or ported later (not generated as stubs).\n# rust_name | signature | go_file:line | receiver  (generated by tools/gosig)\n", adv); err != nil {
		return st, err
	}
	return st, nil
}
