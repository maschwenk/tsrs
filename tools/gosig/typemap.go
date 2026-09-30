package main

import (
	"fmt"
	"go/types"
	"strings"
)

type tpos int

const (
	posParam     tpos = iota // top-level parameter
	posResult                // top-level result
	posElem                  // inside an owned container / stored value
	posElemParam             // element of a parameter slice
	posCbParam               // parameter of a callback type
)

const unmappedMark = " /*?*/"

type TypeMapper struct {
	foreign  map[string]int // named types from other packages seen while counting
	cfg      *Config
	main     *types.Package
	checker  *types.TypeName
	unmapped map[string]int
	counting bool
	ifaces   []resolvedIface
}

type resolvedIface struct {
	iface  *types.Interface
	rust   string
	except []string
}

// resolveIfaces looks up the configured data interfaces among the loaded packages.
func (m *TypeMapper) resolveIfaces() []string {
	pkgs := map[string]*types.Package{"": m.main}
	var visit func(p *types.Package)
	visit = func(p *types.Package) {
		for _, q := range p.Imports() {
			if _, ok := pkgs[q.Name()]; !ok {
				pkgs[q.Name()] = q
				visit(q)
			}
		}
	}
	visit(m.main)
	var missing []string
	for _, di := range m.cfg.DataInterfaces {
		pkgName, name := "", di.Iface
		if i := strings.LastIndex(di.Iface, "."); i >= 0 {
			pkgName, name = di.Iface[:i], di.Iface[i+1:]
		}
		p := pkgs[pkgName]
		var obj types.Object
		if p != nil {
			obj = p.Scope().Lookup(name)
		}
		if obj == nil {
			missing = append(missing, di.Iface)
			continue
		}
		it, ok := obj.Type().Underlying().(*types.Interface)
		if !ok {
			missing = append(missing, di.Iface)
			continue
		}
		m.ifaces = append(m.ifaces, resolvedIface{it, di.Rust, di.Except})
	}
	return missing
}

func (m *TypeMapper) qual(p *types.Package) string {
	if p == m.main {
		return ""
	}
	return p.Name()
}

func (m *TypeMapper) typeString(t types.Type) string { return types.TypeString(t, m.qual) }

func (m *TypeMapper) miss(goType, guess string) string {
	if m.counting {
		m.unmapped[goType]++
	}
	return guess + unmappedMark
}

func (m *TypeMapper) isChecker(t types.Type) bool {
	p, ok := types.Unalias(t).(*types.Pointer)
	if !ok {
		return false
	}
	n, ok := types.Unalias(p.Elem()).(*types.Named)
	return ok && n.Origin().Obj() == m.checker
}

// holdsChecker reports whether a type is one of the short-lived helper structs holding
// `c *Checker` (config checkerHolders), ported as `Name<'c> { c: &'c mut Checker, … }`.
func (m *TypeMapper) holdsChecker(n *types.Named) bool {
	o := n.Origin().Obj()
	_, ok := m.cfg.CheckerHolders[o.Name()]
	return o.Pkg() == m.main && ok
}

func (m *TypeMapper) paramStyle() bool { return m.cfg.CheckerHolderStyle == "param" }

// unlistedHolders returns structs of the main package with a *Checker field that are
// neither checker holders nor arena types (a config review is needed).
func (m *TypeMapper) unlistedHolders() []string {
	var out []string
	if m.checker == nil {
		return nil // the package has no state-machine type (e.g. pseudochecker, modulespecifiers)
	}
	for _, name := range m.main.Scope().Names() {
		tn, ok := m.main.Scope().Lookup(name).(*types.TypeName)
		_, holder := m.cfg.CheckerHolders[name]
		if !ok || tn.IsAlias() || name == m.checker.Name() || holder || contains(m.cfg.ArenaTypes, name) || contains(m.cfg.NotCheckerHolders, name) {
			continue
		}
		st, ok := tn.Type().Underlying().(*types.Struct)
		if !ok {
			continue
		}
		for i := 0; i < st.NumFields(); i++ {
			if m.isChecker(st.Field(i).Type()) {
				out = append(out, name)
				break
			}
		}
	}
	return out
}

// nilable reports whether values of t can be nil in a way that maps to Option in Rust.
func (m *TypeMapper) nilable(t types.Type) bool {
	switch u := types.Unalias(t).(type) {
	case *types.Pointer, *types.Signature, *types.Interface:
		return true
	case *types.Named:
		switch u.Underlying().(type) {
		case *types.Pointer, *types.Signature, *types.Interface:
			return true
		case *types.Map:
			r := m.cfg.TypeMap[m.typeString(u)]
			return strings.HasPrefix(r, "P<")
		}
	}
	return false
}

