//! Environment variable interpolation syntax rewrites across tools.
//!
//! Canonical IR form uses Claude-style `${VAR}` / `${VAR:-default}`.

use std::collections::BTreeMap;

/// Rewrite all values in a string map with `f`.
pub fn rewrite_map_values(
    map: BTreeMap<String, String>,
    f: fn(&str) -> String,
) -> BTreeMap<String, String> {
    map.into_iter().map(|(k, v)| (k, f(&v))).collect()
}

/// `${env:VAR}` → `${VAR}` (Cursor → IR/Claude).
pub fn rewrite_env_cursor_to_claude(input: &str) -> String {
    rewrite_placeholders(input, |inner| {
        if let Some(rest) = inner.strip_prefix("env:") {
            format!("${{{rest}}}")
        } else {
            format!("${{{inner}}}")
        }
    })
}

/// `${VAR}` → `${env:VAR}` (IR/Claude → Cursor). Keep `${VAR:-default}` as `${env:VAR:-default}`.
pub fn rewrite_env_claude_to_cursor(input: &str) -> String {
    rewrite_placeholders(input, |inner| {
        if inner.starts_with("env:") {
            format!("${{{inner}}}")
        } else {
            format!("${{env:{inner}}}")
        }
    })
}

/// `{env:VAR}` → `${VAR}` (OpenCode → IR/Claude).
pub fn rewrite_env_opencode_to_claude(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 5 < bytes.len() && &input[i..i + 5] == "{env:" {
            if let Some(end) = input[i + 5..].find('}') {
                let name = &input[i + 5..i + 5 + end];
                out.push_str("${");
                out.push_str(name);
                out.push('}');
                i = i + 5 + end + 1;
                continue;
            }
        }
        // Also normalize ${env:VAR} if present.
        out.push(bytes[i] as char);
        i += 1;
    }
    rewrite_env_cursor_to_claude(&out)
}

/// `${VAR}` → `{env:VAR}` (IR/Claude → OpenCode).
pub fn rewrite_env_claude_to_opencode(input: &str) -> String {
    rewrite_placeholders(input, |inner| {
        let name = inner.strip_prefix("env:").unwrap_or(inner);
        // OpenCode does not document :- defaults; keep name only before :-
        let name = name.split(":-").next().unwrap_or(name);
        format!("{{env:{name}}}")
    })
}

fn rewrite_placeholders(input: &str, replace: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end_rel) = input[i + 2..].find('}') {
                let inner = &input[i + 2..i + 2 + end_rel];
                out.push_str(&replace(inner));
                i = i + 2 + end_rel + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let s = "Bearer ${API_KEY}";
        let cursor = rewrite_env_claude_to_cursor(s);
        assert_eq!(cursor, "Bearer ${env:API_KEY}");
        assert_eq!(rewrite_env_cursor_to_claude(&cursor), s);
    }

    #[test]
    fn opencode_roundtrip() {
        let s = "Bearer ${TOKEN}";
        let open = rewrite_env_claude_to_opencode(s);
        assert_eq!(open, "Bearer {env:TOKEN}");
        assert_eq!(rewrite_env_opencode_to_claude(&open), s);
    }

    #[test]
    fn default_syntax_to_opencode_strips_default() {
        let s = "${VAR:-fallback}";
        assert_eq!(rewrite_env_claude_to_opencode(s), "{env:VAR}");
    }
}
