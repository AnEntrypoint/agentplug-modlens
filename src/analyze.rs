use serde_json::{json, Value};

use crate::abi;
use crate::config::{self, ProviderCfg, CONFIG_RELATIVE_PATH};
use crate::image;
use crate::prompt;
use crate::providers;
use crate::redact::{mask, redact_secrets, truncate};
use crate::validate;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;

fn load_file_config() -> Option<Value> {
    serde_json::from_str(&abi::read_text(CONFIG_RELATIVE_PATH)?).ok()
}

fn env_lookup(key: &str) -> Option<String> {
    abi::env(key)
}

pub fn capabilities() -> Value {
    json!({
        "ok": true,
        "plugin": "modlens",
        "verbs": ["read_image", "doctor", "capabilities"],
        "description": "Vision bridge for text-only models, after liustack/modlens. read_image takes {path | url | base64+mime, mode?: describe|ocr|ui|chart|diagram|error, prompt?, provider?, pin?, model?, timeoutMs?} and answers structured evidence (summary, ocr, layout, semantics, visual, uncertainty) read by a multimodal API provider (gemini-api, openai-compatible, anthropic). Providers come from .gm/modlens.json or env keys; doctor lists what is configured.",
        "modes": ["describe", "ocr", "ui", "chart", "diagram", "error"],
        "providers": config::PROVIDER_ORDER,
        "config_path": CONFIG_RELATIVE_PATH,
    })
}

pub fn doctor(body: &Value) -> Value {
    let file = load_file_config();
    let chain = config::chain(file.as_ref(), &env_lookup, body.get("provider").and_then(Value::as_str));
    let entries: Vec<Value> = chain
        .iter()
        .map(|c| {
            json!({
                "provider": c.name,
                "configured": !c.api_keys.is_empty() && (c.name != "openai" || c.effective_model().is_some()),
                "source": c.source,
                "keys": c.api_keys.iter().map(|k| mask(k)).collect::<Vec<_>>(),
                "baseUrl": c.base_url,
                "model": c.effective_model(),
                "missing": missing_for(c),
            })
        })
        .collect();
    let ready = entries.iter().any(|e| e["configured"] == true);
    json!({
        "ok": true,
        "ready": ready,
        "config_file_present": file.is_some(),
        "config_path": CONFIG_RELATIVE_PATH,
        "chain": entries,
        "hint": if ready { Value::Null } else { json!("No vision engine configured. Set GEMINI_API_KEY (free key at https://aistudio.google.com), or OPENAI_API_KEY + OPENAI_BASE_URL + OPENAI_MODEL, or ANTHROPIC_API_KEY, or write .gm/modlens.json (keep it out of git).") },
    })
}

fn missing_for(c: &ProviderCfg) -> Vec<&'static str> {
    let mut m = Vec::new();
    if c.api_keys.is_empty() {
        m.push("apiKey");
    }
    if c.name == "openai" && c.effective_model().is_none() {
        m.push("model");
    }
    m
}

struct Loaded {
    mime: String,
    b64: String,
    label: String,
}

fn load_image(body: &Value, timeout_ms: u64) -> Result<Loaded, String> {
    let get = |k: &str| body.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    if let Some(data) = get("base64") {
        let data = data.rsplit(',').next().unwrap_or(data).to_string();
        let mime = get("mime")
            .map(String::from)
            .or_else(|| image::sniff_mime(&data).map(String::from))
            .ok_or("base64 input needs a mime field or a recognisable image header")?;
        return finish(mime, data, "inline".into());
    }
    if let Some(path) = get("path").or_else(|| get("image")).filter(|p| !p.starts_with("http://") && !p.starts_with("https://")) {
        let data = abi::read_base64(path).ok_or_else(|| format!("could not read image {path:?} (missing, empty, over 20 MB, or outside the project sandbox)"))?;
        let mime = image::sniff_mime(&data).map(String::from).or_else(|| image::mime_from_extension(path).map(String::from)).unwrap_or_default();
        return finish(mime, data, path.to_string());
    }
    let url = get("url").or_else(|| get("image")).ok_or("read_image needs one of: path, url, base64")?;
    image::check_remote_url(url)?;
    let resp = abi::fetch(url, &json!({"method": "GET", "responseEncoding": "base64", "timeoutMs": timeout_ms}));
    if resp["ok"] != true {
        return Err(format!("image download failed: status {} {}", resp["status"], resp["error"].as_str().unwrap_or("")));
    }
    let data = resp["body"].as_str().unwrap_or("").to_string();
    let mime = image::sniff_mime(&data).map(String::from).unwrap_or_default();
    finish(mime, data, url.to_string())
}

fn finish(mime: String, b64: String, label: String) -> Result<Loaded, String> {
    if b64.is_empty() {
        return Err("image is empty".into());
    }
    if !image::supported_mime(&mime) {
        return Err(format!("unsupported or unrecognised image type {mime:?}; supported: png, jpeg, gif, webp, bmp, pdf"));
    }
    if b64.len() / 4 * 3 > image::MAX_IMAGE_BYTES {
        return Err("image exceeds the 20 MB limit".into());
    }
    Ok(Loaded { mime, b64, label })
}

