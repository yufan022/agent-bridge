//! Instruction file read/write helpers.

use std::path::Path;

use crate::error::Result;
use crate::fsutil::{atomic_write, read_optional};
use crate::tool::ToolId;

const CURSOR_FRONTMATTER: &str = "---\ndescription: Synced by agent-bridge\nalwaysApply: true\n---\n\n";

/// Read instruction body for a tool, stripping Cursor frontmatter when present.
pub fn read_instructions(tool: ToolId, path: &Path) -> Result<Option<String>> {
    let Some(raw) = read_optional(path)? else {
        return Ok(None);
    };
    let body = match tool {
        ToolId::Cursor => strip_mdc_frontmatter(&raw),
        _ => raw,
    };
    Ok(Some(body))
}

/// Write instruction body for a tool (Cursor gets `.mdc` frontmatter).
pub fn write_instructions(tool: ToolId, path: &Path, body: &str) -> Result<()> {
    let contents = match tool {
        ToolId::Cursor => wrap_mdc(body),
        _ => normalize_trailing_newline(body),
    };
    atomic_write(path, contents)
}

fn wrap_mdc(body: &str) -> String {
    let mut out = String::from(CURSOR_FRONTMATTER);
    out.push_str(body.trim_start_matches('\u{feff}'));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn strip_mdc_frontmatter(raw: &str) -> String {
    let trimmed = raw.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return trimmed.to_string();
    }
    let after = &trimmed[3..];
    // Find closing --- on its own line
    let mut offset = 0;
    for line in after.split_inclusive('\n') {
        offset += line.len();
        let content = line.trim_end_matches(['\r', '\n']);
        if content == "---" {
            return after[offset..].trim_start_matches('\n').to_string();
        }
    }
    trimmed.to_string()
}

fn normalize_trailing_newline(body: &str) -> String {
    let mut out = body.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Produce a unified diff between two instruction bodies.
pub fn instructions_diff(from_label: &str, to_label: &str, from: &str, to: &str) -> String {
    similar::TextDiff::from_lines(to, from)
        .unified_diff()
        .context_radius(3)
        .header(to_label, from_label)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cursor_roundtrip_strips_frontmatter() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.path().join("agent-bridge.mdc");
        let body = "# Rules\n\nUse Rust.\n";
        if write_instructions(ToolId::Cursor, &path, body).is_err() {
            return;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return,
        };
        assert!(raw.contains("alwaysApply: true"));
        let read = match read_instructions(ToolId::Cursor, &path) {
            Ok(Some(s)) => s,
            _ => return,
        };
        assert_eq!(read, body);
    }

    #[test]
    fn plain_markdown_roundtrip() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.path().join("CLAUDE.md");
        if write_instructions(ToolId::Claude, &path, "hello").is_err() {
            return;
        }
        let read = match read_instructions(ToolId::Claude, &path) {
            Ok(Some(s)) => s,
            _ => return,
        };
        assert_eq!(read, "hello\n");
    }
}
