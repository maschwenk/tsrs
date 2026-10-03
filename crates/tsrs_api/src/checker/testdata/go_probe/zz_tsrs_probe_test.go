package api

// tsrs checker-lane differential probe. Copied temporarily into the pinned reference checkout
// (ts-ref/tsc/internal/api) by regen.sh, which removes it again afterwards. Drives the pinned Go API
// session over its JSON request surface with the fixture in ./fixture and the needles in needles.txt,
// and writes go_probe_b85298b6.jsonl. The request sequence mirrors probe_lines() in
// crates/tsrs_api/src/checker/tests.rs; keep both in sync.

import (
	"context"
	"fmt"
	"os"
	"strings"
	"testing"
	"unicode/utf16"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/json"
	"github.com/microsoft/TypeScript/tsc/internal/testutil/projecttestutil"
)

type probe struct {
	s    *Session
	snap float64
	proj string
	out  *os.File
}

func (p *probe) call(method string, params map[string]any) (any, string) {
	b, _ := json.Marshal(params)
	res, err := p.s.HandleRequest(context.Background(), method, json.Value(b))
	if err != nil {
		return nil, err.Error()
	}
	out, _ := json.Marshal(res)
	var v any
	_ = json.Unmarshal(out, &v)
	return v, ""
}

func (p *probe) sp(kv ...any) map[string]any {
	m := map[string]any{"snapshot": p.snap, "project": p.proj}
	for i := 0; i < len(kv); i += 2 {
		m[kv[i].(string)] = kv[i+1]
	}
	return m
}

func (p *probe) emit(label string, v any) {
	b, _ := json.Marshal(map[string]any{"q": label, "r": v}, json.Deterministic(true))
	fmt.Fprintln(p.out, string(b))
}

func u16(text, needle string) int {
	i := strings.Index(text, needle)
	if i < 0 {
		panic(needle)
	}
	return len(utf16.Encode([]rune(text[:i])))
}

func get(v any, k string) any {
	if m, ok := v.(map[string]any); ok {
		return m[k]
	}
	return nil
}