pub fn read_image(body: &Value) -> Value {
    let started = abi::now_ms();
    let mode = body.get("mode").and_then(Value::as_str).unwrap_or("describe");
    if !prompt::known_mode(mode) {
        return json!({"ok": false, "error": "unknown_mode", "mode": mode, "modes": ["describe", "ocr", "ui", "chart", "diagram", "error"]});
    }
    let timeout_ms = body.get("timeoutMs").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT_MS).clamp(1_000, 600_000);
    let image = match load_image(body, timeout_ms) {
        Ok(i) => i,
        Err(e) => return json!({"ok": false, "error": "image_unavailable", "reason": e}),
    };
    let file = load_file_config();
    let requested = body.get("provider").and_then(Value::as_str);
    if let Some(p) = requested {
        if config::canonical_provider(p).is_none() {
            return json!({"ok": false, "error": "unknown_provider", "provider": p, "providers": config::PROVIDER_ORDER});
        }
    }
    let mut chain = config::chain(file.as_ref(), &env_lookup, requested);
    if body.get("pin").and_then(Value::as_bool).unwrap_or(false) {
        chain.truncate(1);
    }
    let model_override = body.get("model").and_then(Value::as_str).map(String::from);
    let prompt_text = prompt::build_vision_prompt(mode, body.get("prompt").and_then(Value::as_str));
    let schema = prompt::vision_result_schema();
    let mut attempts: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for cfg in &chain {
        let Some(model) = model_override.clone().filter(|_| requested.and_then(config::canonical_provider) == Some(cfg.name)).or_else(|| cfg.effective_model()) else {
            attempts.push(json!({"provider": cfg.name, "ok": false, "error": "no model configured"}));
            continue;
        };
        if cfg.api_keys.is_empty() {
            attempts.push(json!({"provider": cfg.name, "ok": false, "error": "not configured"}));
            continue;
        }
        let secrets = cfg.api_keys.clone();
        for (key_index, key) in cfg.api_keys.iter().enumerate() {
            let t0 = abi::now_ms();
            let outcome = attempt(cfg, key, &model, &image, &prompt_text, &schema, timeout_ms, &secrets);
            let took = abi::now_ms().saturating_sub(t0) as f64 / 1000.0;
            match outcome {
                Ok((result, usage)) => {
                    attempts.push(json!({"provider": cfg.name, "ok": true, "durationSeconds": took}));
                    if key_index > 0 {
                        warnings.push(format!("{} rotated to key #{} after earlier keys failed", cfg.name, key_index + 1));
                    }
                    return json!({
                        "ok": true,
                        "image": image.label,
                        "provider": cfg.name,
                        "mode": mode,
                        "untrusted_content": true,
                        "result": result,
                        "meta": {
                            "model": model,
                            "durationSeconds": (abi::now_ms().saturating_sub(started)) as f64 / 1000.0,
                            "usage": usage,
                            "attempts": attempts,
                            "warnings": warnings,
                        }
                    });
                }
                Err((rotate, message)) => {
                    attempts.push(json!({"provider": cfg.name, "ok": false, "durationSeconds": took, "error": message}));
                    if !rotate {
                        break;
                    }
                }
            }
        }
    }
    json!({
        "ok": false,
        "error": "all_providers_failed",
        "reason": "no configured vision engine produced a valid result",
        "meta": {"attempts": attempts, "warnings": warnings},
        "hint": "dispatch modlens {\"verb\":\"doctor\"} to see what is configured",
    })
}

#[allow(clippy::too_many_arguments)]
fn attempt(cfg: &ProviderCfg, key: &str, model: &str, image: &Loaded, prompt_text: &str, schema: &Value, timeout_ms: u64, secrets: &[String]) -> Result<(Value, Value), (bool, String)> {
    let request = providers::build_request(cfg, key, model, &image.mime, &image.b64, prompt_text).map_err(|e| (false, e))?;
    let headers: serde_json::Map<String, Value> = request.headers.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    let resp = abi::fetch(&request.url, &json!({"method": "POST", "headers": headers, "body": request.body, "timeoutMs": timeout_ms}));
    let status = resp["status"].as_u64().unwrap_or(0);
    let text = resp["body"].as_str().unwrap_or("");
    if resp["ok"] != true {
        let detail = redact_secrets(&format!("{} {}", resp["error"].as_str().unwrap_or(""), text), secrets);
        return Err((providers::rotates_to_next_key(status, text), format!("{} API error {status}: {}", cfg.name, truncate(&detail))));
    }
    let payload: Value = serde_json::from_str(text).map_err(|_| (false, format!("{} returned a non-JSON body", cfg.name)))?;
    let (mut result, usage) = providers::parse_response(cfg.name, &payload).map_err(|e| (false, redact_secrets(&e, secrets)))?;
    validate::drop_nulls(&mut result);
    let problems = validate::violations(schema, &result);
    if !problems.is_empty() {
        return Err((false, format!("{} result violates the vision contract: {}", cfg.name, problems.iter().take(6).cloned().collect::<Vec<_>>().join("; "))));
    }
    Ok((result, usage))
}
