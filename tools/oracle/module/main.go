// tsrs-oracle-module: ground truth for tsrs_module's resolver.
//
// Reads JSON-lines scenarios on stdin (files, a fixed subset of compiler options, resolution
// requests) and writes one JSON line per scenario with each request's result and traces.
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"
	"testing/fstest"

	"github.com/microsoft/TypeScript/tsc/internal/collections"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/module"
	"github.com/microsoft/TypeScript/tsc/internal/vfs"
	"github.com/microsoft/TypeScript/tsc/internal/vfs/vfstest"
)

type scenario struct {
	Name          string                     `json:"name"`
	Cwd           string                     `json:"cwd"`
	CaseSensitive bool                       `json:"caseSensitive"`
	Files         map[string]json.RawMessage `json:"files"`
	Options       options                    `json:"options"`
	Requests      []request                  `json:"requests"`
}

type options struct {
	Module                     int                 `json:"module"`
	ModuleResolution           int                 `json:"moduleResolution"`
	Jsx                        int                 `json:"jsx"`
	ResolveJsonModule          *bool               `json:"resolveJsonModule"`
	AllowJs                    *bool               `json:"allowJs"`
	NoDtsResolution            *bool               `json:"noDtsResolution"`
	PreserveSymlinks           *bool               `json:"preserveSymlinks"`
	ResolvePackageJsonExports  *bool               `json:"resolvePackageJsonExports"`
	ResolvePackageJsonImports  *bool               `json:"resolvePackageJsonImports"`
	AllowArbitraryExtensions   *bool               `json:"allowArbitraryExtensions"`
	CustomConditions           []string            `json:"customConditions"`
	ModuleSuffixes             []string            `json:"moduleSuffixes"`
	RootDirs                   []string            `json:"rootDirs"`
	TypeRoots                  []string            `json:"typeRoots"`
	Types                      []string            `json:"types"`
	Paths                      [][2]json.RawMessage `json:"paths"`
	PathsBasePath              string              `json:"pathsBasePath"`
	OutDir                     string              `json:"outDir"`
	DeclarationDir             string              `json:"declarationDir"`
	RootDir                    string              `json:"rootDir"`
	ConfigFilePath             string              `json:"configFilePath"`
}

type request struct {
	Kind string `json:"kind"`
	Name string `json:"name"`
	File string `json:"file"`
	Mode int    `json:"mode"`
}

type result struct {
	Kind                         string   `json:"kind"`
	Name                         string   `json:"name"`
	File                         string   `json:"file"`
	Mode                         int      `json:"mode"`
	ResolvedFileName             string   `json:"resolvedFileName"`
	OriginalPath                 string   `json:"originalPath"`
	Extension                    string   `json:"extension"`
	PackageId                    string   `json:"packageId"`
	IsExternalLibraryImport      bool     `json:"isExternalLibraryImport"`
	ResolvedUsingTsExtension     bool     `json:"resolvedUsingTsExtension"`
	ResolvedUsingExtraExtensions bool     `json:"resolvedUsingExtraExtensions"`
	AlternateResult              string   `json:"alternateResult"`
	Primary                      bool     `json:"primary"`
	Diagnostics                  []string `json:"diagnostics"`
	Traces                       []string `json:"traces"`
}

type output struct {
	Name                string   `json:"name"`
	Results             []result `json:"results"`
	AutomaticTypes      []string `json:"automaticTypes"`
	Error               string   `json:"error,omitempty"`
}

type host struct {
	fs  vfs.FS
	cwd string
}

func (h *host) FS() vfs.FS                  { return h.fs }
func (h *host) GetCurrentDirectory() string { return h.cwd }

func tristate(b *bool) core.Tristate {
	if b == nil {
		return core.TSUnknown
	}
	if *b {
		return core.TSTrue
	}
	return core.TSFalse
}

func formatTrace(d module.DiagAndArgs) string {
	parts := []string{string(d.Message.Key())}
	for _, a := range d.Args {
		parts = append(parts, fmt.Sprint(a))
	}
	return strings.Join(parts, "|")
}

