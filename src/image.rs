pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

pub fn sniff_mime(b64: &str) -> Option<&'static str> {
    const TABLE: [(&str, &str); 6] = [
        ("iVBORw0KGgo", "image/png"),
        ("/9j/", "image/jpeg"),
        ("R0lGOD", "image/gif"),
        ("UklGR", "image/webp"),
        ("JVBER", "application/pdf"),
        ("Qk", "image/bmp"),
    ];
    TABLE.iter().find(|(prefix, _)| b64.starts_with(prefix)).map(|(_, mime)| *mime)
}

pub fn mime_from_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

pub fn supported_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp" | "application/pdf"
    )
}

pub fn host_of(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "url must start with http:// or https://".to_string())?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err("url has an empty host".to_string());
    }
    if authority.contains('@') {
        return Err("url must not carry userinfo".to_string());
    }
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("").to_string()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    if host.is_empty() {
        return Err("url has an empty host".to_string());
    }
    Ok(host.to_ascii_lowercase())
}

/// Resolve a redirect target against the URL that produced it. Only http(s) targets
/// are accepted, so a redirect cannot switch the request to another scheme.
pub fn resolve_location(base: &str, location: &str) -> Result<String, String> {
    let next = if location.starts_with("https://") || location.starts_with("http://") {
        location.to_string()
    } else if let Some(path) = location.strip_prefix("//") {
        let scheme = if base.starts_with("https://") { "https:" } else { "http:" };
        format!("{scheme}//{path}")
    } else if location.starts_with('/') {
        let origin_end = base.find("://").map(|i| i + 3).unwrap_or(0);
        let authority_end = base[origin_end..].find(['/', '?', '#']).map(|i| origin_end + i).unwrap_or(base.len());
        format!("{}{}", &base[..authority_end], location)
    } else {
        return Err(format!("unsupported redirect target {location:?}"));
    };
    host_of(&next)?;
    Ok(next)
}

pub fn check_remote_url(url: &str) -> Result<(), String> {
    let host = host_of(url)?;
    if is_private_host(&host) {
        return Err(format!("refusing to fetch private or local host {host:?}"));
    }
    Ok(())
}

fn is_private_host(host: &str) -> bool {
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || !host.contains('.') && !host.contains(':')
    {
        return true;
    }
    if host.contains(':') {
        let h = host.trim_start_matches("::ffff:");
        return host == "::1"
            || host == "::"
            || h.starts_with("fc")
            || h.starts_with("fd")
            || h.starts_with("fe8")
            || h.starts_with("fe9")
            || h.starts_with("fea")
            || h.starts_with("feb")
            || h.contains('.') && is_private_host(h);
    }
    let parts: Vec<&str> = host.split('.').collect();
    let numericish = parts
        .iter()
        .all(|p| !p.is_empty() && (p.starts_with("0x") || p.chars().all(|c| c.is_ascii_digit())));
    if !numericish {
        return false;
    }
    let decimal_quad = parts.len() == 4 && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()));
    if !decimal_quad {
        return true;
    }
    let o: Vec<u16> = parts.iter().map(|p| p.parse().unwrap_or(999)).collect();
    o.iter().any(|v| *v > 255)
        || o[0] == 10
        || o[0] == 127
        || o[0] == 0
        || o[0] >= 224
        || (o[0] == 169 && o[1] == 254)
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 100 && (64..=127).contains(&o[1]))
}
