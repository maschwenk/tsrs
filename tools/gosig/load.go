package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/importer"
	"go/parser"
	"go/token"
	"go/types"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type listedPkg struct {
	ImportPath string
	Name       string
	Dir        string
	GoFiles    []string
	Export     string
	Standard   bool
	Module     *struct{ Path string }
}

// Package is a source-type-checked package with full type information.
type Package struct {
	Path  string
	Types *types.Package
	Files []*ast.File
	Names []string // base file names, parallel to Files
	Info  *types.Info
}

type Loaded struct {
	Fset   *token.FileSet
	Target *Package
	Extra  []*Package
}

// load type-checks the target package (and the result-only packages) from source.
// All packages of the same module are checked from source so that type identities
// agree; everything else (std, third party) comes from `go list -export` data.
func load(moduleDir, target string, extra []string) (*Loaded, error) {
	args := append([]string{"list", "-e", "-export", "-deps", "-json", target}, extra...)
	cmd := exec.Command("go", args...)
	cmd.Dir = moduleDir
	cmd.Env = append(os.Environ(), "GOTOOLCHAIN=auto")
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("go list: %v\n%s", err, stderr.String())
	}
	var pkgs []*listedPkg
	dec := json.NewDecoder(bytes.NewReader(out))
	for {
		var p listedPkg
		if err := dec.Decode(&p); err == io.EOF {
			break
		} else if err != nil {
			return nil, err
		}
		pkgs = append(pkgs, &p)
	}
	byPath := map[string]*listedPkg{}
	for _, p := range pkgs {
		byPath[p.ImportPath] = p
	}
	// Resolve relative patterns to import paths.
	resolve := func(pat string) string {
		if strings.HasPrefix(pat, ".") {
			dir := filepath.Clean(filepath.Join(moduleDir, pat))
			for _, p := range pkgs {
				if p.Dir == dir {
					return p.ImportPath
				}
			}
		}
		return pat
	}
	targetPath := resolve(target)
	tp := byPath[targetPath]
	if tp == nil || tp.Module == nil {
		return nil, fmt.Errorf("package %s not found", target)
	}
	modPath := tp.Module.Path
	wantInfo := map[string]bool{targetPath: true}
	var extraPaths []string
	for _, e := range extra {
		ep := resolve(e)
		wantInfo[ep] = true
		extraPaths = append(extraPaths, ep)
	}

	fset := token.NewFileSet()
	gc := importer.ForCompiler(fset, "gc", func(path string) (io.ReadCloser, error) {
		p := byPath[path]
		if p == nil || p.Export == "" {
			return nil, fmt.Errorf("no export data for %s", path)
		}
		return os.Open(p.Export)
	})
	src := map[string]*Package{}
	imp := importerFunc(func(path string) (*types.Package, error) {
		if p, ok := src[path]; ok {
			return p.Types, nil
		}
		return gc.Import(path)
	})

	// go list -deps prints dependencies before dependents.
	for _, p := range pkgs {
		if p.Standard || p.Module == nil || p.Module.Path != modPath {
			continue
		}
		pk := &Package{Path: p.ImportPath}
		for _, f := range p.GoFiles {
			af, err := parser.ParseFile(fset, filepath.Join(p.Dir, f), nil, parser.ParseComments|parser.SkipObjectResolution)
			if err != nil {
				return nil, err
			}
			pk.Files = append(pk.Files, af)
			pk.Names = append(pk.Names, f)
		}
		var info *types.Info
		if wantInfo[p.ImportPath] {
			info = &types.Info{
				Types:      map[ast.Expr]types.TypeAndValue{},
				Defs:       map[*ast.Ident]types.Object{},
				Uses:       map[*ast.Ident]types.Object{},
				Selections: map[*ast.SelectorExpr]*types.Selection{},
				Implicits:  map[ast.Node]types.Object{},
			}
		}
		var firstErr error
		conf := types.Config{
			Importer: imp,
			Sizes:    types.SizesFor("gc", "arm64"),
			Error: func(err error) {
				if firstErr == nil {
					firstErr = err
				}
			},
		}
		tpkg, _ := conf.Check(p.ImportPath, fset, pk.Files, info)
		if firstErr != nil {
			return nil, fmt.Errorf("type-checking %s: %v", p.ImportPath, firstErr)
		}
		pk.Types = tpkg
		pk.Info = info
		src[p.ImportPath] = pk
	}
	l := &Loaded{Fset: fset, Target: src[targetPath]}
	for _, e := range extraPaths {
		l.Extra = append(l.Extra, src[e])
	}
	return l, nil
}

type importerFunc func(path string) (*types.Package, error)

func (f importerFunc) Import(path string) (*types.Package, error) { return f(path) }
