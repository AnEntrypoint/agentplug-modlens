pub const REDACTED: &str = "[redacted]";
const MAX_DETAIL_CHARS: usize = 600;

pub fn redact_secrets(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= 6 {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    out
}

pub fn truncate(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_DETAIL_CHARS {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(MAX_DETAIL_CHARS).collect();
    format!("{cut}...")
}

pub fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 8 {
        return "****".to_string();
    }
    let head: String = key.chars().take(4).collect();
    let tail: String = key.chars().skip(n - 4).collect();
    format!("{head}...{tail}")
}
