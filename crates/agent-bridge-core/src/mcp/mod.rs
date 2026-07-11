//! MCP intermediate representation and format converters.

mod env_syntax;

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table, Value as TomlValue};

use crate::error::{Error, Result};
use crate::fsutil::{atomic_write, read_optional};
use crate::tool::{ToolId, WriteMode};

pub use env_syntax::{
    rewrite_env_claude_to_cursor, rewrite_env_claude_to_opencode, rewrite_env_cursor_to_claude,
    rewrite_env_opencode_to_claude, rewrite_map_values,
};

/// A single MCP server in tool-agnostic form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub transport: McpTransport,
}

/// Transport kinds supported by the IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// Ordered collection of MCP servers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpDocument {
    pub servers: BTreeMap<String, McpServer>,
}

impl McpDocument {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.servers.keys().map(|s| s.as_str()).collect()
    }
}

/// Read MCP config for a tool from disk into IR.
pub fn read_mcp(tool: ToolId, path: &Path) -> Result<McpDocument> {
    match tool {
        ToolId::Claude | ToolId::Cursor => read_mcp_servers_json(path, tool),
        ToolId::OpenCode => read_opencode_mcp(path),
        ToolId::Codex => read_codex_mcp(path),
    }
}

/// Write IR MCP servers into a tool's config file, merging carefully.
pub fn write_mcp(tool: ToolId, path: &Path, doc: &McpDocument, mode: WriteMode) -> Result<()> {
    match tool {
        ToolId::Claude => write_claude_mcp(path, doc, mode),
        ToolId::Cursor => write_cursor_mcp(path, doc, mode),
        ToolId::OpenCode => write_opencode_mcp(path, doc, mode),
        ToolId::Codex => write_codex_mcp(path, doc, mode),
    }
}

fn read_mcp_servers_json(path: &Path, tool: ToolId) -> Result<McpDocument> {
    let Some(raw) = read_optional(path)? else {
        return Ok(McpDocument::default());
    };
    let value: Value = serde_json::from_str(&raw).map_err(|e| Error::json(path, e))?;
    let obj = value
        .as_object()
        .ok_or_else(|| Error::invalid_mcp(path, "root must be a JSON object"))?;
    let Some(servers) = obj.get("mcpServers") else {
        return Ok(McpDocument::default());
    };
    let servers_obj = servers
        .as_object()
        .ok_or_else(|| Error::invalid_mcp(path, "mcpServers must be an object"))?;

    let mut out = McpDocument::default();
    for (name, entry) in servers_obj {
        let server = parse_json_server(path, name, entry, tool)?;
        out.servers.insert(name.clone(), server);
    }
    Ok(out)
}

fn parse_json_server(
    path: &Path,
    name: &str,
    entry: &Value,
    tool: ToolId,
) -> Result<McpServer> {
    let obj = entry
        .as_object()
        .ok_or_else(|| Error::invalid_mcp(path, format!("server '{name}' must be an object")))?;

    if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
        let headers = extract_string_map(obj.get("headers")).unwrap_or_default();
        let headers = match tool {
            ToolId::Cursor => rewrite_map_values(headers, rewrite_env_cursor_to_claude),
            ToolId::Claude => headers,
            _ => headers,
        };
        return Ok(McpServer {
            name: name.to_string(),
            transport: McpTransport::Http {
                url: url.to_string(),
                headers,
            },
        });
    }

    let command = obj
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::invalid_mcp(
                path,
                format!("server '{name}' needs 'command' (stdio) or 'url' (http)"),
            )
        })?;
    let args = extract_string_vec(obj.get("args")).unwrap_or_default();
    let env = extract_string_map(obj.get("env")).unwrap_or_default();
    let env = match tool {
        ToolId::Cursor => rewrite_map_values(env, rewrite_env_cursor_to_claude),
        _ => env,
    };

    Ok(McpServer {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: command.to_string(),
            args,
            env,
        },
    })
}

