#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatcher {
    patterns: Vec<String>,
}

impl PatternMatcher {
    pub fn new(patterns: Vec<String>) -> Self {
        let patterns = patterns
            .into_iter()
            .map(|pattern| pattern.trim().to_ascii_lowercase())
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>();
        Self { patterns }
    }

    pub fn is_match(&self, caller_name: &str, from_headers: &[String]) -> bool {
        let caller_name = caller_name.to_ascii_lowercase();
        let headers = from_headers
            .iter()
            .map(|value| value.to_ascii_lowercase())
            .collect::<Vec<_>>();

        self.patterns.iter().any(|pattern| {
            contains_token_pattern(&caller_name, pattern)
                || headers
                    .iter()
                    .any(|header| contains_token_pattern(header, pattern))
        })
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

fn contains_token_pattern(haystack: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }

    let mut start = 0;
    while let Some(offset) = haystack[start..].find(pattern) {
        let match_start = start + offset;
        let match_end = match_start + pattern.len();
        if is_boundary(haystack, match_start) && is_boundary(haystack, match_end) {
            return true;
        }
        start = match_start + 1;
    }

    false
}

fn is_boundary(value: &str, byte_index: usize) -> bool {
    if byte_index == 0 || byte_index == value.len() {
        return true;
    }

    let before = value[..byte_index].chars().next_back();
    let after = value[byte_index..].chars().next();
    !matches!(
        (before, after),
        (Some(left), Some(right)) if is_word_char(left) && is_word_char(right)
    )
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_caller_name_case_insensitively() {
        let matcher = PatternMatcher::new(vec!["nelson".into()]);
        assert!(matcher.is_match("NELSON", &[]));
        assert!(matcher.is_match("Potential Nelson Scam", &[]));
        assert!(!matcher.is_match("Pat", &[]));
    }

    #[test]
    fn falls_back_to_from_header_text() {
        let matcher = PatternMatcher::new(vec!["nelson".into()]);
        assert!(matcher.is_match("", &["\"Nelson\" <sip:+15551212@example.test>".into()]));
    }

    #[test]
    fn pch_matches_as_token_not_embedded_text() {
        let matcher = PatternMatcher::new(vec!["pch".into()]);
        assert!(matcher.is_match("PCH", &[]));
        assert!(matcher.is_match("PCH-CLAIMS", &[]));
        assert!(matcher.is_match("CALL FROM PCH INC", &[]));
        assert!(!matcher.is_match("Kupchak", &[]));
        assert!(!matcher.is_match("shopcharge", &[]));
    }

    #[test]
    fn phrase_patterns_match_on_phrase_boundaries() {
        let matcher = PatternMatcher::new(vec!["publishers clearing house".into()]);
        assert!(matcher.is_match("Publishers Clearing House", &[]));
        assert!(matcher.is_match("THE PUBLISHERS CLEARING HOUSE DEPT", &[]));
        assert!(!matcher.is_match("xpublishers clearing house", &[]));
        assert!(!matcher.is_match("publishers clearing housex", &[]));
    }
}
