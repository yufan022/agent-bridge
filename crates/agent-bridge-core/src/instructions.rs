//! Instruction file read/write helpers.

use std::path::Path;

use crate::error::Result;
use crate::fsutil::{atomic_write, read_optional};

/// Read instruction body from a plain markdown file.
pub fn read_instructions(path: &Path) -> Result<Option<String>> {
    read_optional(path)
}

/// Write instruction body as plain markdown (trailing newline normalized).
pub fn write_instructions(path: &Path, body: &str) -> Result<()> {
    atomic_write(path, normalize_trailing_newline(body))
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
    fn plain_markdown_roundtrip() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.path().join("CLAUDE.md");
        if write_instructions(&path, "hello").is_err() {
            return;
        }
        let read = match read_instructions(&path) {
            Ok(Some(s)) => s,
            _ => return,
        };
        assert_eq!(read, "hello\n");
    }
}
