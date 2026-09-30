// Command gosig generates Rust signature stubs for a Go package, with whole-package
// nil-ability analysis deciding P<T> vs Option<P<T>>. See README.md.
package main

import (
	"flag"
	"fmt"
	"go/types"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

func main() {
	cfgPath := flag.String("config", "tools/gosig/checker.json", "config file")
	whyPath := flag.String("why", "", "write the reason for every Option decision to this file")
	dry := flag.Bool("dry", false, "analyze and report, but do not write any files")
	forwarding := flag.String("forwarding", "", "override the config's paramForwarding mode")
	outDir := flag.String("out", "", "override the config's outDir (and write sigs files there too)")
	transparent := flag.String("transparent", "", "override the config's nilTransparent (true/false)")
	traceNames := flag.String("trace", "", "comma-separated Go function names: explain their Option results")
	roots := flag.Bool("roots", false, "print a histogram of root causes of Option results")
	hot := flag.Bool("hot", false, "list the most-called functions with Option parameters/results")
	flowRoots := flag.Bool("flowroots", false, "print a histogram of nil origins for forward-flow Option params")
	flowTrace := flag.String("flowtrace", "", "Go function name: print the nil-flow hops into its first flow-marked parameter")
	flag.Parse()

	cfg, err := loadConfig(*cfgPath)
	if err != nil {
		fatal(err)
	}
	if *forwarding != "" {
		cfg.ParamForwarding = *forwarding
	}
	if *outDir != "" {
		cfg.OutDir = *outDir
		cfg.SigsFile = filepath.Join(*outDir, "sigs", filepath.Base(cfg.SigsFile))
		cfg.AdvisorySigsFile = filepath.Join(*outDir, "sigs", filepath.Base(cfg.AdvisorySigsFile))
		if err := os.MkdirAll(*outDir, 0o755); err != nil {
			fatal(err)
		}
	}
	if *transparent != "" {
		cfg.NilTransparent = *transparent == "true"
	}
	l, err := load(cfg.ModuleDir, cfg.Package, cfg.ResultOnlyPackages)
	if err != nil {
		fatal(err)
	}
	checker, _ := l.Target.Types.Scope().Lookup(cfg.CheckerType).(*types.TypeName)
	tm := &TypeMapper{cfg: cfg, main: l.Target.Types, checker: checker, unmapped: map[string]int{}}
	for _, n := range tm.resolveIfaces() {
		fmt.Fprintf(os.Stderr, "warning: data interface %s not found\n", n)
	}
	for _, n := range tm.unlistedHolders() {
		fmt.Fprintf(os.Stderr, "warning: %s holds *%s but is not in checkerHolders/notCheckerHolders/arenaTypes\n", n, cfg.CheckerType)
	}
	a := newAnalyzer(l.Fset, cfg, l.Target.Types, tm.nilable)
	for _, p := range l.Extra {
		a.collect(p, true)
	}
	a.collect(l.Target, false)
	g := &Gen{cfg: cfg, l: l, a: a, tm: tm, namer: newNamer(cfg.WordReplacements)}
	for _, p := range l.Extra {
		a.walkPackage(p, true, nil)
	}
	a.walkPackage(l.Target, false, g.notPorted)
	g.computeMutation()
	g.computeNeedsChecker()
	a.readTolerant = func(v *types.Var) bool { return !g.mutParam[v] }
	for _, k := range a.applyOverrides() {
		fmt.Fprintf(os.Stderr, "warning: override %q matches nothing\n", k)
	}
	a.finish()

	if *traceNames != "" {
		for _, name := range strings.Split(*traceNames, ",") {
			for _, fn := range a.order {
				if fn.Name == name && len(fn.Slots.Results) > 0 {
					var out []string
					a.trace(fn.Slots.Results[0], 0, map[*Slot]bool{}, &out)
					fmt.Println(strings.Join(out, "\n"))
				}
			}
		}
	}
	if *roots {
		hist := map[string]int{}
		examples := map[string][]string{}
		for _, fn := range a.order {
			for _, r := range fn.Slots.Results {
				if r.Option {
					k := a.root(r, map[*Slot]bool{})
					hist[k]++
					if len(examples[k]) < 4 {
						examples[k] = append(examples[k], fn.Name)
					}
				}
			}
		}
		type kv struct {
			k string
			v int
		}
		var h []kv
		for k, v := range hist {
			h = append(h, kv{k, v})
		}
		sort.Slice(h, func(i, j int) bool { return h[i].v > h[j].v || h[i].v == h[j].v && h[i].k < h[j].k })
		for i, x := range h {
			if i >= 40 {
				break
			}
			fmt.Printf("%5d %s   e.g. %s\n", x.v, x.k, strings.Join(examples[x.k], ", "))
		}
	}
	if *flowTrace != "" {
		for _, fn := range a.order {
			if fn.Name != *flowTrace {
				continue
			}
			for i, p := range fn.Slots.Params {
				for hop := p; hop != nil && hop.flowFrom != nil; {
					fmt.Printf("%s.p%d <- %s\n", ownerName(hop), i, hop.flowFrom.why)
					var next *Slot
					src := hop.flowFrom.src
					if src.kind == srcVar {
						if vi := a.vars[src.v]; vi != nil && vi.slot != nil && vi.slot.NilFlow {
							next = vi.slot
							i = -1
						}
					}
					hop = next
				}
			}
		}
	}
	if *hot {
		type hf struct {
			fn   *Func
			what string
		}
		var list []hf
		for _, fn := range a.order {
			for i, p := range fn.Slots.Params {
				if p.Option {
					list = append(list, hf{fn, fmt.Sprintf("p%d %s", i, p.Why)})
				}
			}
			for i, r := range fn.Slots.Results {
				if r.Option {
					list = append(list, hf{fn, fmt.Sprintf("r%d %s", i, r.Why)})
				}
			}
		}
		sort.SliceStable(list, func(i, j int) bool { return list[i].fn.Calls > list[j].fn.Calls })
		for i, x := range list {
			if i >= 60 {
				break
			}
			fmt.Printf("%5d %s.%s\n", x.fn.Calls, x.fn.Name, x.what)
		}
	}
	if *flowRoots {
		hist := map[string]int{}
		examples := map[string][]string{}
		for _, fn := range a.order {
			for _, p := range fn.Slots.Params {
				if p.flowFrom != nil {
					k := a.flowRoot(p, 0)
					hist[k]++
					if len(examples[k]) < 3 {
						examples[k] = append(examples[k], fn.Name)
					}
				}
			}
		}
		type kv struct {
			k string
			v int
		}
		var h []kv
		for k, v := range hist {
			h = append(h, kv{k, v})
		}
		sort.Slice(h, func(i, j int) bool { return h[i].v > h[j].v || h[i].v == h[j].v && h[i].k < h[j].k })
		for i, x := range h {
			if i >= 30 {
				break
			}
			fmt.Printf("%5d %s   e.g. %s\n", x.v, x.k, strings.Join(examples[x.k], ", "))
		}
	}
	g.plan()
	g.assignNames()
	g.scanHandWritten()
	st, err := g.emit(*dry)
	if err != nil {
		fatal(err)
	}

	if *whyPath != "" {
		var b strings.Builder
		for _, fn := range a.order {
			if fn.RustName == "" {
				continue
			}
			if w := fn.whyLines(); w != "" {
				fmt.Fprintf(&b, "# %s %s:%d\n%s\n", g.funcKey(fn), fn.File, fn.Line, w)
			}
		}
		if err := os.WriteFile(*whyPath, []byte(b.String()), 0o644); err != nil {
			fatal(err)
		}
	}

	fmt.Printf("functions emitted: %d (+ %d Checker function fields), advisory (not emitted): %d\n", st.funcs, st.fields, len(g.advisory))
	fmt.Printf("nil-able params: %d, Option: %d\n", st.ptrParams, st.optParams)
	fmt.Printf("nil-able results: %d, Option: %d\n", st.ptrResults, st.optResults)
	total := 0
	type kv struct {
		k string
		v int
	}
	var um []kv
	for k, v := range tm.unmapped {
		total += v
		um = append(um, kv{k, v})
	}
	sort.Slice(um, func(i, j int) bool {
		if um[i].v != um[j].v {
			return um[i].v > um[j].v
		}
		return um[i].k < um[j].k
	})
	fmt.Printf("unmapped /*?*/: %d occurrences, %d distinct\n", total, len(um))
	for _, x := range um {
		fmt.Printf("  %5d  %s\n", x.v, x.k)
	}
	var fk []string
	for k := range tm.foreign {
		fk = append(fk, k)
	}
	sort.Strings(fk)
	fmt.Printf("named types from other packages (by Go name): %s\n", strings.Join(fk, ", "))
	if len(g.skippedHand) > 0 {
		var names []string
		for _, fn := range g.skippedHand {
			names = append(names, fn.RustName)
		}
		fmt.Printf("already defined by hand (not generated): %d: %s\n", len(names), strings.Join(names, ", "))
	}
	fmt.Printf("merged exported forwarders: %d\n", len(g.merges))
	for _, m := range g.merges {
		fmt.Println("  " + m)
	}
	fmt.Printf("name collisions: %d\n", len(g.collisions))
	for _, c := range g.collisions {
		fmt.Println("  " + c)
	}
	// Flag names with single-letter inner segments (usually a missing word replacement).
	odd := regexp.MustCompile(`(^|_)[a-z](_|$)`)
	var suspicious []string
	for _, fn := range a.order {
		if fn.RustName != "" && odd.MatchString(fn.RustName) {
			suspicious = append(suspicious, fn.Name+" -> "+fn.RustName)
		}
	}
	if len(suspicious) > 0 {
		fmt.Printf("names with single-letter segments (check wordReplacements): %d\n", len(suspicious))
		for _, s := range suspicious {
			fmt.Println("  " + s)
		}
	}
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "gosig:", err)
	os.Exit(1)
}
