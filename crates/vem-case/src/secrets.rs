//! Secret-candidate rules scanned over prompts, tool inputs, tool results and pasted content (spec §4).
//! Each rule has a stable id; `RULESET_VERSION` is recorded in the ingest audit entry. A low-confidence
//! generic match that overlaps a specific match is dropped.

use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

pub const RULESET_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretMatch {
    pub rule: &'static str,
    pub confidence: &'static str,
    pub offset: usize,
    pub length: usize,
    pub matched: String,
}

struct Rule {
    id: &'static str,
    confidence: &'static str,
    re: Regex,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |id, confidence, pattern: &str| Rule {
        id,
        confidence,
        re: Regex::new(pattern).expect("valid secret rule"),
    };
    vec![
        r(
            "aws-access-key-id",
            "high",
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        ),
        r(
            "github-token",
            "high",
            r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})",
        ),
        r("gitlab-token", "high", r"\bglpat-[A-Za-z0-9_-]{20,}"),
        r("slack-token", "high", r"\bxox[abposr]-[A-Za-z0-9-]{10,}"),
        r("anthropic-key", "high", r"\bsk-ant-[A-Za-z0-9_-]{20,}"),
        r("openai-key", "high", r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}"),
        r("google-api-key", "high", r"\bAIza[0-9A-Za-z_-]{35}"),
        r("private-key", "high", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        r(
            "jwt",
            "medium",
            r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        ),
        r(
            "generic-assignment",
            "low",
            r#"(?i)(?:password|passwd|secret|token|api[_-]?key)\s*[:=]\s*["']?[^\s"']{8,}"#,
        ),
    ]
});

pub fn scan(text: &str) -> Vec<SecretMatch> {
    let mut found: Vec<SecretMatch> = Vec::new();
    for rule in RULES.iter() {
        for m in rule.re.find_iter(text) {
            if rule.id == "openai-key" && m.as_str().starts_with("sk-ant-") {
                continue;
            }
            found.push(SecretMatch {
                rule: rule.id,
                confidence: rule.confidence,
                offset: m.start(),
                length: m.len(),
                matched: m.as_str().to_string(),
            });
        }
    }
    let specific: Vec<(usize, usize)> = found
        .iter()
        .filter(|m| m.confidence != "low")
        .map(|m| (m.offset, m.offset + m.length))
        .collect();
    found.retain(|m| {
        m.confidence != "low"
            || !specific
                .iter()
                .any(|&(s, e)| m.offset < e && s < m.offset + m.length)
    });
    found.sort_by_key(|m| (m.offset, m.rule));
    found.dedup_by(|a, b| a.rule == b.rule && a.offset == b.offset);
    found
}
