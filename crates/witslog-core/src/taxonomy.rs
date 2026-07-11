use serde::{Deserialize, Serialize};

/// Category tree node — builtin hierarchy (infrastructure/application/runtime/external).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryNode {
    pub canonical: String,
    pub parent: Option<String>,
    pub label: String,
}

/// Raw builtin category data (uses &str to allow const context).
pub(crate) struct BuiltinCategoryData {
    canonical: &'static str,
    parent: Option<&'static str>,
    label: &'static str,
}

/// Builtin category hierarchy. Edit here to change taxonomy without code rebuild.
const BUILTIN_CATEGORIES_DATA: &[BuiltinCategoryData] = &[
    // Root categories
    BuiltinCategoryData { canonical: "infrastructure", parent: None, label: "Infrastructure" },
    BuiltinCategoryData { canonical: "application", parent: None, label: "Application" },
    BuiltinCategoryData { canonical: "runtime", parent: None, label: "Runtime" },
    BuiltinCategoryData { canonical: "external", parent: None, label: "External" },
    // Infrastructure children
    BuiltinCategoryData { canonical: "infrastructure.network", parent: Some("infrastructure"), label: "Network" },
    BuiltinCategoryData { canonical: "infrastructure.network.dns", parent: Some("infrastructure.network"), label: "DNS" },
    BuiltinCategoryData { canonical: "infrastructure.network.timeout", parent: Some("infrastructure.network"), label: "Timeout" },
    BuiltinCategoryData { canonical: "infrastructure.network.connection", parent: Some("infrastructure.network"), label: "Connection" },
    BuiltinCategoryData { canonical: "infrastructure.storage", parent: Some("infrastructure"), label: "Storage" },
    BuiltinCategoryData { canonical: "infrastructure.storage.disk", parent: Some("infrastructure.storage"), label: "Disk" },
    BuiltinCategoryData { canonical: "infrastructure.storage.database", parent: Some("infrastructure.storage"), label: "Database" },
    BuiltinCategoryData { canonical: "infrastructure.compute", parent: Some("infrastructure"), label: "Compute" },
    BuiltinCategoryData { canonical: "infrastructure.compute.memory", parent: Some("infrastructure.compute"), label: "Memory" },
    BuiltinCategoryData { canonical: "infrastructure.compute.cpu", parent: Some("infrastructure.compute"), label: "CPU" },
    // Application children
    BuiltinCategoryData { canonical: "application.error", parent: Some("application"), label: "Error" },
    BuiltinCategoryData { canonical: "application.validation", parent: Some("application"), label: "Validation" },
    BuiltinCategoryData { canonical: "application.authentication", parent: Some("application"), label: "Authentication" },
    BuiltinCategoryData { canonical: "application.authorization", parent: Some("application"), label: "Authorization" },
    // Runtime children
    BuiltinCategoryData { canonical: "runtime.panic", parent: Some("runtime"), label: "Panic" },
    BuiltinCategoryData { canonical: "runtime.segfault", parent: Some("runtime"), label: "Segmentation Fault" },
    BuiltinCategoryData { canonical: "runtime.outofmemory", parent: Some("runtime"), label: "Out of Memory" },
    // External children
    BuiltinCategoryData { canonical: "external.api", parent: Some("external"), label: "API" },
    BuiltinCategoryData { canonical: "external.api.rate_limit", parent: Some("external.api"), label: "Rate Limit" },
    BuiltinCategoryData { canonical: "external.service", parent: Some("external"), label: "Service" },
];

/// Convert const data to owned CategoryNode structs for public API.
pub fn builtin_categories() -> Vec<CategoryNode> {
    BUILTIN_CATEGORIES_DATA
        .iter()
        .map(|data| CategoryNode {
            canonical: data.canonical.to_string(),
            parent: data.parent.map(|s| s.to_string()),
            label: data.label.to_string(),
        })
        .collect()
}

/// Classification result — what rule matched and suggested tags.
#[derive(Debug, Clone)]
pub struct Classification {
    pub canonical: Option<String>,
    pub rule_ids: Vec<String>,
    pub suggested_tags: Vec<String>,
}

/// Rule for auto-classifying errors (error_code map, exception map, message regex/keyword).
#[derive(Debug, Clone)]
pub struct ClassifyRule {
    pub id: String,
    pub kind: ClassifyRuleKind,
    pub canonical: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ClassifyRuleKind {
    ErrorCode(String),           // Exact error_code match
    Exception(String),           // Exact exception type match
    MessageKeyword(String),      // Substring in message (case-insensitive)
    MessageRegex(String),        // Regex on message (compiled on load; see validate_rule)
}

/// Classifier — applies rules in order (error_code → exception → message).
pub struct Classifier {
    pub rules: Vec<ClassifyRule>,
}

impl Classifier {
    pub fn new(rules: Vec<ClassifyRule>) -> Self {
        Classifier { rules }
    }

