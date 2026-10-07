use serde_json::Value;

pub const GEMINI_DEFAULT_MODEL: &str = "gemini-3.6-flash";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";
pub const GEMINI_DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";
pub const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com/v1";
pub const ANTHROPIC_DEFAULT_BASE: &str = "https://api.anthropic.com";
pub const CONFIG_RELATIVE_PATH: &str = ".gm/modlens.json";
pub const PROVIDER_ORDER: [&str; 3] = ["gemini-api", "openai", "anthropic"];

#[derive(Clone, Debug)]
pub struct ProviderCfg {
    pub name: &'static str,
    pub api_keys: Vec<String>,
    pub base_url: String,
    pub model: Option<String>,
    pub extra_body: Option<Value>,
    pub structured_output: bool,
    pub source: &'static str,
}

impl ProviderCfg {
    pub fn effective_model(&self) -> Option<String> {
        self.model.clone().or_else(|| match self.name {
            "gemini-api" => Some(GEMINI_DEFAULT_MODEL.to_string()),
            "anthropic" => Some(ANTHROPIC_DEFAULT_MODEL.to_string()),
            _ => None,
        })
    }
}

pub fn canonical_provider(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "gemini-api" | "gemini" => Some("gemini-api"),
        "openai" | "openai-compat" => Some("openai"),
        "anthropic" | "claude" => Some("anthropic"),
        _ => None,
    }
}

pub fn split_keys(raw: &str) -> Vec<String> {
    raw.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

fn nonempty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn file_entry<'a>(file: &'a Value, name: &str) -> Option<&'a Value> {
    let providers = file.get("providers")?.as_object()?;
    let aliases: &[&str] = match name {
        "gemini-api" => &["gemini-api", "gemini"],
        "openai" => &["openai", "openai-compat"],
        _ => &["anthropic", "claude"],
    };
    aliases.iter().find_map(|a| providers.get(*a)).filter(|v| v.is_object())
}

pub fn resolve_provider(name: &'static str, file: Option<&Value>, env: &dyn Fn(&str) -> Option<String>) -> ProviderCfg {
    let default_base = match name {
        "gemini-api" => GEMINI_DEFAULT_BASE,
        "openai" => OPENAI_DEFAULT_BASE,
        _ => ANTHROPIC_DEFAULT_BASE,
    };
    if let Some(entry) = file.and_then(|f| file_entry(f, name)) {
        let str_field = |k: &str| nonempty(entry.get(k).and_then(Value::as_str).map(String::from));
        return ProviderCfg {
            name,
            api_keys: str_field("apiKey").map(|k| split_keys(&k)).unwrap_or_default(),
            base_url: str_field("baseUrl").unwrap_or_else(|| default_base.to_string()).trim_end_matches('/').to_string(),
            model: str_field("model"),
            extra_body: entry.get("extraBody").filter(|v| v.is_object()).cloned(),
            structured_output: entry.get("structuredOutput").and_then(Value::as_bool).unwrap_or(false),
            source: "file",
        };
    }
    let first = |keys: &[&str]| keys.iter().find_map(|k| nonempty(env(k)));
    let (key_vars, base_vars, model_vars): (&[&str], &[&str], &[&str]) = match name {
        "gemini-api" => (&["GEMINI_API_KEY", "GOOGLE_API_KEY"], &["GEMINI_BASE_URL"], &["GEMINI_MODEL", "MODLENS_GEMINI_MODEL"]),
        "openai" => (&["OPENAI_API_KEY"], &["OPENAI_BASE_URL"], &["OPENAI_MODEL", "MODLENS_OPENAI_MODEL"]),
        _ => (&["ANTHROPIC_API_KEY"], &["ANTHROPIC_BASE_URL"], &["ANTHROPIC_MODEL", "MODLENS_ANTHROPIC_MODEL"]),
    };
    ProviderCfg {
        name,
        api_keys: first(key_vars).map(|k| split_keys(&k)).unwrap_or_default(),
        base_url: first(base_vars).unwrap_or_else(|| default_base.to_string()).trim_end_matches('/').to_string(),
        model: first(model_vars),
        extra_body: None,
        structured_output: false,
        source: "env",
    }
}

pub fn chain(file: Option<&Value>, env: &dyn Fn(&str) -> Option<String>, preferred: Option<&str>) -> Vec<ProviderCfg> {
    let preferred = preferred
        .map(String::from)
        .or_else(|| nonempty(file.and_then(|f| f.get("provider")).and_then(Value::as_str).map(String::from)))
        .and_then(|p| canonical_provider(&p));
    let mut names: Vec<&'static str> = PROVIDER_ORDER.to_vec();
    if let Some(p) = preferred {
        names.retain(|n| *n != p);
        names.insert(0, p);
    }
    names.into_iter().map(|n| resolve_provider(n, file, env)).collect()
}
