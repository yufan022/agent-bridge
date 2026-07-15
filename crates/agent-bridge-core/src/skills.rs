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

/// Remove orphan symlinks under `skills_dir` whose names are not in `keep`
/// and that point under `source_skills_root` (optional filter).
pub fn prune_skill_links(
    skills_dir: &Path,
    keep: &std::collections::BTreeSet<String>,
    source_skills_root: Option<&Path>,
) -> Result<Vec<String>> {
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
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
            let target = match fs::read_link(&path) {
                Ok(t) => t,
                Err(_) => {
                    // Broken symlink: remove when pruning
                    fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
                    removed.push(name);
                    continue;
                }
            };
            let resolved = symlink::resolve_link_target(&path, &target).ok();
            let under_root = resolved
                .as_ref()
                .map(|r| r.starts_with(root))
                .unwrap_or(false);
            if !under_root {
                // Only prune links we can attribute to the source skills tree.
                continue;
            }
        }
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
}
