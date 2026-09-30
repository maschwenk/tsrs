#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pattern {
    pub text: String,
    pub star_index: i32, // -1 for exact match
}

pub fn try_parse_pattern(pattern: &str) -> Pattern {
    let star_index = pattern.find('*');
    match star_index {
        None => Pattern { text: pattern.to_string(), star_index: -1 },
        Some(i) if !pattern[i + 1..].contains('*') => Pattern { text: pattern.to_string(), star_index: i as i32 },
        _ => Pattern::default(),
    }
}

impl Pattern {
    pub fn is_valid(&self) -> bool {
        self.star_index == -1 || (self.star_index as usize) < self.text.len()
    }

    pub fn matches(&self, candidate: &str) -> bool {
        if self.star_index == -1 {
            return self.text == candidate;
        }
        let star = self.star_index as usize;
        candidate.len() + 1 >= self.text.len()
            && candidate.as_bytes().starts_with(&self.text.as_bytes()[..star])
            && candidate.as_bytes().ends_with(&self.text.as_bytes()[star + 1..])
    }

    pub fn matched_text<'a>(&self, candidate: &'a str) -> &'a str {
        if !self.matches(candidate) {
            panic!("candidate does not match pattern");
        }
        if self.star_index == -1 {
            return "";
        }
        let star = self.star_index as usize;
        &candidate[star..candidate.len() + star + 1 - self.text.len()]
    }
}

pub fn find_best_pattern_match<T: Clone>(values: &[T], mut get_pattern: impl FnMut(&T) -> Pattern, candidate: &str) -> Option<T> {
    let mut best_pattern = None;
    let mut longest_match_prefix_length = -1;
    for value in values {
        let pattern = get_pattern(value);
        if (pattern.star_index == -1 || pattern.star_index > longest_match_prefix_length) && pattern.matches(candidate) {
            best_pattern = Some(value.clone());
            longest_match_prefix_length = pattern.star_index;
        }
    }
    best_pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_overlapping_match() {
        let p = try_parse_pattern("ab*ab");
        assert!(!p.matches("ab"));
        assert!(p.matches("abXab"));
        assert_eq!(p.matched_text("abXab"), "X");
        assert!(p.matches("abab"));
        assert_eq!(p.matched_text("abab"), "");
        assert_eq!(try_parse_pattern("a*b*c"), Pattern::default());
    }
}
