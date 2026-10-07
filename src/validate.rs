use serde_json::Value;

pub fn violations(schema: &Value, value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk(schema, value, "", &mut out);
    out
}

fn walk(schema: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
    let here = if path.is_empty() { "$" } else { path };
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let Some(map) = value.as_object() else {
                out.push(format!("{here}: expected object"));
                return;
            };
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let props = schema.get("properties").and_then(Value::as_object);
            for key in &required {
                if !map.contains_key(*key) {
                    out.push(format!("{here}.{key}: missing"));
                }
            }
            if let Some(props) = props {
                for (key, child) in props {
                    if let Some(v) = map.get(key) {
                        walk(child, v, &format!("{path}.{key}"), out);
                    }
                }
            }
        }
        Some("array") => {
            let Some(items) = value.as_array() else {
                out.push(format!("{here}: expected array"));
                return;
            };
            if let Some(item_schema) = schema.get("items") {
                for (i, item) in items.iter().enumerate() {
                    walk(item_schema, item, &format!("{path}[{i}]"), out);
                }
            }
        }
        Some("string") if !value.is_string() => out.push(format!("{here}: expected string")),
        Some("number") if !value.is_number() => out.push(format!("{here}: expected number")),
        _ => {}
    }
}

pub fn drop_nulls(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                drop_nulls(v);
            }
        }
        Value::Array(items) => {
            for v in items.iter_mut() {
                drop_nulls(v);
            }
        }
        _ => {}
    }
}

pub fn extract_json_object(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if v.is_object() {
            return Some(v);
        }
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Value>(&trimmed[start..=end]).ok().filter(Value::is_object)
}