func (m *TypeMapper) namedName(n *types.Named) string {
	name := n.Obj().Name()
	if m.counting && n.Obj().Pkg() != nil && n.Obj().Pkg() != m.main {
		if m.foreign == nil {
			m.foreign = map[string]int{}
		}
		m.foreign[n.Obj().Pkg().Name()+"."+name]++
	}
	if args := n.TypeArgs(); args != nil && args.Len() > 0 {
		var parts []string
		for i := 0; i < args.Len(); i++ {
			parts = append(parts, m.rust(args.At(i), posElem, "", nil))
		}
		name += "<" + strings.Join(parts, ", ") + ">"
	}
	return name
}

// rust maps a Go type to Rust. cb is the checker-first callback argument ("" for none);
// sub carries nested nil-ability for func types.
func (m *TypeMapper) rust(t types.Type, p tpos, cb string, sub *SigSlots) string {
	key := m.typeString(t)
	if r, ok := m.cfg.TypeMap[key]; ok {
		return r
	}
	if a, ok := t.(*types.Alias); ok {
		return m.rust(types.Unalias(a), p, cb, sub)
	}
	switch u := t.(type) {
	case *types.Basic:
		switch u.Kind() {
		case types.Bool:
			return "bool"
		case types.Int, types.Int32:
			return "i32"
		case types.Int8:
			return "i8"
		case types.Int16:
			return "i16"
		case types.Int64:
			return "i64"
		case types.Uint, types.Uint64:
			return "u64"
		case types.Uint8:
			return "u8"
		case types.Uint16:
			return "u16"
		case types.Uint32:
			return "u32"
		case types.Uintptr:
			return "usize"
		case types.Float32:
			return "f32"
		case types.Float64:
			return "f64"
		case types.String:
			if p == posParam || p == posCbParam || p == posElemParam {
				return "&str"
			}
			return "String"
		}
		return m.miss(key, key)
	case *types.Pointer:
		el := types.Unalias(u.Elem())
		if r, ok := m.cfg.TypeMap["*"+m.typeString(el)]; ok {
			return r
		}
		if n, ok := el.(*types.Named); ok {
			for _, ri := range m.ifaces {
				if !contains(ri.except, n.Obj().Name()) && types.Implements(u, ri.iface) {
					if !strings.Contains(ri.rust, "{T}") {
						return ri.rust
					}
					return strings.ReplaceAll(ri.rust, "{T}", m.namedName(n))
				}
			}
			if _, isStruct := n.Underlying().(*types.Struct); isStruct {
				if n.Origin().Obj() == m.checker {
					return "&mut " + n.Obj().Name()
				}
				if m.holdsChecker(n) && !contains(m.cfg.ArenaTypes, n.Obj().Name()) {
					if m.paramStyle() && !strings.Contains(m.cfg.CheckerHolders[n.Obj().Name()], "'") {
						return "&mut " + n.Obj().Name()
					}
					return "&mut " + n.Obj().Name() + "<'_>"
				}
				return "P<" + m.namedName(n) + ">"
			}
		}
		return "&mut " + m.rust(el, posElem, cb, nil)
	case *types.Named:
		if args := u.TypeArgs(); args != nil && args.Len() > 0 {
			on := u.Origin().Obj()
			qn := on.Name()
			if on.Pkg() != nil && on.Pkg() != m.main {
				qn = on.Pkg().Name() + "." + qn
			}
			tmpl, ok := m.cfg.TypeMap[qn+"@param"]
			if !ok || (p != posParam && p != posCbParam) {
				tmpl, ok = m.cfg.TypeMap[qn]
			}
			if ok {
				for i := 0; i < args.Len(); i++ {
					tmpl = strings.ReplaceAll(tmpl, fmt.Sprintf("$%d", i), m.rust(args.At(i), posElem, cb, nil))
				}
				return tmpl
			}
		}
		if _, ok := u.Underlying().(*types.Interface); ok {
			return m.miss(key, m.namedName(u))
		}
		return m.namedName(u)
	case *types.TypeParam:
		return u.Obj().Name()
	case *types.Slice:
		ep := posElem
		if p == posParam || p == posCbParam {
			ep = posElemParam
		}
		el := m.rust(u.Elem(), ep, cb, nil)
		if p == posParam || p == posCbParam {
			return "&[" + el + "]"
		}
		return "Vec<" + el + ">"
	case *types.Array:
		return fmt.Sprintf("[%s; %d]", m.rust(u.Elem(), posElem, cb, nil), u.Len())
	case *types.Map:
		k := m.rust(u.Key(), posElem, cb, nil)
		v := m.rust(u.Elem(), posElem, cb, nil)
		s := "FxHashMap<" + k + ", " + v + ">"
		if p == posParam || p == posCbParam {
			return "&" + s
		}
		return s
	case *types.Signature:
		return m.fnType(u, p, cb, sub)
	case *types.Interface:
		if u.Empty() {
			return m.miss("any", "&dyn std::any::Any")
		}
		return m.miss(key, "&dyn ?")
	}
	return m.miss(key, key)
}

