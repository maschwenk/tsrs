package main

import (
	"go/types"
	"strings"
)

const (
	fsPath          = modPath + "/internal/fourslash"
	lsprotoPath     = modPath + "/internal/lsp/lsproto"
	lsutilPath      = modPath + "/internal/ls/lsutil"
	lsPath          = modPath + "/internal/ls"
	corePath        = modPath + "/internal/core"
	utilPath        = modPath + "/internal/fourslash/tests/util"
	collectionsPath = modPath + "/internal/collections"
	modspecPath     = modPath + "/internal/modulespecifiers"
	cmPath          = modPath + "/internal/contentmapper"
	cmtestPath      = modPath + "/internal/testutil/contentmappertest"
	testutilPath    = modPath + "/internal/testutil"
)

// Rust module (as seen from generated code, see tsrs_fourslash::tests::prelude) for each Go package.
var pkgModule = map[string]string{
	fsPath:          "fourslash",
	lsprotoPath:     "lsproto",
	lsutilPath:      "lsutil",
	lsPath:          "ls",
	utilPath:        "util",
	collectionsPath: "collections",
	modspecPath:     "modulespecifiers",
	cmPath:          "contentmapper",
	cmtestPath:      "contentmappertest",
	testutilPath:    "testutil",
	modPath + "/internal/testutil/stringtestutil": "stringtestutil",
	modPath + "/internal/testutil/baseline":       "baseline",
	modPath + "/internal/ls/lsconv":               "lsconv",
	corePath:                                      "gocore",
}

type untranslated struct{ what string }

func fail(format string, args ...any) {
	panic(untranslated{sprintf(format, args...)})
}

// harnessObject reports whether *T is a shared harness object (`Arc<T>` in Rust).
func harnessObject(t types.Type) bool {
	n, ok := types.Unalias(t).(*types.Named)
	if !ok || n.Obj().Pkg() == nil {
		return false
	}
	switch n.Obj().Pkg().Path() {
	case fsPath:
		switch n.Obj().Name() {
		case "Marker", "RangeMarker", "TestFileInfo":
			return true
		}
	case collectionsPath:
		return n.Obj().Name() == "MultiMap"
	}
	return false
}

func isNamed(t types.Type, pkg, name string) bool {
	n, ok := types.Unalias(t).(*types.Named)
	return ok && n.Obj().Pkg() != nil && n.Obj().Pkg().Path() == pkg && n.Obj().Name() == name
}

func isFourslashTestPtr(t types.Type) bool {
	p, ok := types.Unalias(t).(*types.Pointer)
	return ok && isNamed(p.Elem(), fsPath, "FourslashTest")
}

func isTestingTPtr(t types.Type) bool {
	p, ok := types.Unalias(t).(*types.Pointer)
	return ok && isNamed(p.Elem(), "testing", "T")
}

func isString(t types.Type) bool {
	b, ok := types.Unalias(t).Underlying().(*types.Basic)
	return ok && b.Info()&types.IsString != 0 && !stringNewtype(t) && !rustEnumType(t)
}

func isPlainString(t types.Type) bool {
	b, ok := types.Unalias(t).(*types.Basic)
	return ok && b.Info()&types.IsString != 0
}

// stringNewtype: Go named string types that are Rust newtypes over `&'static str` (lsproto string enums).
func stringNewtype(t types.Type) bool {
	n, ok := types.Unalias(t).(*types.Named)
	if !ok || n.Obj().Pkg() == nil {
		return false
	}
	b, ok := n.Underlying().(*types.Basic)
	if !ok || b.Info()&types.IsString == 0 {
		return false
	}
	return n.Obj().Pkg().Path() == lsprotoPath
}

// rustEnumType: Go named basic types that are Rust enums (variants named after the Go constants).
func rustEnumType(t types.Type) bool {
	n, ok := types.Unalias(t).(*types.Named)
	if !ok || n.Obj().Pkg() == nil {
		return false
	}
	switch n.Obj().Pkg().Path() {
	case lsutilPath:
		switch n.Obj().Name() {
		case "QuotePreference", "JsxAttributeCompletionStyle", "IncludeInlayParameterNameHints", "OrganizeImportsSort",
			"OrganizeImportsCollation", "OrganizeImportsCaseFirst", "OrganizeImportsTypeOrder":
			return true
		}
	case modspecPath:
		switch n.Obj().Name() {
		case "ImportModuleSpecifierPreference", "ImportModuleSpecifierEndingPreference":
			return true
		}
	case corePath:
		return n.Obj().Name() == "Tristate"
	}
	return false
}

func isSlice(t types.Type) (*types.Slice, bool) {
	s, ok := types.Unalias(t).Underlying().(*types.Slice)
	return s, ok
}

func isStringSlice(t types.Type) bool {
	s, ok := isSlice(t)
	return ok && isString(s.Elem())
}

func isInterface(t types.Type) bool {
	return types.IsInterface(types.Unalias(t))
}

func isAny(t types.Type) bool {
	i, ok := types.Unalias(t).Underlying().(*types.Interface)
	return ok && i.Empty() && !isNamed(t, cmPath, "Spawner")
}