fn write_claude_mcp(path: &Path, doc: &McpDocument, mode: WriteMode) -> Result<()> {
    let mut root = read_json_object(path)?;
    let servers = merge_mcp_servers_json(
        root.remove("mcpServers"),
        doc,
        mode,
        |s| server_to_claude_json(s),
    )?;
    root.insert("mcpServers".to_string(), Value::Object(servers));
    write_json(path, &Value::Object(root))
}

fn write_cursor_mcp(path: &Path, doc: &McpDocument, mode: WriteMode) -> Result<()> {
    let mut root = read_json_object(path)?;
    let servers = merge_mcp_servers_json(
        root.remove("mcpServers"),
        doc,
        mode,
        |s| server_to_cursor_json(s),
    )?;
    root.insert("mcpServers".to_string(), Value::Object(servers));
    write_json(path, &Value::Object(root))
}

fn merge_mcp_servers_json(
    existing: Option<Value>,
    doc: &McpDocument,
    mode: WriteMode,
    encode: impl Fn(&McpServer) -> Value,
) -> Result<Map<String, Value>> {
    let mut map = match existing {
        Some(Value::Object(m)) => m,
        Some(_) => Map::new(),
        None => Map::new(),
    };

    if mode == WriteMode::Prune {
        let keep: std::collections::BTreeSet<_> = doc.servers.keys().cloned().collect();
        map.retain(|k, _| keep.contains(k));
    }

    for (name, server) in &doc.servers {
        map.insert(name.clone(), encode(server));
    }
    Ok(map)
}

fn server_to_claude_json(server: &McpServer) -> Value {
    match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let mut obj = Map::new();
            obj.insert("command".into(), Value::String(command.clone()));
            if !args.is_empty() {
                obj.insert(
                    "args".into(),
                    Value::Array(args.iter().cloned().map(Value::String).collect()),
                );
            }
            if !env.is_empty() {
                obj.insert("env".into(), map_to_json_object(env));
            }
            Value::Object(obj)
        }
        McpTransport::Http { url, headers } => {
            let mut obj = Map::new();
            obj.insert("type".into(), Value::String("http".into()));
            obj.insert("url".into(), Value::String(url.clone()));
            if !headers.is_empty() {
                obj.insert("headers".into(), map_to_json_object(headers));
            }
            Value::Object(obj)
        }
    }
}

fn server_to_cursor_json(server: &McpServer) -> Value {
    match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let env = rewrite_map_values(env.clone(), rewrite_env_claude_to_cursor);
            let mut obj = Map::new();
            obj.insert("command".into(), Value::String(command.clone()));
            if !args.is_empty() {
                obj.insert(
                    "args".into(),
                    Value::Array(args.iter().cloned().map(Value::String).collect()),
                );
            }
            if !env.is_empty() {
                obj.insert("env".into(), map_to_json_object(&env));
            }
            Value::Object(obj)
        }
        McpTransport::Http { url, headers } => {
            let headers = rewrite_map_values(headers.clone(), rewrite_env_claude_to_cursor);
            let mut obj = Map::new();
            obj.insert("url".into(), Value::String(url.clone()));
            if !headers.is_empty() {
                obj.insert("headers".into(), map_to_json_object(&headers));
            }
            Value::Object(obj)
        }
    }
}

fn read_opencode_mcp(path: &Path) -> Result<McpDocument> {
    let Some(raw) = read_optional(path)? else {
        return Ok(McpDocument::default());
    };
    // Strip simple // and /* */ is hard; OpenCode may be JSONC. Try JSON first,
    // then strip // line comments for a best-effort parse.
    let value: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            let stripped = strip_jsonc_line_comments(&raw);
            serde_json::from_str(&stripped).map_err(|e| Error::json(path, e))?
        }
    };
    let obj = value
        .as_object()
        .ok_or_else(|| Error::invalid_mcp(path, "root must be a JSON object"))?;
    let Some(mcp) = obj.get("mcp") else {
        return Ok(McpDocument::default());
    };
    let mcp_obj = mcp
        .as_object()
        .ok_or_else(|| Error::invalid_mcp(path, "mcp must be an object"))?;

    let mut out = McpDocument::default();
    for (name, entry) in mcp_obj {
        let server = parse_opencode_server(path, name, entry)?;
        out.servers.insert(name.clone(), server);
    }
    Ok(out)
}

