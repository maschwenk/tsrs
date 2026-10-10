use super::ExcludeRegex;

fn compare(pattern: &str, haystacks: &[&str]) {
    let reference = regex::Regex::new(pattern);
    let matcher = ExcludeRegex::new(pattern);
    assert_eq!(matcher.is_some(), reference.is_ok(), "pattern acceptance: {pattern}");
    if let (Some(matcher), Ok(reference)) = (matcher, reference) {
        for haystack in haystacks {
            assert_eq!(
                matcher.is_match(haystack),
                reference.is_match(haystack),
                "pattern {pattern}, input {haystack:?}"
            );
        }
    }
}

#[test]
fn syntax_and_unicode_match_current_engine() {
    let haystacks = [
        "",
        "src/utils/index",
        "lodash/fp",
        "Lodash/fp",
        "foo/bar",
        "ramda/internal/utils",
        "src/café/工具",
        "Κόσμος/αβ",
        "K",
        "ß",
        "123/٤٥٦",
        "a\nb",
        "É",
        "aaaaab",
        "ab",
        "αβγ",
    ];
    for pattern in [
        r"utils",
        r"^lodash/",
        r"(?i:^lodash/)",
        r"(?:foo|bar|quux)",
        r"\p{Letter}+",
        r"\p{Script=Greek}+",
        r"\p{Age=6.0}+",
        r"\p{Alphabetic}+",
        r"\d+",
        r"\w+",
        r"\s+",
        r"\butils\b",
        r"\b\w+\b",
        r"^\b\w+\b$",
        r"(?i:k)",
        r"(?i:ß)",
        "",
        r"^$",
        r"(?m:^src)",
        r"(?s:a.b)",
        r"[a-z&&[^aeiou]]+",
        r"[\pL--\p{Greek}]",
        r"(a|ab)+",
        r"(a?){10}",
        r"(?P<word>\w+)",
        r"(?-u:[a-z]+)",
        r"(?-u:\xFF)",
        "(",
        "[",
        r"(?=a)",
        r"(a)\1",
        "(?<bad",
        r"\p{notvalid}",
        "[z-a]",
        "a{100000000}",
        r"\Afoo\z",
        r"(?U:a+)",
        "a{0}",
        r"\B",
        "(?x:a # comment\n b)",
        r"(?:src|lib)/.*(?:\.test|\.spec)$",
        "[ab]{6}",
        "(?i:[a-z]{2})",
        "(?:a|ab)",
        "(?:ab|a)",
        "(?-u:a|b)",
        "[^a]",
        "[éαβ]",
        "(?i:é)",
        r"\p{Greek}{0}",
        "(?:a{0}|b)",
    ] {
        compare(pattern, &haystacks);
    }
}

#[test]
fn generated_patterns_match_current_engine() {
    struct Generator(u64);
    impl Generator {
        fn pick(&mut self, choices: usize) -> usize {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 32) as usize) % choices
        }
        fn pattern(&mut self, depth: usize) -> String {
            let atoms = [
                "a",
                "b",
                "é",
                "K",
                "α",
                r"\w",
                r"\d",
                r"\s",
                r"\p{L}",
                r"\p{Greek}",
                "[a-z]",
                "[^a/]",
                ".",
                "",
                r"\b",
                r"\B",
            ];
            if depth == 0 || self.pick(4) == 0 {
                return atoms[self.pick(atoms.len())].to_string();
            }
            let kind = self.pick(5);
            let first = self.pattern(depth - 1);
            match kind {
                0 => format!("(?:{first}|{})", self.pattern(depth - 1)),
                1 => {
                    let repeats = ["?", "*", "+", "{0,3}", "+?"];
                    format!("(?:{first}){}", repeats[self.pick(repeats.len())])
                }
                2 => format!("{first}{}", self.pattern(depth - 1)),
                3 => format!("(?i:{first})"),
                _ => format!("({first})"),
            }
        }
    }

    let mut generator = Generator(51719);
    for _ in 0..150 {
        let mut pattern = generator.pattern(3);
        if generator.pick(3) == 0 {
            pattern = format!("^{pattern}$");
        }
        let haystacks: Vec<_> = (0..8)
            .map(|_| {
                let alphabet = ['a', 'b', '/', 'é', 'α', 'K', '1', '٤', ' ', '\n'];
                (0..generator.pick(16))
                    .map(|_| alphabet[generator.pick(alphabet.len())])
                    .collect::<String>()
            })
            .collect();
        let inputs: Vec<_> = haystacks.iter().map(String::as_str).collect();
        compare(&pattern, &inputs);
    }
}

#[test]
fn compilation_limits_match_current_engine() {
    for pattern in ["(a){40000}", "(a){80000}", "(a){160000}", "(a){240000}", r"\w{160}", r"\p{L}{160}"] {
        compare(pattern, &["", "a"]);
    }
    compare(&"x".repeat(600_000), &[""]);
}

#[test]
fn large_literal_alternations_keep_the_nfa_bypass() {
    for count in [2999, 3000, 30_000, 60_000] {
        let words: Vec<_> = (0..count)
            .map(|i| format!("{:016x}", (i as u64).wrapping_mul(0x9e3779b97f4a7c15)))
            .collect();
        let pattern = words.join("|");
        let haystacks = ["", words[0].as_str(), words[count / 2].as_str(), words[count - 1].as_str(), "not_a_match"];
        compare(&pattern, &haystacks);
        if count == 30_000 {
            // Captures and assertions must not bypass the original compilation limit.
            compare(&format!("({pattern})"), &haystacks);
            compare(&format!("^(?:{pattern})$"), &haystacks);
        }
    }
}

#[test]
fn execution_cache_is_reused_safely_between_threads() {
    let matcher = ExcludeRegex::new(r"^\b\p{L}+\b/\w+$").unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let matcher = &matcher;
            scope.spawn(move || {
                for _ in 0..100 {
                    assert!(matcher.is_match("café/工具"));
                    assert!(!matcher.is_match("café/工具!"));
                }
            });
        }
    });
}
