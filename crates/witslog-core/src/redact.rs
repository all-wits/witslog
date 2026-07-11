use regex::Regex;
use serde_json::Value as JsonValue;

const REDACTED: &str = "«redacted»";

#[derive(Debug, thiserror::Error)]
pub enum RedactError {
    #[error("invalid redaction regex '{pattern}': {source}")]
    InvalidPattern {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

struct Rule {
    pattern: Regex,
}

/// Applies built-in + configured secret-redaction patterns to event text.
pub struct Redactor {
    rules: Vec<Rule>,
}

fn built_in_patterns() -> &'static [&'static str] {
    &[
        // Authorization: Bearer <token> — keep prefix, redact token
        r"(?i)(Authorization:\s*Bearer\s+)\S+",
        // api_key / apikey = value
        r#"(?i)(api[_-]?key["']?\s*[:=]\s*["']?)[\w-]{8,}"#,
        // password = value
        r#"(?i)(password["']?\s*[:=]\s*["']?)\S+"#,
        // AWS_* env-style secrets
        r"(AWS_[A-Z_]*\s*[:=]\s*)\S+",
        // AWS access key ids
        r"(AKIA)[0-9A-Z]{16}",
        // connection strings: scheme://user:pass@host
        r"([a-zA-Z]+://[^:\s/]+:)[^@\s]+(@)",
    ]
}

impl Rule {
    fn compile(pattern: &str) -> Result<Self, RedactError> {
        let compiled = Regex::new(pattern).map_err(|source| RedactError::InvalidPattern {
            pattern: pattern.to_string(),
            source,
        })?;
        Ok(Rule { pattern: compiled })
    }

    fn apply(&self, input: &str) -> String {
        // Convention: capture group 1 (if present) is a non-secret prefix to keep,
        // groups 2.. (if present) are a non-secret suffix to keep (e.g. the '@' in
        // a connection string); everything else in the full match is the secret,
        // replaced with a single REDACTED marker between prefix and suffix.
        let mut out = String::with_capacity(input.len());
        let mut last_end = 0;
        for caps in self.pattern.captures_iter(input) {
            let m = caps.get(0).unwrap();
            out.push_str(&input[last_end..m.start()]);
            if let Some(prefix) = caps.get(1) {
                out.push_str(prefix.as_str());
            }
            out.push_str(REDACTED);
            for i in 2..caps.len() {
                if let Some(suffix) = caps.get(i) {
                    out.push_str(suffix.as_str());
                }
            }
            last_end = m.end();
        }
        out.push_str(&input[last_end..]);
        out
    }
}

impl Redactor {
    /// Built-in patterns only, no custom rules. Never fails.
    pub fn built_in() -> Self {
        let rules = built_in_patterns()
            .iter()
            .map(|p| Rule::compile(p).expect("built-in redaction pattern must compile"))
            .collect();
        Redactor { rules }
    }

    /// Built-ins plus caller-supplied regex patterns. Each custom pattern is
    /// applied whole-match (no capture-group prefix preservation).
    pub fn new(custom_patterns: &[String]) -> Result<Self, RedactError> {
        let mut rules: Vec<Rule> = built_in_patterns()
            .iter()
            .map(|p| Rule::compile(p).expect("built-in redaction pattern must compile"))
            .collect();

        for pattern in custom_patterns {
            rules.push(Rule::compile(pattern)?);
        }

        Ok(Redactor { rules })
    }

    pub fn redact(&self, input: &str) -> String {
        let mut current = input.to_string();
        for rule in &self.rules {
            current = rule.apply(&current);
        }
        current
    }

    pub fn redact_json(&self, value: &mut JsonValue) {
        match value {
            JsonValue::String(s) => {
                *s = self.redact(s);
            }
            JsonValue::Array(items) => {
                for item in items {
                    self.redact_json(item);
                }
            }
            JsonValue::Object(map) => {
                for (_, v) in map.iter_mut() {
                    self.redact_json(v);
                }
            }
            _ => {}
        }
    }
}

impl Default for Redactor {
    fn default() -> Self {
        Redactor::built_in()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_token() {
        let r = Redactor::built_in();
        let out = r.redact("Authorization: Bearer abc.def.ghi");
        assert_eq!(out, "Authorization: Bearer «redacted»");
    }

    #[test]
    fn redacts_api_key() {
        let r = Redactor::built_in();
        let out = r.redact(r#"api_key: "sk_live_abcdef1234567890""#);
        assert!(out.contains(REDACTED));
        assert!(!out.contains("sk_live_abcdef1234567890"));
    }

    #[test]
    fn redacts_password() {
        let r = Redactor::built_in();
        let out = r.redact("password=hunter2secret");
        assert!(out.contains(REDACTED));
        assert!(!out.contains("hunter2secret"));
    }

    #[test]
    fn redacts_aws_key() {
        let r = Redactor::built_in();
        let out = r.redact("AWS_SECRET_ACCESS_KEY=abcd1234EFGH5678ijkl");
        assert!(out.contains(REDACTED));
        assert!(!out.contains("abcd1234EFGH5678ijkl"));
    }

    #[test]
    fn redacts_akia_key() {
        let r = Redactor::built_in();
        let out = r.redact("key is AKIA1234567890ABCDEF elsewhere");
        assert!(out.contains(REDACTED));
        assert!(!out.contains("AKIA1234567890ABCDEF"));
    }

    #[test]
    fn redacts_connection_string() {
        let r = Redactor::built_in();
        let out = r.redact("postgres://user:supersecret@localhost:5432/db");
        assert!(out.contains(REDACTED));
        assert!(!out.contains("supersecret"));
        assert!(out.starts_with("postgres://user:"));
    }

    #[test]
    fn custom_pattern_applied() {
        let r = Redactor::new(&["MY_SECRET_[A-Z0-9]+=\\S+".to_string()]).unwrap();
        let out = r.redact("MY_SECRET_TOKEN=xyz123");
        assert!(out.contains(REDACTED));
        assert!(!out.contains("xyz123"));
    }

    #[test]
    fn invalid_custom_pattern_rejected() {
        let err = Redactor::new(&["(unclosed".to_string()]);
        assert!(err.is_err());
    }

    #[test]
    fn redact_json_recurses() {
        let r = Redactor::built_in();
        let mut v = serde_json::json!({
            "note": "password=hunter2secret",
            "nested": { "auth": "Authorization: Bearer abc" },
            "list": ["password=another1secret"]
        });
        r.redact_json(&mut v);
        assert!(!v.to_string().contains("hunter2secret"));
        assert!(!v.to_string().contains("another1secret"));
        assert!(v.to_string().contains(REDACTED));
    }

    #[test]
    fn non_matching_text_untouched() {
        let r = Redactor::built_in();
        let input = "plain error message with no secrets";
        assert_eq!(r.redact(input), input);
    }
}