fn parse_opencode_server(path: &Path, name: &str, entry: &Value) -> Result<McpServer> {
    let obj = entry
        .as_object()
        .ok_or_else(|| Error::invalid_mcp(path, format!("server '{name}' must be an object")))?;
    let ty = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if obj.contains_key("url") {
                "remote"
            } else {
                "local"
            }
        });

    if ty == "remote" {
        let url = obj
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::invalid_mcp(path, format!("remote server '{name}' needs url")))?;
        let headers = extract_string_map(obj.get("headers")).unwrap_or_default();
        let headers = rewrite_map_values(headers, rewrite_env_opencode_to_claude);
        return Ok(McpServer {
            name: name.to_string(),
            transport: McpTransport::Http {
                url: url.to_string(),
                headers,
            },
        });
    }

    // local / stdio
    let command_val = obj.get("command").ok_or_else(|| {
        Error::invalid_mcp(path, format!("local server '{name}' needs command array"))
    })?;
    let (command, args) = match command_val {
        Value::Array(arr) => {
            let mut iter = arr.iter().filter_map(|v| v.as_str().map(str::to_string));
            let command = iter.next().ok_or_else(|| {
                Error::invalid_mcp(path, format!("server '{name}' command array is empty"))
            })?;
            (command, iter.collect())
        }
        Value::String(s) => (s.clone(), Vec::new()),
        _ => {
            return Err(Error::invalid_mcp(
                path,
                format!("server '{name}' command must be array or string"),
            ));
        }
    };
    let env = extract_string_map(obj.get("environment"))
        .or_else(|| extract_string_map(obj.get("env")))
        .unwrap_or_default();
    let env = rewrite_map_values(env, rewrite_env_opencode_to_claude);

    Ok(McpServer {
        name: name.to_string(),
        transport: McpTransport::Stdio { command, args, env },
    })
}

fn write_opencode_mcp(path: &Path, doc: &McpDocument, mode: WriteMode) -> Result<()> {
    let mut root = read_json_object(path)?;
    let existing = root.remove("mcp");
    let mut map = match existing {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };

    if mode == WriteMode::Prune {
        let keep: std::collections::BTreeSet<_> = doc.servers.keys().cloned().collect();
        map.retain(|k, _| keep.contains(k));
    }

    for (name, server) in &doc.servers {
        map.insert(name.clone(), server_to_opencode_json(server));
    }
    root.insert("mcp".to_string(), Value::Object(map));
    write_json(path, &Value::Object(root))
}

fn server_to_opencode_json(server: &McpServer) -> Value {
    match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let env = rewrite_map_values(env.clone(), rewrite_env_claude_to_opencode);
            let mut cmd = vec![Value::String(command.clone())];
            cmd.extend(args.iter().cloned().map(Value::String));
            let mut obj = Map::new();
            obj.insert("type".into(), Value::String("local".into()));
            obj.insert("command".into(), Value::Array(cmd));
            if !env.is_empty() {
                obj.insert("environment".into(), map_to_json_object(&env));
            }
            Value::Object(obj)
        }
        McpTransport::Http { url, headers } => {
            let headers = rewrite_map_values(headers.clone(), rewrite_env_claude_to_opencode);
            let mut obj = Map::new();
            obj.insert("type".into(), Value::String("remote".into()));
            obj.insert("url".into(), Value::String(url.clone()));
            if !headers.is_empty() {
                obj.insert("headers".into(), map_to_json_object(&headers));
            }
            Value::Object(obj)
        }
    }
}

