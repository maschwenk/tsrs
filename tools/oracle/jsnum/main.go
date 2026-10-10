// Stream protocol shared with crates/tsrs_core/examples/jsnum_oracle.rs:
// "format <f64 bits in hex>" -> JavaScript number string;
// "parse <text>" -> f64 bits in hex.
package main

import (
	"bufio"
	"fmt"
	"math"
	"os"
	"strconv"
	"strings"

	"github.com/microsoft/TypeScript/tsc/internal/jsnum"
)

func main() {
	in := bufio.NewScanner(os.Stdin)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		mode, value, _ := strings.Cut(in.Text(), " ")
		switch mode {
		case "format":
			bits, err := strconv.ParseUint(value, 16, 64)
			if err != nil {
				panic(err)
			}
			fmt.Fprintln(out, jsnum.Number(math.Float64frombits(bits)).String())
		case "parse":
			fmt.Fprintf(out, "%016x\n", math.Float64bits(float64(jsnum.FromString(value))))
		default:
			panic("unknown mode: " + mode)
		}
	}
	if err := in.Err(); err != nil {
		panic(err)
	}
}
