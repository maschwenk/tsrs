package main

import (
	"sort"
	"strings"
	"unicode"
)

var rustKeywords = map[string]bool{}

func init() {
	for _, k := range strings.Fields(`as break const continue crate else enum extern false fn for if impl in let loop
		match mod move mut pub ref return self Self static struct super trait true type unsafe use where while
		async await dyn abstract become box do final macro override priv typeof unsized virtual yield try gen`) {
		rustKeywords[k] = true
	}
}

type namer struct {
	repl []string // pairs, longest key first
}

func newNamer(repl map[string]string) *namer {
	keys := make([]string, 0, len(repl))
	for k := range repl {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool {
		if len(keys[i]) != len(keys[j]) {
			return len(keys[i]) > len(keys[j])
		}
		return keys[i] < keys[j]
	})
	n := &namer{}
	for _, k := range keys {
		n.repl = append(n.repl, k, repl[k])
	}
	return n
}

// snake converts a Go identifier to snake_case, treating acronym runs as one word:
// getJSXElementType -> get_jsx_element_type, isESSymbol -> is_es_symbol, nodeID -> node_id.
func (n *namer) snake(name string) string {
	if n != nil && len(n.repl) > 0 {
		name = strings.NewReplacer(n.repl...).Replace(name)
	}
	rs := []rune(name)
	var out []rune
	for i, r := range rs {
		if unicode.IsUpper(r) {
			if i > 0 && len(out) > 0 && out[len(out)-1] != '_' {
				prev := rs[i-1]
				nextLower := i+1 < len(rs) && unicode.IsLower(rs[i+1])
				if unicode.IsLower(prev) || unicode.IsDigit(prev) || (unicode.IsUpper(prev) && nextLower) {
					out = append(out, '_')
				}
			}
			out = append(out, unicode.ToLower(r))
		} else {
			out = append(out, r)
		}
	}
	return string(out)
}

// ident returns a Rust-safe snake_case identifier.
func (n *namer) ident(name string) string {
	s := n.snake(name)
	if rustKeywords[s] {
		s += "_"
	}
	return s
}