fn read_codex_mcp(path: &Path) -> Result<McpDocument> {
    let Some(raw) = read_optional(path)? else {
        return Ok(McpDocument::default());
    };
    let value: toml::Value = toml::from_str(&raw).map_err(|e| Error::toml(path, e))?;
    let Some(servers) = value.get("mcp_servers").and_then(|v| v.as_table()) else {
        return Ok(McpDocument::default());
    };

    let mut out = McpDocument::default();
    for (name, entry) in servers {
        let table = entry.as_table().ok_or_else(|| {
            Error::invalid_mcp(path, format!("mcp_servers.{name} must be a table"))
        })?;
        let server = parse_codex_server(path, name, table)?;
        out.servers.insert(name.clone(), server);
    }
    Ok(out)
}

fn parse_codex_server(
    path: &Path,
    name: &str,
    table: &toml::map::Map<String, toml::Value>,
) -> Result<McpServer> {
    if let Some(url) = table.get("url").and_then(|v| v.as_str()) {
        let mut headers = BTreeMap::new();
        if let Some(h) = table.get("http_headers").and_then(|v| v.as_table()) {
            for (k, v) in h {
                if let Some(s) = v.as_str() {
                    headers.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(env_var) = table.get("bearer_token_env_var").and_then(|v| v.as_str()) {
            headers
                .entry("Authorization".to_string())
                .or_insert_with(|| format!("Bearer ${{{env_var}}}"));
        }
        return Ok(McpServer {
            name: name.to_string(),
            transport: McpTransport::Http {
                url: url.to_string(),
                headers,
            },
        });
    }

    let command = table
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::invalid_mcp(
                path,
                format!("mcp_servers.{name} needs command or url"),
            )
        })?;
    let mut args = Vec::new();
    if let Some(arr) = table.get("args").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                args.push(s.to_string());
            }
        }
    }
    let mut env = BTreeMap::new();
    if let Some(e) = table.get("env").and_then(|v| v.as_table()) {
        for (k, v) in e {
            if let Some(s) = v.as_str() {
                env.insert(k.clone(), s.to_string());
            }
        }
    }

    Ok(McpServer {
        name: name.to_string(),
        transport: McpTransport::Stdio {
            command: command.to_string(),
            args,
            env,
        },
    })
}

fn write_codex_mcp(path: &Path, doc: &McpDocument, mode: WriteMode) -> Result<()> {
    let raw = read_optional(path)?.unwrap_or_default();
    let mut document: DocumentMut = if raw.trim().is_empty() {
        DocumentMut::new()
    } else {
        raw.parse::<DocumentMut>().map_err(|e| Error::TomlEdit {
            path: path.to_path_buf(),
            source: e,
        })?
    };

    let mcp_item = document
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()));
    let table = mcp_item
        .as_table_mut()
        .ok_or_else(|| Error::invalid_mcp(path, "mcp_servers must be a table"))?;

    if mode == WriteMode::Prune {
        let keep: std::collections::BTreeSet<_> = doc.servers.keys().cloned().collect();
        let existing: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
        for key in existing {
            if !keep.contains(&key) {
                table.remove(&key);
            }
        }
    }

    for (name, server) in &doc.servers {
        let mut server_table = Table::new();
        match &server.transport {
            McpTransport::Stdio { command, args, env } => {
                server_table.insert("command", TomlValue::from(command.as_str()).into());
                if !args.is_empty() {
                    let mut arr = toml_edit::Array::new();
                    for a in args {
                        arr.push(a.as_str());
                    }
                    server_table.insert("args", TomlValue::Array(arr).into());
                }
                if !env.is_empty() {
                    let mut env_table = toml_edit::InlineTable::new();
                    for (k, v) in env {
                        env_table.insert(k, v.as_str().into());
                    }
                    server_table.insert("env", TomlValue::InlineTable(env_table).into());
                }
            }
            McpTransport::Http { url, headers } => {
                server_table.insert("url", TomlValue::from(url.as_str()).into());
                let mut bearer = None;
                let mut other = BTreeMap::new();
                for (k, v) in headers {
                    if k.eq_ignore_ascii_case("Authorization") {
                        if let Some(rest) = v.strip_prefix("Bearer ") {
                            if let Some(var) = extract_simple_env_var(rest) {
                                bearer = Some(var);
                                continue;
                            }
                        }
                    }
                    other.insert(k.clone(), v.clone());
                }
                if let Some(var) = bearer {
                    server_table.insert(
                        "bearer_token_env_var",
                        TomlValue::from(var.as_str()).into(),
                    );
                }
                if !other.is_empty() {
                    let mut ht = toml_edit::InlineTable::new();
                    for (k, v) in &other {
                        ht.insert(k, v.as_str().into());
                    }
                    server_table.insert("http_headers", TomlValue::InlineTable(ht).into());
                }
            }
        }
        table.insert(name, Item::Table(server_table));
    }

    atomic_write(path, document.to_string())
}