// copyType: values that are Copy in Rust (no `.clone()` needed).
func copyType(t types.Type) bool {
	t = types.Unalias(t)
	switch u := t.(type) {
	case *types.Basic:
		return u.Info()&types.IsString == 0
	case *types.Named:
		if rustEnumType(t) || stringNewtype(t) {
			return true
		}
		if b, ok := u.Underlying().(*types.Basic); ok && u.Obj().Pkg() != nil && u.Obj().Pkg().Path() == lsprotoPath {
			return b.Info()&types.IsString == 0
		}
		if u.Obj().Pkg() != nil && u.Obj().Pkg().Path() == lsprotoPath {
			switch u.Obj().Name() {
			case "Position", "Range":
				return true
			}
		}
	case *types.Pointer:
		return isTestingTPtr(t)
	}
	return false
}

func namedPath(n *types.Named) string {
	obj := n.Obj()
	if obj.Pkg() == nil {
		return obj.Name()
	}
	switch obj.Pkg().Path() {
	case "testing":
		if obj.Name() == "T" {
			return "T"
		}
	case "strings":
		if obj.Name() == "Builder" {
			return "String"
		}
	case corePath:
		switch obj.Name() {
		case "Tristate":
			return "Tristate"
		}
	case lsPath:
		if obj.Name() == "SortText" {
			return "String"
		}
	}
	if m, ok := pkgModule[obj.Pkg().Path()]; ok {
		return m + "::" + obj.Name()
	}
	if obj.Pkg().Path() == curPkgPath {
		return obj.Name()
	}
	fail("type %s.%s", obj.Pkg().Path(), obj.Name())
	return ""
}

var curPkgPath string

// rtype maps a Go type to its Rust type. param: function parameter position (string -> &str, slices -> &[_]).
func rtype(t types.Type, param bool) string {
	if isAny(t) {
		return "Any"
	}
	if param {
		if isString(t) {
			return "&str"
		}
		if s, ok := isSlice(t); ok {
			if isString(s.Elem()) {
				return "&[&str]"
			}
			return "&[" + elemType(s.Elem()) + "]"
		}
		if isFourslashTestPtr(t) {
			return "&mut fourslash::FourslashTest"
		}
	}
	switch u := types.Unalias(t).(type) {
	case *types.Basic:
		switch u.Kind() {
		case types.Bool, types.UntypedBool:
			return "bool"
		case types.String, types.UntypedString:
			return "String"
		case types.Int, types.Int32, types.UntypedInt, types.UntypedRune:
			return "i32"
		case types.Uint32:
			return "u32"
		case types.Int64:
			return "i64"
		case types.Uint64:
			return "u64"
		case types.Uint8:
			return "u8"
		case types.Float64, types.UntypedFloat:
			return "f64"
		}
		fail("basic type %s", u)
	case *types.Named:
		if isNamed(u, cmPath, "Spawner") {
			return "Option<contentmapper::Spawner>"
		}
		args := u.TypeArgs()
		p := namedPath(u)
		if args.Len() > 0 {
			var as []string
			for i := 0; i < args.Len(); i++ {
				as = append(as, elemType(args.At(i)))
			}
			p += "<" + strings.Join(as, ", ") + ">"
		}
		return p
	case *types.Pointer:
		if harnessObject(u.Elem()) {
			return "Arc<" + rtype(u.Elem(), false) + ">"
		}
		if isTestingTPtr(u) {
			return "&T"
		}
		if isFourslashTestPtr(u) {
			return "fourslash::FourslashTest"
		}
		return "Option<" + rtype(u.Elem(), false) + ">"
	case *types.Slice:
		return "Vec<" + elemType(u.Elem()) + ">"
	case *types.Map:
		return "OrderedMap<" + rtype(u.Key(), false) + ", " + rtype(u.Elem(), false) + ">"
	case *types.Struct:
		if u.NumFields() == 0 {
			return "()"
		}
		return anonStruct(u)
	case *types.Interface:
		if isNamed(t, fsPath, "MarkerOrRange") {
			return "fourslash::MarkerOrRange"
		}
	}
	fail("type %s", types.TypeString(t, nil))
	return ""
}

// elemType: the Rust type of a slice element / type argument (`[]*T` -> `Vec<T>` per the lsproto mapping).
func elemType(t types.Type) string {
	if p, ok := types.Unalias(t).(*types.Pointer); ok && !harnessObject(p.Elem()) && !isFourslashTestPtr(p) && !isTestingTPtr(p) {
		return rtype(p.Elem(), false)
	}
	return rtype(t, false)
}

// isOptionPtr: *T represented as Option<T>.
func isOptionPtr(t types.Type) bool {
	p, ok := types.Unalias(t).(*types.Pointer)
	return ok && !harnessObject(p.Elem()) && !isFourslashTestPtr(p) && !isTestingTPtr(p)
}

func isArcPtr(t types.Type) bool {
	p, ok := types.Unalias(t).(*types.Pointer)
	return ok && harnessObject(p.Elem())
}

// Anonymous struct types of the current function get named Rust structs, declared at the top of the function.
var anonStructs = map[string]string{}
var anonStructDefs []string

func anonStruct(st *types.Struct) string {
	key := types.TypeString(st, nil)
	if name, ok := anonStructs[key]; ok {
		return name
	}
	name := sprintf("Anon%d", len(anonStructs)+1)
	anonStructs[key] = name
	def := "#[derive(Clone, Debug, Default)]\nstruct " + name + " {\n"
	for i := 0; i < st.NumFields(); i++ {
		f := st.Field(i)
		def += "    pub " + ident(f.Name()) + ": " + rtype(f.Type(), false) + ",\n"
	}
	def += "}\n"
	anonStructDefs = append(anonStructDefs, def)
	return name
}

func resetAnonStructs() {
	anonStructs = map[string]string{}
	anonStructDefs = nil
}
