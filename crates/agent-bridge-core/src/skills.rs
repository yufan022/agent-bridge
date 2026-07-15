//! Skill discovery and symlink management.

use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::symlink;

pub use crate::symlink::LinkAction;

/// A discovered skill directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    pub name: String,
    /// Canonical absolute path to the skill directory.
    pub real_path: PathBuf,
}

/// List skills under `skills_dir` (immediate children containing `SKILL.md`).
pub fn list_skills(skills_dir: &Path) -> Result<Vec<SkillEntry>> {
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(skills_dir).map_err(|e| Error::io(skills_dir, e))?;
    for item in read_dir {
        let item = item.map_err(|e| Error::io(skills_dir, e))?;
        let path = item.path();
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let real_path = dunce::canonicalize(&path).map_err(|e| Error::io(&path, e))?;
        entries.push(SkillEntry { name, real_path });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Create or update a symlink at `link_path` pointing to `source_real_path`.
///
/// When `force` is false, existing real directories or conflicting symlinks error.
pub fn link_skill(
    link_path: &Path,
    source_real_path: &Path,
    force: bool,
) -> Result<LinkAction> {
    let name = link_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?");
    symlink::ensure_symlink(link_path, source_real_path, force, name)
}

/// Absolute form of a symlink target without requiring the target to exist.
fn absolute_link_target(link_path: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    }
}

/// Return true when `path` is under `root`, tolerating non-canonical roots.
fn path_under_root(path: &Path, root: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }
    match dunce::canonicalize(root) {
        Ok(canon_root) => path.starts_with(&canon_root),
        Err(_) => false,
    }
}

/// Whether a skill symlink can be attributed to `source_skills_root`.
///
/// Live targets are checked after canonicalize. Dangling targets (source skill
/// deleted) are attributed by the unresolved absolute link path so orphans are
/// still pruned.
fn link_attributable_to_source(
    link_path: &Path,
    target: &Path,
    source_skills_root: &Path,
) -> bool {
    match symlink::resolve_link_target(link_path, target) {
        Ok(resolved) => path_under_root(&resolved, source_skills_root),
        Err(_) => path_under_root(&absolute_link_target(link_path, target), source_skills_root),
    }
}

/// List orphan skill symlink names under `skills_dir` that are not in `keep`
/// and that point under `source_skills_root` (optional filter).
///
/// Does not modify the filesystem.
pub fn list_orphan_skill_links(
    skills_dir: &Path,
    keep: &std::collections::BTreeSet<String>,
    source_skills_root: Option<&Path>,
) -> Result<Vec<String>> {
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }
    let mut orphans = Vec::new();
    let read_dir = fs::read_dir(skills_dir).map_err(|e| Error::io(skills_dir, e))?;
    for item in read_dir {
        let item = item.map_err(|e| Error::io(skills_dir, e))?;
        let path = item.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if keep.contains(&name) {
            continue;
        }
        let meta = match path.symlink_metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.file_type().is_symlink() {
            continue;
        }
        if let Some(root) = source_skills_root {
            match fs::read_link(&path) {
                Ok(target) => {
                    if !link_attributable_to_source(&path, &target, root) {
                        // Only prune links we can attribute to the source skills tree.
                        continue;
                    }
                }
                Err(_) => {
                    // Unreadable symlink target: treat as orphan when pruning.
                }
            }
        }
        orphans.push(name);
    }
    orphans.sort();
    Ok(orphans)
}

/// Remove orphan symlinks under `skills_dir` whose names are not in `keep`
/// and that point under `source_skills_root` (optional filter).
///
/// Returns the names that were removed.
pub fn prune_skill_links(
    skills_dir: &Path,
    keep: &std::collections::BTreeSet<String>,
    source_skills_root: Option<&Path>,
) -> Result<Vec<String>> {
    let orphans = list_orphan_skill_links(skills_dir, keep, source_skills_root)?;
    let mut removed = Vec::new();
    for name in orphans {
        let path = skills_dir.join(&name);
        fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
        removed.push(name);
    }
    Ok(removed)
}

