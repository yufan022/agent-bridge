//! Shared symlink create/update helpers.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths::ensure_parent;

/// Result of ensuring a symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAction {
    Created,
    Unchanged,
}

/// Create or update a symlink at `link_path` pointing to `source_path`.
///
/// The source is canonicalized to avoid A→B→C chains. When `force` is false,
/// existing real files/directories or conflicting symlinks error.
pub fn ensure_symlink(
    link_path: &Path,
    source_path: &Path,
    force: bool,
    conflict_name: &str,
) -> Result<LinkAction> {
    let source_real =
        dunce::canonicalize(source_path).map_err(|e| Error::io(source_path, e))?;

    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        if let Ok(meta) = link_path.symlink_metadata() {
            if meta.file_type().is_symlink() {
                let current = fs::read_link(link_path).map_err(|e| Error::io(link_path, e))?;
                if let Ok(resolved) = resolve_link_target(link_path, &current) {
                    if resolved == source_real {
                        return Ok(LinkAction::Unchanged);
                    }
                }
                if !force {
                    return Err(Error::LinkConflict {
                        name: conflict_name.to_string(),
                        path: link_path.to_path_buf(),
                        message: "symlink exists and points elsewhere (use --force to replace)"
                            .to_string(),
                    });
                }
                fs::remove_file(link_path).map_err(|e| Error::io(link_path, e))?;
            } else if meta.is_dir() {
                if !force {
                    return Err(Error::LinkConflict {
                        name: conflict_name.to_string(),
                        path: link_path.to_path_buf(),
                        message: "real directory exists (use --force to replace with symlink)"
                            .to_string(),
                    });
                }
                fs::remove_dir_all(link_path).map_err(|e| Error::io(link_path, e))?;
            } else {
                if !force {
                    return Err(Error::LinkConflict {
                        name: conflict_name.to_string(),
                        path: link_path.to_path_buf(),
                        message: "path exists and is not a symlink (use --force to replace)"
                            .to_string(),
                    });
                }
                fs::remove_file(link_path).map_err(|e| Error::io(link_path, e))?;
            }
        }
    }

    ensure_parent(link_path)?;
    std::os::unix::fs::symlink(&source_real, link_path)
        .map_err(|e| Error::io(link_path, e))?;
    Ok(LinkAction::Created)
}

/// Resolve `path` to a canonical real path when it exists (follows symlinks).
///
/// Returns `Ok(None)` when the path is missing or is a broken symlink.
pub fn resolve_real_path(path: &Path) -> Result<Option<PathBuf>> {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
        Ok(_) => match dunce::canonicalize(path) {
            Ok(p) => Ok(Some(p)),
            Err(_) => Ok(None),
        },
    }
}

pub fn resolve_link_target(link_path: &Path, current: &Path) -> Result<PathBuf> {
    let absolute = if current.is_absolute() {
        current.to_path_buf()
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(current)
    };
    dunce::canonicalize(&absolute).map_err(|e| Error::io(&absolute, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_symlink_created_and_unchanged() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src = dir.path().join("src.md");
        let link = dir.path().join("link.md");
        if fs::write(&src, "hi").is_err() {
            return;
        }
        match ensure_symlink(&link, &src, false, "test") {
            Ok(LinkAction::Created) => {}
            _ => return,
        }
        assert!(link
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false));
        match ensure_symlink(&link, &src, false, "test") {
            Ok(LinkAction::Unchanged) => {}
            other => panic!("expected Unchanged, got {other:?}"),
        }
    }

    #[test]
    fn real_file_conflicts_without_force() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src = dir.path().join("src.md");
        let dst = dir.path().join("dst.md");
        if fs::write(&src, "a").is_err() || fs::write(&dst, "b").is_err() {
            return;
        }
        assert!(ensure_symlink(&dst, &src, false, "test").is_err());
        match ensure_symlink(&dst, &src, true, "test") {
            Ok(LinkAction::Created) => {}
            other => panic!("expected Created with force, got {other:?}"),
        }
    }
}
