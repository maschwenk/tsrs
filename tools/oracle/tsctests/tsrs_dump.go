package tsctests

// tsrs oracle (tools/oracle/tsctests): with TSRS_DUMP=<dir>, every tsc/tsbuild scenario run by the Go tests also
// writes <dir>/<subfolder>/<scenario>/<subScenario>.json: the inputs (command lines, cwd, env, the initial file
// system after newTestSys) and, for every edit, the file system changes it made (ops) in the incremental run and,
// applied from scratch, in the non-incremental replay (freshOps). The Rust harness replays these instead of the Go
// closures. Copy of tools/oracle/tsctests/tsrs_dump.go; apply with tools/oracle/tsctests/dump.sh.

import (
	"encoding/json"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

type tsrsFsEntry struct {
	content string
	symlink string
	mtime   time.Time
}

type tsrsOp struct {
	Op      string `json:"op"`
	Path    string `json:"path"`
	Content string `json:"content,omitempty"`
	Target  string `json:"target,omitempty"`
}

type tsrsEdit struct {
	Caption      string     `json:"caption"`
	Args         []string   `json:"args"`
	ExpectedDiff string     `json:"expectedDiff,omitempty"`
	Ops          []tsrsOp   `json:"ops"`
	FreshOps     []tsrsOp   `json:"freshOps"`
}

type tsrsFile struct {
	Path    string `json:"path"`
	Content string `json:"content,omitempty"`
	Symlink string `json:"symlink,omitempty"`
}

type tsrsScenario struct {
	Baseline         string            `json:"baseline"`
	Args             []string          `json:"args"`
	Cwd              string            `json:"cwd"`
	Env              map[string]string `json:"env,omitempty"`
	OutputIsTTY      *bool             `json:"outputIsTTY,omitempty"`
	IgnoreCase       bool              `json:"ignoreCase,omitempty"`
	WindowsStyleRoot string            `json:"windowsStyleRoot,omitempty"`
	Files            []tsrsFile        `json:"files"`
	DefaultLibs      []string          `json:"defaultLibs"`
	Edits            []*tsrsEdit       `json:"edits"`
	mu               sync.Mutex
}

func tsrsDumpDir() string { return os.Getenv("TSRS_DUMP") }

func tsrsSnap(sys *TestSys) map[string]tsrsFsEntry {
	snap := map[string]tsrsFsEntry{}
	for path, file := range sys.mapFs().Entries() {
		if file.Mode&fs.ModeSymlink != 0 {
			target, _ := sys.mapFs().GetTargetOfSymlink(path)
			snap[path] = tsrsFsEntry{symlink: target}
		} else if file.Mode.IsRegular() {
			snap[path] = tsrsFsEntry{content: string(file.Data), mtime: file.ModTime}
		}
	}
	return snap
}

func tsrsDiff(before, after map[string]tsrsFsEntry) []tsrsOp {
	var ops []tsrsOp
	keys := make([]string, 0, len(after))
	for k := range after {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	for _, k := range keys {
		a := after[k]
		b, ok := before[k]
		switch {
		case a.symlink != "":
			if !ok || b.symlink != a.symlink {
				ops = append(ops, tsrsOp{Op: "symlink", Path: k, Target: a.symlink})
			}
		case !ok || b.symlink != "" || b.content != a.content:
			ops = append(ops, tsrsOp{Op: "write", Path: k, Content: a.content})
		case !b.mtime.Equal(a.mtime):
			ops = append(ops, tsrsOp{Op: "touch", Path: k})
		}
	}
	var deleted []string
	for k := range before {
		if _, ok := after[k]; !ok {
			deleted = append(deleted, k)
		}
	}
	slices.Sort(deleted)
	for _, k := range deleted {
		ops = append(ops, tsrsOp{Op: "delete", Path: k})
	}
	return ops
}

func tsrsNewScenario(test *tscInput, scenario string, sys *TestSys) *tsrsScenario {
	if tsrsDumpDir() == "" {
		return nil
	}
	s := &tsrsScenario{
		Baseline:         filepath.Join(test.getBaselineSubFolder(), scenario, strings.ReplaceAll(test.subScenario, " ", "-")+".js"),
		Args:             test.commandLineArgs,
		Cwd:              sys.GetCurrentDirectory(),
		Env:              test.env,
		OutputIsTTY:      test.outputIsTTY,
		IgnoreCase:       test.ignoreCase,
		WindowsStyleRoot: test.windowsStyleRoot,
		Edits:            make([]*tsrsEdit, len(test.edits)),
	}
	snap := tsrsSnap(sys)
	keys := make([]string, 0, len(snap))
	for k := range snap {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	for _, k := range keys {
		e := snap[k]
		s.Files = append(s.Files, tsrsFile{Path: k, Content: e.content, Symlink: e.symlink})
	}
	if sys.fs.defaultLibs != nil {
		sys.fs.defaultLibs.Range(func(p string) bool {
			s.DefaultLibs = append(s.DefaultLibs, p)
			return true
		})
	}
	slices.Sort(s.DefaultLibs)
	for i, e := range test.edits {
		s.Edits[i] = &tsrsEdit{Caption: e.caption, Args: e.commandLineArgs, ExpectedDiff: e.expectedDiff}
	}
	return s
}

func (s *tsrsScenario) edit(index int, sys *TestSys, do func()) {
	if s == nil {
		do()
		return
	}
	before := tsrsSnap(sys)
	do()
	ops := tsrsDiff(before, tsrsSnap(sys))
	s.mu.Lock()
	s.Edits[index].Ops = ops
	s.mu.Unlock()
}

func (s *tsrsScenario) freshEdits(index int, sys *TestSys, do func()) {
	if s == nil {
		do()
		return
	}
	before := tsrsSnap(sys)
	do()
	ops := tsrsDiff(before, tsrsSnap(sys))
	s.mu.Lock()
	s.Edits[index].FreshOps = ops
	s.mu.Unlock()
}

func (s *tsrsScenario) write() {
	if s == nil {
		return
	}
	path := filepath.Join(tsrsDumpDir(), strings.TrimSuffix(s.Baseline, ".js")+".json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		panic(err)
	}
	data, err := json.MarshalIndent(s, "", " ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(path, data, 0o644); err != nil {
		panic(err)
	}
}