func TestTsrsCheckerProbe(t *testing.T) {
	dir := os.Getenv("TSRS_FX")
	if dir == "" {
		t.Skip("TSRS_FX not set")
	}
	read := func(n string) string { b, _ := os.ReadFile(dir + "/" + n); return string(b) }
	main := read("main.ts")
	files := map[string]any{
		"/p/tsconfig.json": read("tsconfig.json"), "/p/main.ts": main, "/p/types.d.ts": read("types.d.ts"), "/p/other.ts": read("other.ts"),
		"/q/tsconfig.json": `{"files":["q.ts"]}`, "/q/q.ts": "export const q: number = 1;\n",
	}
	init, _ := projecttestutil.GetSessionInitOptions(files, nil, &projecttestutil.TypingsInstallerOptions{})
	s := NewStandaloneSession(init, nil)
	defer s.Close()
	out, _ := os.Create(os.Getenv("TSRS_OUT"))
	defer out.Close()
	p := &probe{s: s, out: out}
	snap, errs := p.call("createSnapshot", map[string]any{"openProjects": []string{"/p/tsconfig.json", "/q/tsconfig.json"}})
	if errs != "" {
		t.Fatal(errs)
	}
	p.snap = get(snap, "snapshot").(float64)
	for _, pr := range get(snap, "projects").([]any) {
		if strings.HasPrefix(get(pr, "configFileName").(string), "/p/") {
			p.proj = get(pr, "id").(string)
		}
	}
	p.emit("flags", map[string]any{"Value": float64(ast.SymbolFlagsValue), "Type": float64(ast.SymbolFlagsType)})
	ts := func(t any) any {
		if t == nil {
			return nil
		}
		r, e := p.call("typeToString", p.sp("type", get(t, "id")))
		if e != "" {
			return e
		}
		return r
	}
	needlesText, _ := os.ReadFile(os.Getenv("TSRS_NEEDLES_FILE"))
	for _, n := range strings.Split(string(needlesText), "\n") {
		if n == "" {
			continue
		}
		pos := u16(main, n)
		ty, e1 := p.call("getTypeAtPosition", p.sp("file", "/p/main.ts", "position", pos))
		sy, e2 := p.call("getSymbolAtPosition", p.sp("file", "/p/main.ts", "position", pos))
		p.emit("at:"+n, map[string]any{"type": ts(ty), "typeFlags": get(ty, "flags"), "objectFlags": get(ty, "objectFlags"), "sym": get(sy, "name"), "symFlags": get(sy, "flags"), "kind": get(get(sy, "reference"), "kind"), "e": e1 + e2})
	}
	symRef := func(needle string) any {
		sy, _ := p.call("getSymbolAtPosition", p.sp("file", "/p/main.ts", "position", u16(main, needle)))
		return get(sy, "reference")
	}
	declared := func(needle string) any {
		t, _ := p.call("getDeclaredTypeOfSymbol", p.sp("symbol", symRef(needle)))
		return t
	}
	// A. conditional check type constraint.
	c := declared("C<T> =")
	check, _ := p.call("getCheckTypeOfType", p.sp("objectId", get(c, "id")))
	cons, e := p.call("getConstraintOfTypeParameter", p.sp("objectId", get(check, "id")))
	// Process-wide symbol ids differ between runs: report identity relations instead.
	p.emit("C.check", map[string]any{"str": ts(check), "flags": get(check, "flags")})
	p.emit("C.constraint", map[string]any{"str": ts(cons), "flags": get(cons, "flags"), "sameAsCheck": get(cons, "id") == get(check, "id"), "sameSymbolAsCheck": get(get(cons, "symbol"), "id") == get(get(check, "symbol"), "id"), "e": e})
	if cons != nil {
		cons2, _ := p.call("getConstraintOfTypeParameter", p.sp("objectId", get(cons, "id")))
		p.emit("C.constraint.constraint", ts(cons2))
	}
	ia := declared("IA<T, K")
	k, _ := p.call("getIndexTypeOfType", p.sp("objectId", get(ia, "id")))
	kc, _ := p.call("getConstraintOfTypeParameter", p.sp("objectId", get(k, "id")))
	p.emit("IA.K.constraint", ts(kc))
	// B. snapshot-less file-owned symbol lookups.
	dog := declared("Dog extends")
	props, _ := p.call("getPropertiesOfType", p.sp("type", get(dog, "id")))
	names := func(v any) any {
		a, ok := v.([]any)
		if !ok {
			return v
		}
		var r []any
		for _, x := range a {
			r = append(r, get(x, "name"))
		}
		return r
	}
	p.emit("Dog.props", names(props))
	legs := props.([]any)[0]
	p.emit("legs.ref.kind", get(get(legs, "reference"), "kind"))
	par, e := p.call("getParentOfSymbol", map[string]any{"symbol": get(legs, "reference")})
	p.emit("legs.parent", map[string]any{"name": get(par, "name"), "kind": get(get(par, "reference"), "kind"), "e": e})
	mem, e := p.call("getMembersOfSymbol", map[string]any{"symbol": symRef("Dog extends")})
	p.emit("Dog.members", map[string]any{"names": names(mem), "e": e})
	mod, _ := p.call("getSymbolOfSourceFile", p.sp("file", "/p/main.ts"))
	exp, e := p.call("getExportsOfSymbol", map[string]any{"symbol": get(mod, "reference")})
	p.emit("main.exportsOfSymbol", map[string]any{"names": names(exp), "e": e})
	expm, e := p.call("getExportsOfModule", p.sp("symbol", get(mod, "reference")))
	p.emit("main.exportsOfModule", map[string]any{"names": names(expm), "e": e})
	scope, _ := p.call("getSymbolsInScope", p.sp("file", "/p/main.ts", "position", u16(main, "box.value"), "meaning", float64(ast.SymbolFlagsValue)))
	for _, sym := range scope.([]any) {
		if get(sym, "name") == "box" {
			es, e := p.call("getExportSymbolOfSymbol", map[string]any{"symbol": get(sym, "reference")})
			p.emit("box.local.exportSymbol", map[string]any{"name": get(es, "name"), "localFlags": get(sym, "flags"), "localKind": get(get(sym, "reference"), "kind"), "exportFlags": get(es, "flags"), "e": e})
			es2, e := p.call("getExportSymbolOfSymbolForChecker", p.sp("symbol", get(sym, "reference")))
			p.emit("box.local.exportSymbolForChecker", map[string]any{"name": get(es2, "name"), "flags": get(es2, "flags"), "e": e})
		}
	}
	// C. resolveName with and without a location.
	for _, q := range [][]any{{"Array", ast.SymbolFlagsType}, {"box", ast.SymbolFlagsValue}, {"Promise", ast.SymbolFlagsValue}, {"Animal", ast.SymbolFlagsValue}} {
		r, e := p.call("resolveName", p.sp("name", q[0], "meaning", float64(q[1].(ast.SymbolFlags))))
		p.emit("resolveName.noloc:"+q[0].(string), map[string]any{"name": get(r, "name"), "flags": get(r, "flags"), "kind": get(get(r, "reference"), "kind"), "e": e})
		r, e = p.call("resolveName", p.sp("name", q[0], "meaning", float64(q[1].(ast.SymbolFlags)), "file", "/p/main.ts", "position", u16(main, "r = over")))
		p.emit("resolveName.loc:"+q[0].(string), map[string]any{"name": get(r, "name"), "flags": get(r, "flags"), "kind": get(get(r, "reference"), "kind"), "e": e})
	}
	// D. stale / foreign handles.
	box, _ := p.call("getTypeAtPosition", p.sp("file", "/p/main.ts", "position", u16(main, "box:")))
	id := get(box, "id")
	_, e = p.call("getTypeArguments", map[string]any{"snapshot": p.snap, "project": "/other/tsconfig.json", "type": id})
	p.emit("err.unknownProject", e)
	qproj := ""
	for _, pr := range get(snap, "projects").([]any) {
		if strings.HasPrefix(get(pr, "configFileName").(string), "/q/") {
			qproj = get(pr, "id").(string)
		}
	}
	r, e := p.call("getTypeArguments", map[string]any{"snapshot": p.snap, "project": qproj, "type": id})
	p.emit("err.foreignProjectType", map[string]any{"r": r, "e": e})
	_, e = p.call("getTypeArguments", p.sp("type", 999999))
	p.emit("err.unknownType", e)
	_, e = p.call("getTypeArguments", map[string]any{"snapshot": p.snap + 1000, "project": p.proj, "type": id})
	p.emit("err.unknownSnapshot", e)
	_, e = p.call("getTypeArguments", p.sp("type", "x"))
	p.emit("err.badType", e)
	arr, _ := p.call("resolveName", p.sp("name", "Array", "meaning", float64(ast.SymbolFlagsType)))
	ref := get(arr, "reference").(map[string]any)
	forged := map[string]any{}
	for k, v := range ref {
		forged[k] = v
	}
	forged["snapshot"] = p.snap + 1
	_, e = p.call("getTypeOfSymbol", p.sp("symbol", forged))
	p.emit("err.forgedSnapshotRef", e)
	qt, e := p.call("getTypeOfSymbol", map[string]any{"snapshot": p.snap, "project": qproj, "symbol": ref})
	p.emit("crossProject.snapshotSymbol", map[string]any{"type": ts2(p, qproj, qt), "e": e})
	_, e = p.call("getParentOfSymbol", map[string]any{"symbol": map[string]any{"kind": 0, "id": 1}})
	p.emit("err.fileRefNoDescriptor", e)
	_, e = p.call("release", map[string]any{"snapshot": p.snap})
	_, e2 := p.call("getTypeArguments", p.sp("type", id))
	p.emit("err.afterRelease", map[string]any{"release": e, "e": e2})
	_, e = p.call("getParentOfSymbol", map[string]any{"symbol": get(legs, "reference")})
	p.emit("afterRelease.fileOwnedParent", e)
}

func ts2(p *probe, proj string, t any) any {
	if t == nil {
		return nil
	}
	r, e := p.call("typeToString", map[string]any{"snapshot": p.snap, "project": proj, "type": get(t, "id")})
	if e != "" {
		return e
	}
	return r
}
