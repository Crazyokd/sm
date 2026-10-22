use crate::wire::truncate;
use regex::{Captures, Regex};
use serde_json::{Map, Value};
use std::sync::LazyLock;
static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)password|passwd|api.?key|authorization|token$|secret|cookie|credential|private.?key",
    )
    .expect("fixed credential regex")
});
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:glpat-|sk-|gh[pousr]_|github_pat_)[A-Za-z0-9_-]{12,}")
        .expect("fixed token regex")
});
static AUTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(bearer|basic)\s+[A-Za-z0-9._~+/=-]+").expect("fixed authorization regex")
});
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(\b[a-z0-9_.-]*(?:api[_-]?key|password|passwd|token|secret|private[_-]?key|cookie|authorization)[a-z0-9_.-]*\b["']?\s*[:=]\s*)("[^"\n]*"|'[^'\n]*'|[^\s,;}&]+)"#).expect("fixed assignment regex")
});
static URL_AUTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(https?://)[^/\s@]+:[^/\s@]+@").expect("fixed URL credential regex")
});
fn policy_flag(key: &str, value: &str) -> bool {
    key.eq_ignore_ascii_case("HERMES_REDACT_SECRETS") && matches!(value, "true" | "false")
}
pub fn redact_text(text: &str, limit: Option<usize>) -> String {
    let value = AUTH.replace_all(text, "$1 [redacted]");
    let value = TOKEN.replace_all(&value, "[redacted]");
    let value = ASSIGNMENT.replace_all(&value, |captures: &Captures<'_>| {
        let raw = captures[2].trim_matches(['\'', '"']);
        let key =
            captures[1].trim_end_matches(|c: char| c.is_ascii_whitespace() || "=:\"'".contains(c));
        if raw.starts_with(['$', '<']) || raw.starts_with("[redacted]") || policy_flag(key, raw) {
            captures[0].to_owned()
        } else {
            format!("{}\"[redacted]\"", &captures[1])
        }
    });
    let value = URL_AUTH.replace_all(&value, "${1}[redacted]@");
    limit.map_or_else(|| value.to_string(), |limit| truncate(&value, limit))
}
pub fn redact(value: &Value, limit: Option<usize>) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if KEY.is_match(k) && !v.as_str().is_some_and(|value| policy_flag(k, value))
                        {
                            Value::String("[redacted]".into())
                        } else {
                            redact(v, limit)
                        },
                    )
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(if limit.is_some() { 100 } else { usize::MAX })
                .map(|v| redact(v, limit))
                .collect(),
        ),
        Value::String(text) => Value::String(redact_text(text, limit)),
        other => other.clone(),
    }
}