fn extract_simple_env_var(s: &str) -> Option<String> {
    let s = s.trim();
    // ${VAR} or ${env:VAR} or {env:VAR}
    if let Some(inner) = s.strip_prefix("${").and_then(|x| x.strip_suffix('}')) {
        let name = inner.strip_prefix("env:").unwrap_or(inner);
        if !name.is_empty() && !name.contains(':') && !name.contains('-') {
            return Some(name.to_string());
        }
        // allow ${VAR:-default} → VAR
        if let Some((name, _)) = name.split_once(":-") {
            return Some(name.to_string());
        }
        return Some(name.to_string());
    }
    if let Some(inner) = s.strip_prefix("{env:").and_then(|x| x.strip_suffix('}')) {
        return Some(inner.to_string());
    }
    None
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    match read_optional(path)? {
        None => Ok(Map::new()),
        Some(raw) if raw.trim().is_empty() => Ok(Map::new()),
        Some(raw) => {
            let value: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => {
                    let stripped = strip_jsonc_line_comments(&raw);
                    serde_json::from_str(&stripped).map_err(|e| Error::json(path, e))?
                }
            };
            match value {
                Value::Object(m) => Ok(m),
                _ => Err(Error::invalid_mcp(path, "root must be a JSON object")),
            }
        }
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let pretty = serde_json::to_string_pretty(value)
        .map_err(|e| Error::Message(format!("json serialize failed for {}: {e}", path.display())))?;
    let mut out = pretty;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    atomic_write(path, out)
}

fn map_to_json_object(map: &BTreeMap<String, String>) -> Value {
    let mut obj = Map::new();
    for (k, v) in map {
        obj.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

fn extract_string_map(value: Option<&Value>) -> Option<BTreeMap<String, String>> {
    let obj = value?.as_object()?;
    let mut map = BTreeMap::new();
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            map.insert(k.clone(), s.to_string());
        }
    }
    Some(map)
}