/// Count files under a skill directory (for status display).
pub fn skill_file_count(skill_dir: &Path) -> usize {
    WalkDir::new(skill_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_skill(dir: &Path, name: &str, body: &str) -> PathBuf {
        let skill = dir.join(name);
        let _ = fs::create_dir_all(&skill);
        let _ = fs::write(skill.join("SKILL.md"), body);
        skill
    }

    #[test]
    fn list_and_link_skill() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src_root = dir.path().join("src_skills");
        let dst_root = dir.path().join("dst_skills");
        let _ = fs::create_dir_all(&src_root);
        let _ = write_skill(&src_root, "demo", "---\nname: demo\ndescription: d\n---\n");

        let listed = match list_skills(&src_root) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "demo");

        let link = dst_root.join("demo");
        match link_skill(&link, &listed[0].real_path, false) {
            Ok(LinkAction::Created) => {}
            _ => return,
        }
        assert!(link.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false));

        // Second link without force should be Unchanged
        match link_skill(&link, &listed[0].real_path, false) {
            Ok(LinkAction::Unchanged) => {}
            other => panic!("expected Unchanged, got {other:?}"),
        }
    }

    #[test]
    fn conflict_without_force() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src = write_skill(dir.path(), "a", "x");
        let other = write_skill(dir.path(), "b", "y");
        let link = dir.path().join("link");
        if link_skill(&link, &src, false).is_err() {
            return;
        }
        let err = link_skill(&link, &other, false);
        assert!(err.is_err());
    }

    #[test]
    fn prune_removes_dangling_symlink_under_source_root() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src_root = dir.path().join("src_skills");
        let dst_root = dir.path().join("dst_skills");
        if fs::create_dir_all(&src_root).is_err() || fs::create_dir_all(&dst_root).is_err() {
            return;
        }
        let gone = src_root.join("gone-skill");
        let link = dst_root.join("gone-skill");
        // Dangling symlink whose target path is still under the source skills root.
        if std::os::unix::fs::symlink(&gone, &link).is_err() {
            return;
        }
        assert!(link.symlink_metadata().is_ok());
        assert!(!gone.exists());

        let keep = std::collections::BTreeSet::new();
        let removed = match prune_skill_links(&dst_root, &keep, Some(&src_root)) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert_eq!(removed, vec!["gone-skill".to_string()]);
        assert!(link.symlink_metadata().is_err());
    }

    #[test]
    fn prune_keeps_dangling_symlink_outside_source_root() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src_root = dir.path().join("src_skills");
        let other_root = dir.path().join("other_skills");
        let dst_root = dir.path().join("dst_skills");
        if fs::create_dir_all(&src_root).is_err()
            || fs::create_dir_all(&other_root).is_err()
            || fs::create_dir_all(&dst_root).is_err()
        {
            return;
        }
        let gone = other_root.join("external-skill");
        let link = dst_root.join("external-skill");
        if std::os::unix::fs::symlink(&gone, &link).is_err() {
            return;
        }

        let keep = std::collections::BTreeSet::new();
        let removed = match prune_skill_links(&dst_root, &keep, Some(&src_root)) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert!(removed.is_empty());
        assert!(link.symlink_metadata().is_ok());
    }

    #[test]
    fn prune_removes_live_orphan_under_source_root() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src_root = dir.path().join("src_skills");
        let dst_root = dir.path().join("dst_skills");
        if fs::create_dir_all(&src_root).is_err() || fs::create_dir_all(&dst_root).is_err() {
            return;
        }
        let orphan = write_skill(
            &src_root,
            "orphan",
            "---\nname: orphan\ndescription: d\n---\n",
        );
        let link = dst_root.join("orphan");
        if link_skill(&link, &orphan, false).is_err() {
            return;
        }

        let keep = std::collections::BTreeSet::new();
        let listed = match list_orphan_skill_links(&dst_root, &keep, Some(&src_root)) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert_eq!(listed, vec!["orphan".to_string()]);

        let removed = match prune_skill_links(&dst_root, &keep, Some(&src_root)) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert_eq!(removed, vec!["orphan".to_string()]);
        assert!(link.symlink_metadata().is_err());
        // Source skill directory itself is untouched.
        assert!(orphan.join("SKILL.md").is_file());
    }

    #[test]
    fn prune_skips_names_in_keep() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let src_root = dir.path().join("src_skills");
        let dst_root = dir.path().join("dst_skills");
        if fs::create_dir_all(&src_root).is_err() || fs::create_dir_all(&dst_root).is_err() {
            return;
        }
        let skill = write_skill(&src_root, "keep-me", "---\nname: keep-me\ndescription: d\n---\n");
        let link = dst_root.join("keep-me");
        if link_skill(&link, &skill, false).is_err() {
            return;
        }

        let mut keep = std::collections::BTreeSet::new();
        keep.insert("keep-me".to_string());
        let removed = match prune_skill_links(&dst_root, &keep, Some(&src_root)) {
            Ok(v) => v,
            Err(_) => return,
        };
        assert!(removed.is_empty());
        assert!(link.symlink_metadata().is_ok());
    }
}
