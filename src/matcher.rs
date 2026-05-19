use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};

#[derive(Debug, Clone)]
pub struct PatternMatcher {
    patterns: Vec<String>,
    regex_sources: Vec<String>,
    regexes: Vec<Regex>,
}

impl PatternMatcher {
    pub fn new(patterns: Vec<String>) -> Self {
        Self::try_new(patterns, vec![]).expect("empty regex list must compile")
    }

    pub fn try_new(patterns: Vec<String>, regexes: Vec<String>) -> Result<Self> {
        let patterns = patterns
            .into_iter()
            .map(|pattern| pattern.trim().to_ascii_lowercase())
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>();
        let regex_sources = regexes
            .into_iter()
            .map(|pattern| pattern.trim().to_string())
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>();
        let regexes = regex_sources
            .iter()
            .map(|pattern| {
                RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .with_context(|| format!("invalid BLOCK_CNAME_REGEXES pattern: {pattern}"))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            patterns,
            regex_sources,
            regexes,
        })
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
        }) || self.regexes.iter().any(|pattern| {
            pattern.is_match(&caller_name) || headers.iter().any(|header| pattern.is_match(header))
        })
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    pub fn regex_patterns(&self) -> &[String] {
        &self.regex_sources
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

    #[test]
    fn regex_patterns_match_case_insensitively() {
        let matcher = PatternMatcher::try_new(vec![], vec![r"[[:alpha:]] CA$".into()]).unwrap();
        assert!(matcher.is_match("UPLAND CA", &[]));
        assert!(matcher.is_match("Brea ca", &[]));
        assert!(matcher.is_match("ONTARIO CA", &[]));
        assert!(!matcher.is_match("CA", &[]));
        assert!(!matcher.is_match("UPLAND NY", &[]));
        assert!(!matcher.is_match("PCH CLAIMS", &[]));
    }

    #[test]
    fn regex_patterns_match_from_header_text() {
        let matcher = PatternMatcher::try_new(vec![], vec![r#""[[:alpha:]]+ CA""#.into()]).unwrap();
        assert!(matcher.is_match(
            "",
            &["\"UPLAND CA\" <sip:+19093604678@example.test>".into()]
        ));
    }

    #[test]
    fn invalid_regexes_return_errors() {
        let err = PatternMatcher::try_new(vec![], vec!["(".into()]).unwrap_err();
        assert!(
            err.to_string()
                .contains("invalid BLOCK_CNAME_REGEXES pattern")
        );
    }
}
