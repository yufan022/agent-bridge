//! Instruction file read/link helpers.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::fsutil::{atomic_write, read_optional};
use crate::symlink::{self, LinkAction};

/// Read instruction body from a plain markdown file (follows symlinks).
pub fn read_instructions(path: &Path) -> Result<Option<String>> {
    read_optional(path)
}

/// Write instruction body as plain markdown (trailing newline normalized).
///
/// Used to seed source files in tests; sync itself links rather than copies.
pub fn write_instructions(path: &Path, body: &str) -> Result<()> {
    atomic_write(path, normalize_trailing_newline(body))
}

/// Canonical real path of an instruction file, if it resolves.
pub fn instructions_real_path(path: &Path) -> Result<Option<PathBuf>> {
    symlink::resolve_real_path(path)
}

/// Symlink `link_path` to the canonical path of `source_path`.
pub fn link_instructions(
    link_path: &Path,
    source_path: &Path,
    force: bool,
) -> Result<LinkAction> {
    symlink::ensure_symlink(link_path, source_path, force, "instructions")
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

/// Format an instructions comparison: real paths first; content diff when paths differ.
pub fn format_instructions_comparison(
    from_label: &str,
    to_label: &str,
    from_path: Option<&Path>,
    to_path: Option<&Path>,
    from_real: Option<&Path>,
    to_real: Option<&Path>,
    from_body: &str,
    to_body: &str,
) -> String {
    let mut out = String::new();
    let from_disp = display_resolved(from_path, from_real);
    let to_disp = display_resolved(to_path, to_real);

    if from_real == to_real {
        match from_real {
            Some(real) => {
                out.push_str("(identical)\n");
                out.push_str(&format!("  real_path: {}\n", real.display()));
            }
            None => out.push_str("(identical; both missing)\n"),
        }
        return out;
    }

    out.push_str(&format!("  {from_label}: {from_disp}\n"));
    out.push_str(&format!("  {to_label}: {to_disp}\n"));

    if from_body.trim_end() == to_body.trim_end() {
        out.push_str("(content identical; paths differ)\n");
    } else {
        let diff = instructions_diff(from_label, to_label, from_body, to_body);
        if diff.trim().is_empty() {
            out.push_str("(content identical; paths differ)\n");
        } else {
            out.push_str(&diff);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

fn display_resolved(configured: Option<&Path>, real: Option<&Path>) -> String {
    match (configured, real) {
        (_, Some(real)) => real.display().to_string(),
        (Some(configured), None) => format!("{} (missing)", configured.display()),
        (None, None) => "(unsupported)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

    #[test]
    fn link_instructions_follows_canonical_source() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src = dir.path().join("CLAUDE.md");
        let mid = dir.path().join("mid.md");
        let dst = dir.path().join("AGENTS.md");
        if write_instructions(&src, "rules").is_err() {
            return;
        }
        if std::os::unix::fs::symlink(&src, &mid).is_err() {
            return;
        }
        match link_instructions(&dst, &mid, false) {
            Ok(LinkAction::Created) => {}
            _ => return,
        }
        let dst_real = match instructions_real_path(&dst) {
            Ok(Some(p)) => p,
            _ => return,
        };
        let src_real = match instructions_real_path(&src) {
            Ok(Some(p)) => p,
            _ => return,
        };
        assert_eq!(dst_real, src_real);
        assert!(dst
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false));
        let _ = fs::remove_file(&dst);
    }

    #[test]
    fn comparison_identical_paths() {
        let text = format_instructions_comparison(
            "claude",
            "codex",
            Some(Path::new("/a/CLAUDE.md")),
            Some(Path::new("/b/AGENTS.md")),
            Some(Path::new("/real/file.md")),
            Some(Path::new("/real/file.md")),
            "same",
            "same",
        );
        assert!(text.contains("(identical)"));
        assert!(text.contains("/real/file.md"));
    }

    #[test]
    fn comparison_paths_differ_content_same() {
        let text = format_instructions_comparison(
            "claude",
            "codex",
            Some(Path::new("/a/CLAUDE.md")),
            Some(Path::new("/b/AGENTS.md")),
            Some(Path::new("/a/CLAUDE.md")),
            Some(Path::new("/b/AGENTS.md")),
            "hello\n",
            "hello\n",
        );
        assert!(text.contains("(content identical; paths differ)"));
        assert!(!text.contains("(identical)\n  real_path"));
    }
}