    /// Builtin + empty custom rules. Call this to get classifier before user-supplied rules.
    pub fn built_in() -> Self {
        let rules = vec![
            ClassifyRule {
                id: "builtin_etimedout".to_string(),
                kind: ClassifyRuleKind::ErrorCode("ETIMEDOUT".to_string()),
                canonical: "infrastructure.network.timeout".to_string(),
                tags: vec!["timeout".to_string()],
            },
            ClassifyRule {
                id: "builtin_econnrefused".to_string(),
                kind: ClassifyRuleKind::ErrorCode("ECONNREFUSED".to_string()),
                canonical: "infrastructure.network.connection".to_string(),
                tags: vec!["connection_refused".to_string()],
            },
            ClassifyRule {
                id: "builtin_enotfound".to_string(),
                kind: ClassifyRuleKind::ErrorCode("ENOTFOUND".to_string()),
                canonical: "infrastructure.network.dns".to_string(),
                tags: vec!["dns_error".to_string()],
            },
            ClassifyRule {
                id: "builtin_enomem".to_string(),
                kind: ClassifyRuleKind::ErrorCode("ENOMEM".to_string()),
                canonical: "infrastructure.compute.memory".to_string(),
                tags: vec!["out_of_memory".to_string()],
            },
            ClassifyRule {
                id: "builtin_panic".to_string(),
                kind: ClassifyRuleKind::Exception("panic".to_string()),
                canonical: "runtime.panic".to_string(),
                tags: vec!["panic".to_string()],
            },
            ClassifyRule {
                id: "builtin_ioerror".to_string(),
                kind: ClassifyRuleKind::Exception("IOException".to_string()),
                canonical: "infrastructure.storage".to_string(),
                tags: vec!["io_error".to_string()],
            },
            ClassifyRule {
                id: "builtin_disk_full".to_string(),
                kind: ClassifyRuleKind::MessageKeyword("disk full".to_string()),
                canonical: "infrastructure.storage.disk".to_string(),
                tags: vec!["disk_full".to_string()],
            },
            ClassifyRule {
                id: "builtin_out_of_memory_msg".to_string(),
                kind: ClassifyRuleKind::MessageKeyword("out of memory".to_string()),
                canonical: "runtime.outofmemory".to_string(),
                tags: vec!["oom".to_string()],
            },
        ];
        Classifier::new(rules)
    }

    /// Classify an event. Deterministic: same input always yields same output.
    /// Applies rules in order (error_code → exception → message). First match wins.
    pub fn classify(
        &self,
        message: &str,
        exception: Option<&str>,
        error_code: Option<&str>,
    ) -> Classification {
        // Priority 1: error_code map
        if let Some(code) = error_code {
            for rule in &self.rules {
                if let ClassifyRuleKind::ErrorCode(ref ec) = rule.kind {
                    if ec == code {
                        return Classification {
                            canonical: Some(rule.canonical.clone()),
                            rule_ids: vec![rule.id.clone()],
                            suggested_tags: rule.tags.clone(),
                        };
                    }
                }
            }
        }

        // Priority 2: exception map
        if let Some(exc) = exception {
            for rule in &self.rules {
                if let ClassifyRuleKind::Exception(ref et) = rule.kind {
                    if et == exc {
                        return Classification {
                            canonical: Some(rule.canonical.clone()),
                            rule_ids: vec![rule.id.clone()],
                            suggested_tags: rule.tags.clone(),
                        };
                    }
                }
            }
        }

        // Priority 3: message keyword/regex
        let msg_lower = message.to_lowercase();
        for rule in &self.rules {
            match &rule.kind {
                ClassifyRuleKind::MessageKeyword(kw) => {
                    if msg_lower.contains(&kw.to_lowercase()) {
                        return Classification {
                            canonical: Some(rule.canonical.clone()),
                            rule_ids: vec![rule.id.clone()],
                            suggested_tags: rule.tags.clone(),
                        };
                    }
                }
                ClassifyRuleKind::MessageRegex(pattern) => {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if re.is_match(message) {
                            return Classification {
                                canonical: Some(rule.canonical.clone()),
                                rule_ids: vec![rule.id.clone()],
                                suggested_tags: rule.tags.clone(),
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        // No match: return null category + unclassified tag
        Classification {
            canonical: None,
            rule_ids: Vec::new(),
            suggested_tags: vec!["unclassified".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_categories_count() {
        assert!(builtin_categories().len() > 10);
    }

    #[test]
    fn error_code_match_returns_canonical() {
        let classifier = Classifier::built_in();
        let result = classifier.classify("connection failed", None, Some("ETIMEDOUT"));
        assert_eq!(result.canonical, Some("infrastructure.network.timeout".to_string()));
        assert!(result.rule_ids.contains(&"builtin_etimedout".to_string()));
        assert!(result.suggested_tags.contains(&"timeout".to_string()));
    }

    #[test]
    fn exception_match_returns_canonical() {
        let classifier = Classifier::built_in();
        let result = classifier.classify("panic in main", Some("panic"), None);
        assert_eq!(result.canonical, Some("runtime.panic".to_string()));
        assert!(result.suggested_tags.contains(&"panic".to_string()));
    }

    #[test]
    fn message_keyword_match_case_insensitive() {
        let classifier = Classifier::built_in();
        let result = classifier.classify("ERROR: Disk Full condition", None, None);
        assert_eq!(result.canonical, Some("infrastructure.storage.disk".to_string()));
    }

    #[test]
    fn no_match_returns_unclassified_tag() {
        let classifier = Classifier::built_in();
        let result = classifier.classify("something weird happened", None, None);
        assert_eq!(result.canonical, None);
        assert!(result.suggested_tags.contains(&"unclassified".to_string()));
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let classifier = Classifier::built_in();
        let result1 = classifier.classify("connection timeout", Some("TimeoutError"), None);
        let result2 = classifier.classify("connection timeout", Some("TimeoutError"), None);
        assert_eq!(result1.canonical, result2.canonical);
        assert_eq!(result1.suggested_tags, result2.suggested_tags);
    }

    #[test]
    fn priority_error_code_wins_over_message() {
        let classifier = Classifier::built_in();
        let result = classifier.classify(
            "out of memory error",
            None,
            Some("ENOMEM"),
        );
        // Should match ENOMEM, not the "out of memory" message keyword
        assert_eq!(result.canonical, Some("infrastructure.compute.memory".to_string()));
        assert!(result.rule_ids.contains(&"builtin_enomem".to_string()));
    }
}