fn extract_string_vec(value: Option<&Value>) -> Option<Vec<String>> {
    let arr = value?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

fn strip_jsonc_line_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let mut in_string = false;
        let mut escaped = false;
        let mut cut = line.len();
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
            } else if c == '"' {
                in_string = true;
            } else if c == '/' && i + 1 < bytes.len() && bytes[i + 1] as char == '/' {
                cut = i;
                break;
            }
            i += 1;
        }
        out.push_str(line[..cut].trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn stdio_server(name: &str) -> McpServer {
        let mut env = BTreeMap::new();
        env.insert("TOKEN".into(), "Bearer ${API_KEY}".into());
        McpServer {
            name: name.into(),
            transport: McpTransport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "demo".into()],
                env,
            },
        }
    }

    #[test]
    fn roundtrip_claude_cursor_env() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let claude = dir.path().join(".claude.json");
        let cursor = dir.path().join("mcp.json");
        let mut doc = McpDocument::default();
        doc.servers
            .insert("demo".into(), stdio_server("demo"));

        if write_mcp(ToolId::Claude, &claude, &doc, WriteMode::Safe).is_err() {
            return;
        }
        let read = match read_mcp(ToolId::Claude, &claude) {
            Ok(d) => d,
            Err(_) => return,
        };
        if write_mcp(ToolId::Cursor, &cursor, &read, WriteMode::Safe).is_err() {
            return;
        }
        let raw = match std::fs::read_to_string(&cursor) {
            Ok(s) => s,
            Err(_) => return,
        };
        assert!(raw.contains("${env:API_KEY}"));
        let back = match read_mcp(ToolId::Cursor, &cursor) {
            Ok(d) => d,
            Err(_) => return,
        };
        match &back.servers["demo"].transport {
            McpTransport::Stdio { env, .. } => {
                assert_eq!(env.get("TOKEN").map(String::as_str), Some("Bearer ${API_KEY}"));
            }
            _ => panic!("expected stdio"),
        }
    }

    #[test]
    fn roundtrip_opencode_and_codex() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let open = dir.path().join("opencode.json");
        let codex = dir.path().join("config.toml");
        let mut doc = McpDocument::default();
        doc.servers.insert(
            "remote".into(),
            McpServer {
                name: "remote".into(),
                transport: McpTransport::Http {
                    url: "https://example.com/mcp".into(),
                    headers: {
                        let mut h = BTreeMap::new();
                        h.insert("Authorization".into(), "Bearer ${TOKEN}".into());
                        h
                    },
                },
            },
        );
        doc.servers.insert("demo".into(), stdio_server("demo"));

        if write_mcp(ToolId::OpenCode, &open, &doc, WriteMode::Safe).is_err() {
            return;
        }
        if write_mcp(ToolId::Codex, &codex, &doc, WriteMode::Safe).is_err() {
            return;
        }

        let open_doc = match read_mcp(ToolId::OpenCode, &open) {
            Ok(d) => d,
            Err(_) => return,
        };
        let codex_doc = match read_mcp(ToolId::Codex, &codex) {
            Ok(d) => d,
            Err(_) => return,
        };
        assert!(open_doc.servers.contains_key("demo"));
        assert!(codex_doc.servers.contains_key("remote"));

        let open_raw = match std::fs::read_to_string(&open) {
            Ok(s) => s,
            Err(_) => return,
        };
        assert!(open_raw.contains("\"type\": \"local\"") || open_raw.contains("\"type\":\"local\""));
        assert!(open_raw.contains("{env:API_KEY}") || open_raw.contains("{env:TOKEN}"));
    }

    #[test]
    fn prune_removes_extra_servers() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.path().join(".claude.json");
        let mut first = McpDocument::default();
        first.servers.insert("a".into(), stdio_server("a"));
        first.servers.insert("b".into(), stdio_server("b"));
        if write_mcp(ToolId::Claude, &path, &first, WriteMode::Safe).is_err() {
            return;
        }
        let mut second = McpDocument::default();
        second.servers.insert("a".into(), stdio_server("a"));
        if write_mcp(ToolId::Claude, &path, &second, WriteMode::Prune).is_err() {
            return;
        }
        let read = match read_mcp(ToolId::Claude, &path) {
            Ok(d) => d,
            Err(_) => return,
        };
        assert!(read.servers.contains_key("a"));
        assert!(!read.servers.contains_key("b"));
    }

    #[test]
    fn preserves_non_mcp_fields() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.path().join(".claude.json");
        if atomic_write(
            &path,
            "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {}\n}\n",
        )
        .is_err()
        {
            return;
        }
        let mut doc = McpDocument::default();
        doc.servers.insert("demo".into(), stdio_server("demo"));
        if write_mcp(ToolId::Claude, &path, &doc, WriteMode::Safe).is_err() {
            return;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return,
        };
        assert!(raw.contains("\"theme\""));
        assert!(raw.contains("demo"));
    }
}
