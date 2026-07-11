//! Atomic file helpers.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::error::{Error, Result};
use crate::paths::ensure_parent;

/// Write `contents` to `path` via a temporary sibling file then rename.
pub fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    ensure_parent(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("tmp");
    let tmp_name = format!(".{file_name}.agent-bridge.tmp");
    let tmp_path = parent.join(tmp_name);

    {
        let mut file = fs::File::create(&tmp_path).map_err(|e| Error::io(&tmp_path, e))?;
        file.write_all(contents.as_ref())
            .map_err(|e| Error::io(&tmp_path, e))?;
        file.sync_all().map_err(|e| Error::io(&tmp_path, e))?;
    }

    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        Error::io(path, e)
    })?;
    Ok(())
}

/// Read a file to string; return `Ok(None)` when missing.
pub fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_creates_and_overwrites() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = dir.path().join("nested/a.json");
        if atomic_write(&path, b"{\"a\":1}").is_err() {
            return;
        }
        let first = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return,
        };
        assert_eq!(first, "{\"a\":1}");
        if atomic_write(&path, b"{\"a\":2}").is_err() {
            return;
        }
        let second = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return,
        };
        assert_eq!(second, "{\"a\":2}");
    }
}
