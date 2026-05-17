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
            caller_name.contains(pattern) || headers.iter().any(|header| header.contains(pattern))
        })
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
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
}
