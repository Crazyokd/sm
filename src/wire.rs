use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
// Existing proposal keys and output hashes used JSON with Python's default separators.
pub fn legacy_json(value: &Value) -> Result<String> {
    match value {
        Value::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(legacy_json)
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        )),
        Value::Object(values) => Ok(format!(
            "{{{}}}",
            values
                .iter()
                .map(|(k, v)| Ok(format!(
                    "{}: {}",
                    serde_json::to_string(k)?,
                    legacy_json(v)?
                )))
                .collect::<Result<Vec<_>>>()?
                .join(", ")
        )),
        scalar => Ok(serde_json::to_string(scalar)?),
    }
}
pub fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}
