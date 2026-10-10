//! Boolean matching for the cold auto-import exclusion preference. Keep the syntax and Unicode support of
//! `regex`, but omit its search accelerators. The literal selection below follows regex-automata 0.4.18's
//! `meta::strategy::Pre` and `meta::literal::alternation_literals`: these paths also affect which patterns compile.

use std::sync::Mutex;

use regex_automata::nfa::thompson::{
    self,
    pikevm::{Cache, PikeVM},
    NFA, WhichCaptures,
};
use regex_syntax::hir::{literal, Hir, HirKind};

pub(crate) enum ExcludeRegex {
    Literals(Vec<Vec<u8>>),
    Pike { vm: PikeVM, cache: Mutex<Box<Cache>> },
}

impl ExcludeRegex {
    pub(crate) fn new(pattern: &str) -> Option<Self> {
        let hir = regex_syntax::ParserBuilder::new().utf8(true).build().parse(pattern).ok()?;
        let properties = hir.properties();
        if properties.explicit_captures_len() == 0 && properties.look_set().is_empty() {
            let mut prefixes = literal::Seq::empty();
            prefixes.union(&mut literal::Extractor::new().extract(&hir));
            prefixes.optimize_for_prefix_by_preference();
            if prefixes.is_exact() {
                if let Some(literals) = prefixes.literals() {
                    if !literals.is_empty() && literals.iter().all(|literal| !literal.as_bytes().is_empty()) {
                        return Some(Self::Literals(literals.iter().map(|literal| literal.as_bytes().to_vec()).collect()));
                    }
                }
            }

            if properties.is_alternation_literal() {
                if let HirKind::Alternation(alternatives) = hir.kind() {
                    // The current meta-engine bypasses the NFA limit for this many literal alternatives.
                    if alternatives.len() >= 3000 {
                        let mut literals = Vec::with_capacity(alternatives.len());
                        for alternative in alternatives {
                            let mut bytes = Vec::new();
                            append_literal_bytes(alternative, &mut bytes);
                            literals.push(bytes);
                        }
                        return Some(Self::Literals(literals));
                    }
                }
            }
        }

        let config = thompson::Config::new()
            .utf8(true)
            .nfa_size_limit(Some(10 * (1 << 20)))
            .shrink(false)
            .which_captures(WhichCaptures::All);
        let nfa = NFA::compiler().configure(config.clone()).build_from_hir(&hir).ok()?;
        let vm = PikeVM::new_from_nfa(nfa).ok()?;
        // `regex::Regex` also builds a reverse NFA. Preserve its rejection at the same limit even though
        // boolean PikeVM searches only need the forward NFA. Captures are unsupported in reverse NFAs.
        NFA::compiler()
            .configure(config.which_captures(WhichCaptures::None).reverse(true))
            .build_from_hir(&hir)
            .ok()?;
        // Box the execution cache so literal-only patterns don't reserve space for the large PikeVM cache.
        let cache = Mutex::new(Box::new(vm.create_cache()));
        Some(Self::Pike { vm, cache })
    }

    pub(crate) fn is_match(&self, haystack: &str) -> bool {
        match self {
            Self::Literals(literals) => literals
                .iter()
                .any(|literal| literal.is_empty() || haystack.as_bytes().windows(literal.len()).any(|window| window == literal.as_slice())),
            Self::Pike { vm, cache } => vm.is_match(&mut cache.lock().unwrap(), haystack),
        }
    }
}

fn append_literal_bytes(hir: &Hir, bytes: &mut Vec<u8>) {
    debug_assert!(matches!(hir.kind(), HirKind::Literal(_) | HirKind::Concat(_)));
    match hir.kind() {
        HirKind::Literal(literal) => bytes.extend_from_slice(&literal.0),
        HirKind::Concat(expressions) => {
            for expression in expressions {
                append_literal_bytes(expression, bytes);
            }
        }
        _ => unreachable!("a literal alternation contains only literals and concatenations"),
    }
}

#[cfg(test)]
mod tests;