func (m *TypeMapper) fnType(sig *types.Signature, p tpos, cb string, sub *SigSlots) string {
	var args []string
	if cb != "" && !(sig.Params().Len() > 0 && m.isChecker(sig.Params().At(0).Type())) {
		args = append(args, cb)
	}
	n := sig.Params().Len()
	for i := 0; i < n; i++ {
		t := sig.Params().At(i).Type()
		var s string
		if sig.Variadic() && i == n-1 {
			s = m.variadic(t.(*types.Slice).Elem(), cb)
		} else {
			var ss *SigSlots
			var opt bool
			if sub != nil && i < len(sub.Params) {
				ss, opt = sub.Params[i].Sub, sub.Params[i].Option
			}
			s = m.rust(t, posCbParam, cb, ss)
			if opt {
				s = wrapOption(s)
			}
		}
		args = append(args, s)
	}
	out := "FnMut(" + strings.Join(args, ", ") + ")" + m.results(sig, cb, sub)
	switch p {
	case posParam:
		return "impl " + out
	case posCbParam:
		return "&mut dyn " + out
	}
	return "Box<dyn " + out + ">"
}

func (m *TypeMapper) variadic(elem types.Type, cb string) string {
	if it, ok := types.Unalias(elem).(*types.Interface); ok && it.Empty() {
		return "&[&dyn Display]"
	}
	return "&[" + m.rust(elem, posElemParam, cb, nil) + "]"
}

func (m *TypeMapper) results(sig *types.Signature, cb string, sub *SigSlots) string {
	n := sig.Results().Len()
	if n == 0 {
		return ""
	}
	var rs []string
	for j := 0; j < n; j++ {
		var ss *SigSlots
		var opt bool
		if sub != nil && j < len(sub.Results) {
			ss, opt = sub.Results[j].Sub, sub.Results[j].Option
		}
		s := m.rust(sig.Results().At(j).Type(), posResult, cb, ss)
		if opt {
			s = wrapOption(s)
		}
		rs = append(rs, s)
	}
	if n == 1 {
		return " -> " + rs[0]
	}
	return " -> (" + strings.Join(rs, ", ") + ")"
}

// wrapOption turns T into Option<T>; `impl FnMut…` becomes `Option<&mut dyn FnMut…>`.
func wrapOption(s string) string {
	mark := ""
	if strings.HasSuffix(s, unmappedMark) {
		s = strings.TrimSuffix(s, unmappedMark)
		mark = unmappedMark
	}
	if strings.HasPrefix(s, "impl ") {
		return "Option<&mut dyn " + strings.TrimPrefix(s, "impl ") + ">" + mark
	}
	return "Option<" + s + ">" + mark
}

func (m *TypeMapper) typeParams(tps *types.TypeParamList) string {
	if tps == nil || tps.Len() == 0 {
		return ""
	}
	var parts []string
	for i := 0; i < tps.Len(); i++ {
		tp := tps.At(i)
		c := m.typeString(tp.Constraint())
		b, ok := m.cfg.ConstraintMap[c]
		if !ok {
			if m.counting {
				m.unmapped["constraint "+c]++
			}
			b = "/*? " + c + " */"
		}
		if b == "" {
			parts = append(parts, tp.Obj().Name())
		} else if strings.HasPrefix(b, "/*") {
			parts = append(parts, tp.Obj().Name()+" "+b)
		} else {
			parts = append(parts, tp.Obj().Name()+": "+b)
		}
	}
	return "<" + strings.Join(parts, ", ") + ">"
}
