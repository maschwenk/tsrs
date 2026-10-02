package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/build"
	"go/importer"
	"go/parser"
	"go/token"
	"go/types"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

const modPath = "github.com/microsoft/TypeScript/tsc"

type listedPkg struct {
	ImportPath string
	Dir        string
	Export     string
}

// Package is a source-type-checked package.
type Package struct {
	Path  string
	Name  string
	Types *types.Package
	Files []*ast.File
	Names []string // base file names, parallel to Files
	Info  *types.Info
}

type Loaded struct {
	Fset  *token.FileSet
	Util  *Package
	Tests []*Package // the fourslash_test package and the in-package test package
}

func newInfo() *types.Info {
	return &types.Info{
		Types:      map[ast.Expr]types.TypeAndValue{},
		Defs:       map[*ast.Ident]types.Object{},
		Uses:       map[*ast.Ident]types.Object{},
		Selections: map[*ast.SelectorExpr]*types.Selection{},
		Implicits:  map[ast.Node]types.Object{},
		Scopes:     map[ast.Node]*types.Scope{},
		Instances:  map[*ast.Ident]types.Instance{},
	}
}

// load type-checks tests/util and every *_test.go file in the fourslash tests directory from source; all other
// packages come from `go list -export` data.
func load(tscDir string) (*Loaded, error) {
	testsDir := filepath.Join(tscDir, "internal", "fourslash", "tests")
	utilDir := filepath.Join(testsDir, "util")
	fset := token.NewFileSet()

	entries, err := os.ReadDir(testsDir)
	if err != nil {
		return nil, err
	}
	var testNames []string
	for _, e := range entries {
		if !e.IsDir() && strings.HasSuffix(e.Name(), "_test.go") {
			// Files `go test` does not build (file name GOOS/GOARCH suffixes such as `_js_test.go`, build tags)
			// hold tests that never run in Go.
			if ok, err := build.Default.MatchFile(testsDir, e.Name()); err != nil || !ok {
				continue
			}
			testNames = append(testNames, e.Name())
		}
	}
	sort.Strings(testNames)

	byPkg := map[string]*Package{}
	imports := map[string]bool{}
	for _, name := range testNames {
		af, err := parser.ParseFile(fset, filepath.Join(testsDir, name), nil, parser.ParseComments|parser.SkipObjectResolution)
		if err != nil {
			return nil, err
		}
		pn := af.Name.Name
		p := byPkg[pn]
		if p == nil {
			p = &Package{Name: pn, Path: modPath + "/internal/fourslash/tests"}
			if strings.HasSuffix(pn, "_test") {
				p.Path += "_test"
			}
			byPkg[pn] = p
		}
		p.Files = append(p.Files, af)
		p.Names = append(p.Names, name)
		for _, im := range af.Imports {
			imports[strings.Trim(im.Path.Value, `"`)] = true
		}
	}
	utilPkg := &Package{Path: modPath + "/internal/fourslash/tests/util"}
	{
		af, err := parser.ParseFile(fset, filepath.Join(utilDir, "util.go"), nil, parser.ParseComments|parser.SkipObjectResolution)
		if err != nil {
			return nil, err
		}
		utilPkg.Files = []*ast.File{af}
		utilPkg.Names = []string{"util.go"}
		utilPkg.Name = af.Name.Name
		for _, im := range af.Imports {
			imports[strings.Trim(im.Path.Value, `"`)] = true
		}
	}

	args := []string{"list", "-e", "-export", "-deps", "-json=ImportPath,Dir,Export"}
	var importList []string
	for im := range imports {
		if im != utilPkg.Path {
			importList = append(importList, im)
		}
	}
	sort.Strings(importList)
	args = append(args, importList...)
	cmd := exec.Command("go", args...)
	cmd.Dir = tscDir
	cmd.Env = append(os.Environ(), "GOTOOLCHAIN=auto")
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("go list: %v\n%s", err, stderr.String())
	}
	exports := map[string]string{}
	dec := json.NewDecoder(bytes.NewReader(out))
	for {
		var p listedPkg
		if err := dec.Decode(&p); err == io.EOF {
			break
		} else if err != nil {
			return nil, err
		}
		exports[p.ImportPath] = p.Export
	}

	gc := importer.ForCompiler(fset, "gc", func(path string) (io.ReadCloser, error) {
		e := exports[path]
		if e == "" {
			return nil, fmt.Errorf("no export data for %s", path)
		}
		return os.Open(e)
	})
	src := map[string]*types.Package{}
	imp := importerFunc(func(path string) (*types.Package, error) {
		if p, ok := src[path]; ok {
			return p, nil
		}
		return gc.Import(path)
	})
	check := func(p *Package) error {
		p.Info = newInfo()
		var errs []string
		conf := types.Config{
			Importer: imp,
			Sizes:    types.SizesFor("gc", "arm64"),
			Error: func(err error) {
				if len(errs) < 10 {
					errs = append(errs, err.Error())
				}
			},
		}
		tpkg, _ := conf.Check(p.Path, fset, p.Files, p.Info)
		if len(errs) > 0 {
			return fmt.Errorf("type-checking %s:\n%s", p.Path, strings.Join(errs, "\n"))
		}
		p.Types = tpkg
		return nil
	}
	if err := check(utilPkg); err != nil {
		return nil, err
	}
	src[utilPkg.Path] = utilPkg.Types
	l := &Loaded{Fset: fset, Util: utilPkg}
	var pkgNames []string
	for n := range byPkg {
		pkgNames = append(pkgNames, n)
	}
	sort.Strings(pkgNames)
	for _, n := range pkgNames {
		if err := check(byPkg[n]); err != nil {
			return nil, err
		}
		l.Tests = append(l.Tests, byPkg[n])
	}
	return l, nil
}

type importerFunc func(path string) (*types.Package, error)

func (f importerFunc) Import(path string) (*types.Package, error) { return f(path) }
