// tsrs-oracle-modulespecifiers: ground truth for tsrs_modulespecifiers (and the compiler's
// ModuleSpecifierGenerationHost / GetSymlinkCache).
//
// Reads JSON-lines scenarios on stdin. Each scenario is an in-memory project (files, symlinks, a
// tsconfig path) plus requests (importing file, target file, ending preference, override mode). For
// each scenario it builds a Program exactly like `tsc -p` and writes one JSON line with the sorted
// symlink cache (realpath directory -> symlink directories) and, per request, the result of
// modulespecifiers.GetModuleSpecifiersForFileWithInfo with the node builder's preferences
// (ImportModuleSpecifierPreferenceProjectRelative).
//
// Build (from ts-ref/tsc, source copied to cmd/tsrs-oracle-modulespecifiers/main.go):
//   GOTOOLCHAIN=auto go build -o ../../bin/tsrs-oracle-modulespecifiers ./cmd/tsrs-oracle-modulespecifiers
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"sort"

	"github.com/microsoft/TypeScript/tsc/internal/bundled"
	"github.com/microsoft/TypeScript/tsc/internal/collections"
	"github.com/microsoft/TypeScript/tsc/internal/compiler"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/modulespecifiers"
	"github.com/microsoft/TypeScript/tsc/internal/tsoptions"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
	"github.com/microsoft/TypeScript/tsc/internal/vfs/vfstest"
)

type scenario struct {
	Name          string            `json:"name"`
	Cwd           string            `json:"cwd"`
	Config        string            `json:"config"`
	CaseSensitive bool              `json:"caseSensitive"`
	Files         map[string]string `json:"files"`
	Symlinks      map[string]string `json:"symlinks"`
	Requests      []request         `json:"requests"`
}

type request struct {
	From   string `json:"from"`
	To     string `json:"to"`
	Ending string `json:"ending"` // "" or "js"
	Mode   int    `json:"mode"`   // OverrideImportMode
	Pref   string `json:"pref"`   // ImportModuleSpecifierPreference; "" means the node builder's "project-relative"
}

type result struct {
	From       string   `json:"from"`
	To         string   `json:"to"`
	Ending     string   `json:"ending"`
	Mode       int      `json:"mode"`
	Pref       string   `json:"pref"`
	Specifiers []string `json:"specifiers"`
	Kind       int      `json:"kind"`
	Error      string   `json:"error,omitempty"`
}

type output struct {
	Name     string   `json:"name"`
	Symlinks []string `json:"symlinks"`
	Results  []result `json:"results"`
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 64<<20), 64<<20)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var s scenario
		if err := json.Unmarshal(line, &s); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		b, _ := json.Marshal(run(&s))
		out.Write(b)
		out.WriteString("\n")
	}
}

func run(s *scenario) output {
	m := map[string]any{}
	for k, v := range s.Files {
		m[k] = v
	}
	for k, v := range s.Symlinks {
		m[k] = vfstest.Symlink(v)
	}
	fs := bundled.WrapFS(vfstest.FromMap(m, s.CaseSensitive))
	host := compiler.NewCompilerHost(s.Cwd, fs, bundled.LibPath(), nil, nil, nil)
	config, errs := tsoptions.GetParsedCommandLineOfConfigFile(s.Config, &core.CompilerOptions{}, nil, host, nil)
	o := output{Name: s.Name, Symlinks: []string{}, Results: []result{}}
	if len(errs) > 0 || config == nil {
		o.Symlinks = append(o.Symlinks, "config error")
		return o
	}
	program := compiler.NewProgram(compiler.ProgramOptions{
		ProgramConfig: compiler.ProgramConfig{Config: config, SingleThreaded: core.TSTrue},
		ProgramHosts:  compiler.ProgramHosts{Host: host},
	})

	o.Symlinks = dumpSymlinks(program)

	for _, r := range s.Requests {
		res := result{From: r.From, To: r.To, Ending: r.Ending, Mode: r.Mode, Pref: r.Pref, Specifiers: []string{}}
		file := program.GetSourceFile(r.From)
		if file == nil {
			res.Error = "no importing file"
			o.Results = append(o.Results, res)
			continue
		}
		prefs := modulespecifiers.UserPreferences{ImportModuleSpecifierPreference: modulespecifiers.ImportModuleSpecifierPreferenceProjectRelative}
		if r.Pref != "" {
			prefs.ImportModuleSpecifierPreference = modulespecifiers.ImportModuleSpecifierPreference(r.Pref)
		}
		if r.Ending == "js" {
			prefs.ImportModuleSpecifierEnding = modulespecifiers.ImportModuleSpecifierEndingPreferenceJs
		}
		specifiers, kind := modulespecifiers.GetModuleSpecifiersForFileWithInfo(
			file,
			r.To,
			program.Options(),
			program,
			prefs,
			modulespecifiers.ModuleSpecifierOptions{OverrideImportMode: core.ResolutionMode(r.Mode)},
			false,
		)
		if specifiers != nil {
			res.Specifiers = specifiers
		}
		res.Kind = int(kind)
		o.Results = append(o.Results, res)
	}
	return o
}

func dumpSymlinks(program *compiler.Program) []string {
	var entries []string
	cache := program.GetSymlinkCache()
	cache.DirectoriesByRealpath().Range(func(realpath tspath.Path, set *collections.SyncSet[string]) bool {
		for _, k := range set.ToSlice() {
			entries = append(entries, string(realpath)+" <- "+k)
		}
		return true
	})
	sort.Strings(entries)
	if entries == nil {
		entries = []string{}
	}
	return entries
}