func run(s scenario) (out output) {
	out.Name = s.Name
	defer func() {
		if r := recover(); r != nil {
			out.Error = fmt.Sprint(r)
		}
	}()
	files := map[string]*fstest.MapFile{}
	for path, raw := range s.Files {
		var text string
		if err := json.Unmarshal(raw, &text); err == nil {
			files[path] = &fstest.MapFile{Data: []byte(text)}
			continue
		}
		var link struct {
			Symlink string `json:"symlink"`
		}
		if err := json.Unmarshal(raw, &link); err != nil {
			panic(err)
		}
		files[path] = vfstest.Symlink(link.Symlink)
	}
	h := &host{fs: vfstest.FromMap(files, s.CaseSensitive), cwd: s.Cwd}
	o := s.Options
	opts := &core.CompilerOptions{
		Module:                    core.ModuleKind(o.Module),
		ModuleResolution:          core.ModuleResolutionKind(o.ModuleResolution),
		Jsx:                       core.JsxEmit(o.Jsx),
		ResolveJsonModule:         tristate(o.ResolveJsonModule),
		AllowJs:                   tristate(o.AllowJs),
		NoDtsResolution:           tristate(o.NoDtsResolution),
		PreserveSymlinks:          tristate(o.PreserveSymlinks),
		ResolvePackageJsonExports: tristate(o.ResolvePackageJsonExports),
		ResolvePackageJsonImports: tristate(o.ResolvePackageJsonImports),
		AllowArbitraryExtensions:  tristate(o.AllowArbitraryExtensions),
		CustomConditions:          o.CustomConditions,
		ModuleSuffixes:            o.ModuleSuffixes,
		RootDirs:                  o.RootDirs,
		TypeRoots:                 o.TypeRoots,
		Types:                     o.Types,
		PathsBasePath:             o.PathsBasePath,
		OutDir:                    o.OutDir,
		DeclarationDir:            o.DeclarationDir,
		RootDir:                   o.RootDir,
		ConfigFilePath:            o.ConfigFilePath,
		TraceResolution:           core.TSTrue,
	}
	if o.Paths != nil {
		paths := collections.NewOrderedMapWithSizeHint[string, []string](len(o.Paths))
		for _, entry := range o.Paths {
			var key string
			var values []string
			if err := json.Unmarshal(entry[0], &key); err != nil {
				panic(err)
			}
			if err := json.Unmarshal(entry[1], &values); err != nil {
				panic(err)
			}
			paths.Set(key, values)
		}
		opts.Paths = paths
	}
	resolver := module.NewResolver(module.ResolverOptions{Host: h, CompilerOptions: opts})
	for _, req := range s.Requests {
		r := result{Kind: req.Kind, Name: req.Name, File: req.File, Mode: req.Mode, Diagnostics: []string{}, Traces: []string{}}
		var traces []module.DiagAndArgs
		if req.Kind == "module" {
			m, t, _ := resolver.ResolveModuleName(req.Name, req.File, core.ResolutionMode(req.Mode), nil)
			traces = t
			r.ResolvedFileName = m.ResolvedFileName
			r.OriginalPath = m.OriginalPath
			r.Extension = m.Extension
			if m.PackageId.Name != "" {
				r.PackageId = m.PackageId.String()
			}
			r.IsExternalLibraryImport = m.IsExternalLibraryImport
			r.ResolvedUsingTsExtension = m.ResolvedUsingTsExtension
			r.ResolvedUsingExtraExtensions = m.ResolvedUsingExtraExtensions
			r.AlternateResult = m.AlternateResult
			for _, d := range m.ResolutionDiagnostics {
				r.Diagnostics = append(r.Diagnostics, fmt.Sprintf("%d:%s", d.Code(), d.String()))
			}
		} else {
			m, t := resolver.ResolveTypeReferenceDirective(req.Name, req.File, core.ResolutionMode(req.Mode), nil)
			traces = t
			r.ResolvedFileName = m.ResolvedFileName
			r.OriginalPath = m.OriginalPath
			if m.PackageId.Name != "" {
				r.PackageId = m.PackageId.String()
			}
			r.IsExternalLibraryImport = m.IsExternalLibraryImport
			r.Primary = m.Primary
			for _, d := range m.ResolutionDiagnostics {
				r.Diagnostics = append(r.Diagnostics, fmt.Sprintf("%d:%s", d.Code(), d.String()))
			}
		}
		for _, t := range traces {
			r.Traces = append(r.Traces, formatTrace(t))
		}
		out.Results = append(out.Results, r)
	}
	out.AutomaticTypes = module.GetAutomaticTypeDirectiveNames(opts, h)
	sort.Strings(out.AutomaticTypes)
	return out
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 64<<20), 64<<20)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for scanner.Scan() {
		var s scenario
		if err := json.Unmarshal(scanner.Bytes(), &s); err != nil {
			panic(err)
		}
		b, err := json.Marshal(run(s))
		if err != nil {
			panic(err)
		}
		w.Write(b)
		w.WriteByte('\n')
	}
}
