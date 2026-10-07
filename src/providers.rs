use crate::config::ProviderCfg;
use crate::prompt::{vision_result_schema, JSON_TEMPLATE_INSTRUCTION};
use serde_json::{json, Value};

pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

fn protected_fields(provider: &str) -> &'static [&'static str] {
    match provider {
        "gemini-api" => &["contents", "generationConfig.responseMimeType", "generationConfig.responseJsonSchema"],
        "anthropic" => &["messages", "tools", "tool_choice"],
        _ => &["messages"],
    }
}

fn touches(extra: &Value, dotted: &str) -> bool {
    let mut cur = extra;
    for part in dotted.split('.') {
        match cur.get(part) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    true
}

fn merge(into: &mut Value, extra: &Value) {
    match (into, extra) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                match a.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge(existing, v),
                    _ => {
                        a.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, v) => *slot = v.clone(),
    }
}

fn apply_extra(provider: &str, body: &mut Value, extra: Option<&Value>) -> Result<(), String> {
    let Some(extra) = extra else { return Ok(()) };
    for field in protected_fields(provider) {
        if touches(extra, field) {
            return Err(format!("extraBody may not set {field:?} on {provider}"));
        }
    }
    merge(body, extra);
    Ok(())
}

pub fn build_request(cfg: &ProviderCfg, key: &str, model: &str, mime: &str, b64: &str, prompt: &str) -> Result<HttpRequest, String> {
    let json_header = ("Content-Type".to_string(), "application/json".to_string());
    match cfg.name {
        "gemini-api" => {
            let mut body = json!({
                "contents": [{"parts": [
                    {"inline_data": {"mime_type": mime, "data": b64}},
                    {"text": prompt}
                ]}],
                "generationConfig": {"responseMimeType": "application/json", "responseJsonSchema": vision_result_schema()}
            });
            apply_extra("gemini-api", &mut body, cfg.extra_body.as_ref())?;
            Ok(HttpRequest {
                url: format!("{}/v1beta/models/{}:generateContent", cfg.base_url, model),
                headers: vec![("x-goog-api-key".into(), key.into()), json_header],
                body: body.to_string(),
            })
        }
        "openai" => {
            let mut body = json!({
                "model": model,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": format!("{prompt}\n\n{JSON_TEMPLATE_INSTRUCTION}")},
                    {"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b64}")}}
                ]}]
            });
            if cfg.structured_output {
                body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "vision_result", "strict": false, "schema": vision_result_schema()}});
            }
            apply_extra("openai", &mut body, cfg.extra_body.as_ref())?;
            Ok(HttpRequest {
                url: format!("{}/chat/completions", cfg.base_url),
                headers: vec![("Authorization".into(), format!("Bearer {key}")), json_header],
                body: body.to_string(),
            })
        }
        "anthropic" => {
            if mime == "application/pdf" || mime == "image/bmp" {
                return Err(format!("anthropic does not accept {mime} as an image"));
            }
            let mut body = json!({
                "model": model,
                "max_tokens": 8192,
                "messages": [{"role": "user", "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": mime, "data": b64}},
                    {"type": "text", "text": prompt}
                ]}],
                "tools": [{"name": "report_vision", "description": "Report the structured evidence read from the image.", "input_schema": vision_result_schema()}],
                "tool_choice": {"type": "tool", "name": "report_vision"}
            });
            apply_extra("anthropic", &mut body, cfg.extra_body.as_ref())?;
            Ok(HttpRequest {
                url: format!("{}/v1/messages", cfg.base_url),
                headers: vec![("x-api-key".into(), key.into()), ("anthropic-version".into(), "2023-06-01".into()), json_header],
                body: body.to_string(),
            })
        }
        other => Err(format!("unknown provider {other}")),
    }
}

pub fn parse_response(provider: &str, payload: &Value) -> Result<(Value, Value), String> {
    match provider {
        "gemini-api" => {
            let text: String = payload["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect())
                .unwrap_or_default();
            if text.is_empty() {
                return Err("Gemini API returned no text candidate".into());
            }
            Ok((parse_text(&text)?, payload["usageMetadata"].clone()))
        }
        "openai" => {
            let text = payload["choices"][0]["message"]["content"].as_str().unwrap_or("");
            if text.is_empty() {
                return Err("OpenAI-compatible endpoint returned no message content".into());
            }
            Ok((parse_text(text)?, payload["usage"].clone()))
        }
        _ => {
            let blocks = payload["content"].as_array().cloned().unwrap_or_default();
            if let Some(input) = blocks.iter().find(|b| b["type"] == "tool_use").map(|b| b["input"].clone()) {
                return Ok((input, payload["usage"].clone()));
            }
            let text: String = blocks.iter().filter_map(|b| b["text"].as_str()).collect();
            if text.is_empty() {
                return Err("Anthropic returned neither a tool call nor text".into());
            }
            Ok((parse_text(&text)?, payload["usage"].clone()))
        }
    }
}

fn parse_text(text: &str) -> Result<Value, String> {
    crate::validate::extract_json_object(text)
        .ok_or_else(|| format!("provider returned non-JSON output: {}", crate::redact::truncate(text)))
}

pub fn rotates_to_next_key(status: u64, body: &str) -> bool {
    if matches!(status, 401 | 403 | 429 | 432 | 433) {
        return true;
    }
    let lower = body.to_ascii_lowercase();
    ["quota", "rate limit", "rate_limit", "invalid api key", "api key not valid", "insufficient_quota"]
        .iter()
        .any(|needle| lower.contains(needle))
}
