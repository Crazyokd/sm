use sha2::{Digest, Sha256};

pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
pub fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}
