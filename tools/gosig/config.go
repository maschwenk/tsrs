package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// Config drives one gosig run. Relative paths are resolved against the directory
// containing the config file.
type Config struct {
	// Go module root (directory with go.mod) and the package to generate stubs for.
	ModuleDir string `json:"moduleDir"`
	Package   string `json:"package"`
	// Additional in-module packages whose function bodies are analyzed for result
	// nil-ability only (e.g. ast accessors), so `return node.Name()` is judged correctly.
	ResultOnlyPackages []string `json:"resultOnlyPackages"`

	OutDir           string `json:"outDir"`
	SigsFile         string `json:"sigsFile"`
	AdvisorySigsFile string `json:"advisorySigsFile"`
	Prelude          string `json:"prelude"`

	// Emitted Go files, in output order. Several Go files may map to the same Rust file.
	Files []FileRule `json:"files"`
	// Go files that are not ported at all: their bodies are ignored by the analysis and
	// their functions do not reserve Rust names.
	NotPorted []string `json:"notPorted"`
	// Function declarations (Go names, "Recv.name" for methods) never emitted.
	SkipFuncs []string `json:"skipFuncs"`

	// The state-machine type (methods take &mut self, callbacks get it as first arg).
	CheckerType string `json:"checkerType"`
	// Function-typed fields of CheckerType become methods emitted into this file.
	FieldsFile string `json:"fieldsFile"`
	// Short-lived helper structs holding `c *Checker` -> impl header ("" = default). With
	// checkerHolderStyle "param" (docs/CHECKER.md) the field is dropped and every method takes
	// `c: &mut Checker` after the receiver (&self for arena types, else &mut self); with "lifetime"
	// they are `impl<'c> Name<'c>` holding `c: &'c mut Checker`.
	CheckerHolders     map[string]string `json:"checkerHolders"`
	CheckerHolderStyle string            `json:"checkerHolderStyle"`
	// Struct fields pointing back to the checker that Rust drops ("Type.checker"): functions that
	// use them (directly or through callees) get a leading `c: &mut Checker` parameter.
	DroppedCheckerFields []string `json:"droppedCheckerFields"`
	// Structs that hold *Checker in Go but where the pointer is dropped/passed in (not holders).
	NotCheckerHolders []string `json:"notCheckerHolders"`
	// Receiver types whose methods take &self regardless of mutation.
	ArenaTypes []string `json:"arenaTypes"`

	// Go type string (package-name qualified, target package unqualified, e.g. "*ast.Node",
	// "ast.SymbolTable", "*Type") -> Rust type. Checked before the built-in rules.
	TypeMap map[string]string `json:"typeMap"`
	// Pointers to structs implementing one of these interfaces map to Rust by template
	// ({T} = struct name): ast node data -> P<Node>, checker type data -> &'static {T}.
	DataInterfaces []DataInterface `json:"dataInterfaces"`
	// Per-parameter/result Rust types: "Recv.goName.paramName" (or ".r0") -> Rust type.
	ParamTypes map[string]string `json:"paramTypes"`
	// Generic type-parameter constraints -> Rust bounds ("" = no bound).
	ConstraintMap map[string]string `json:"constraintMap"`
	// Substrings replaced before snake_casing ("JSDoc" -> "Jsdoc").
	WordReplacements map[string]string `json:"wordReplacements"`
	// How parameter nil-ability propagates through forwarded arguments:
	// "forward" (default): a nil literal / nil-able parameter flowing unguarded through
	// variables into an argument makes the callee parameter Option;
	// "reverse": the spec'd rule — a parameter forwarded to an Option parameter becomes Option;
	// "both"; "off".
	ParamForwarding string `json:"paramForwarding"`
	// Parameter types for which Go code relies on nil reading as empty (ast.SymbolTable): such
	// parameters are Option unless the function writes through them.
	NilTolerantParams []string `json:"nilTolerantParams"`
	// Treat nil-in/nil-out parameters (`if p == nil { return nil }` early exits) as non-nil.
	NilTransparent bool `json:"nilTransparent"`
	// Functions (Go names) that return nil only when their callback argument returns nil
	// (mapType). A call site is judged by the callback actually passed.
	CallbackConditionalNil []string `json:"callbackConditionalNil"`
	// Wrappers returning their func argument unchanged in behavior (core.Memoize).
	FuncPassthrough []string `json:"funcPassthrough"`
	// Generic functions (pkg.Name) returning their first non-zero argument (nil only if all are).
	CoalesceGenerics []string `json:"coalesceGenerics"`
	// Manual corrections, applied before the fixpoint: "Recv.goName.p<i>" or ".r<i>" -> "P" or
	// "Option"; "Recv.goName#localVar" -> "nonnil" (a local whose zero value never escapes).
	Overrides map[string]string `json:"overrides"`
	// Generic functions (pkg.Name) that return the zero value of T on a miss.
	ZeroValueGenerics []string `json:"zeroValueGenerics"`
}

type DataInterface struct {
	Iface  string   `json:"iface"` // "ast.nodeData", or "TypeData" for the target package
	Rust   string   `json:"rust"`
	Except []string `json:"except"`
}

type FileRule struct {
	Go        string  `json:"go"`
	Rust      string  `json:"rust"`
	Chunks    []Chunk `json:"chunks"`
	DeclsNote string  `json:"declsNote"`
}

type Chunk struct {
	Rust string `json:"rust"`
	From int    `json:"from"`
	To   int    `json:"to"`
}

func loadConfig(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var cfg Config
	if err := json.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	base := filepath.Dir(path)
	abs := func(p string) string {
		if p == "" || filepath.IsAbs(p) {
			return p
		}
		return filepath.Clean(filepath.Join(base, p))
	}
	cfg.ModuleDir = abs(cfg.ModuleDir)
	cfg.OutDir = abs(cfg.OutDir)
	cfg.SigsFile = abs(cfg.SigsFile)
	cfg.AdvisorySigsFile = abs(cfg.AdvisorySigsFile)
	if cfg.CheckerType == "" {
		cfg.CheckerType = "Checker"
	}
	return &cfg, nil
}

// rustFileFor returns the Rust output file for a declaration at goFile:line, or "".
func (cfg *Config) rustFileFor(goFile string, line int) (string, *FileRule) {
	for i := range cfg.Files {
		r := &cfg.Files[i]
		if r.Go != goFile {
			continue
		}
		if len(r.Chunks) == 0 {
			return r.Rust, r
		}
		for _, c := range r.Chunks {
			if line >= c.From && line <= c.To {
				return c.Rust, r
			}
		}
		return "", r
	}
	return "", nil
}

func contains(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}
