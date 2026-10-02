// Oracle for tsrs_fourslash::test_parser: runs Go's fourslash.ParseTestData on every constant test content
// (written by `go run ./tools/gen-fourslash -parser-inputs FILE`) and writes the parse results as JSON lines.
//
// Copy of ts-ref/tsc/cmd/tsrs-oracle-fourslash-parser/oracle_test.go (Go internal packages can only be imported
// from inside the module). Run from ts-ref/tsc:
//
//	FOURSLASH_PARSER_IN=in.jsonl FOURSLASH_PARSER_OUT=out.jsonl GOTOOLCHAIN=auto go test ./cmd/tsrs-oracle-fourslash-parser -run TestDump -count=1
package main

import (
	"bufio"
	"encoding/json"
	"os"
	"testing"

	"github.com/microsoft/TypeScript/tsc/internal/fourslash"
)

type input struct {
	Test     string `json:"test"`
	FileName string `json:"fileName"`
	Content  string `json:"content"`
}

type position struct {
	Line      uint32 `json:"line"`
	Character uint32 `json:"character"`
}

type marker struct {
	FileName string         `json:"fileName"`
	Position int            `json:"position"`
	LS       position       `json:"ls"`
	Name     *string        `json:"name"`
	Data     map[string]any `json:"data"`
}

type rangeMarker struct {
	FileName string   `json:"fileName"`
	Pos      int      `json:"pos"`
	End      int      `json:"end"`
	Start    position `json:"start"`
	Stop     position `json:"stop"`
	Marker   int      `json:"marker"` // index into markers, -1 for none
}

type file struct {
	FileName string `json:"fileName"`
	Content  string `json:"content"`
}

type output struct {
	input
	Error         bool              `json:"error"`
	Files         []file            `json:"files"`
	Markers       []marker          `json:"markers"`
	MarkerNames   []string          `json:"markerNames"`
	Ranges        []rangeMarker     `json:"ranges"`
	Symlinks      map[string]string `json:"symlinks"`
	GlobalOptions map[string]string `json:"globalOptions"`
}

func TestDump(t *testing.T) {
	in, err := os.Open(os.Getenv("FOURSLASH_PARSER_IN"))
	if err != nil {
		t.Fatal(err)
	}
	defer in.Close()
	outFile, err := os.Create(os.Getenv("FOURSLASH_PARSER_OUT"))
	if err != nil {
		t.Fatal(err)
	}
	defer outFile.Close()
	w := bufio.NewWriter(outFile)
	defer w.Flush()
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)

	sc := bufio.NewScanner(in)
	sc.Buffer(make([]byte, 64<<20), 64<<20)
	for sc.Scan() {
		var inp input
		if err := json.Unmarshal(sc.Bytes(), &inp); err != nil {
			t.Fatal(err)
		}
		o := output{input: inp}
		ok := t.Run(inp.Test, func(t *testing.T) {
			td := fourslash.ParseTestData(t, inp.Content, inp.FileName)
			index := map[*fourslash.Marker]int{}
			for i, m := range td.Markers {
				index[m] = i
				o.Markers = append(o.Markers, marker{m.FileName(), m.Position, position{m.LSPosition.Line, m.LSPosition.Character}, m.Name, m.Data})
			}
			for name := range td.MarkerPositions {
				o.MarkerNames = append(o.MarkerNames, name)
			}
			for _, r := range td.Ranges {
				mi := -1
				if r.Marker != nil {
					mi = index[r.Marker]
				}
				o.Ranges = append(o.Ranges, rangeMarker{r.FileName(), r.Range.Pos(), r.Range.End(),
					position{r.LSRange.Start.Line, r.LSRange.Start.Character}, position{r.LSRange.End.Line, r.LSRange.End.Character}, mi})
			}
			for _, f := range td.Files {
				o.Files = append(o.Files, file{f.FileName(), f.Content})
			}
			o.Symlinks = td.Symlinks
			o.GlobalOptions = td.GlobalOptions
		})
		if !ok {
			o = output{input: inp, Error: true}
		}
		if err := enc.Encode(o); err != nil {
			t.Fatal(err)
		}
	}
	if err := sc.Err(); err != nil {
		t.Fatal(err)
	}
}
