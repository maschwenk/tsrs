// gen-fourslash converts the Go fourslash tests (ts-ref/tsc/internal/fourslash/tests) into Rust test functions
// for crates/tsrs_fourslash. See docs/LSP.md "Fourslash" and notes/lsp-fsgen.md.
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
)

func main() {
	tscDir := flag.String("tsc", "../../ts-ref/tsc", "Go module root of the reference checkout")
	outDir := flag.String("out", "../../crates/tsrs_fourslash/src/tests", "output directory (gets gen/ and util_gen.rs)")
	doSurvey := flag.Bool("survey", false, "print node kinds, call targets and interface conversions, then exit")
	doSigs := flag.Bool("sigs", false, "print the fourslash API the tests use, then exit")
	perFile := flag.Int("per-file", 0, "tests per generated file (0 = default)")
	flag.Parse()

	l, err := load(*tscDir)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if *doSigs {
		sigs(l)
		return
	}
	if *doSurvey {
		survey(l)
		return
	}
	abs, _ := filepath.Abs(*outDir)
	if err := generate(l, abs, *perFile); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
