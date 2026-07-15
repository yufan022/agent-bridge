//! Global path resolution for each supported tool.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::tool::ToolId;

/// Resolved global filesystem paths for one tool under a home directory.
#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub tool: ToolId,
    pub home: PathBuf,
    /// Global instruction file, if this tool supports file-based instructions.
    ///
    /// Cursor User Rules have no stable file API, so this is `None` for Cursor.
    pub instructions: Option<PathBuf>,
    pub skills_dir: PathBuf,
    pub mcp_config: PathBuf,
}

impl ToolPaths {
    /// Build paths for `tool` under the real user home directory.
    pub fn for_tool(tool: ToolId) -> Result<Self> {
        let home = dirs::home_dir().ok_or(Error::HomeNotFound)?;
        Ok(Self::for_tool_in_home(tool, home))
    }

    /// Build paths for `tool` under an arbitrary home root (useful in tests).
    pub fn for_tool_in_home(tool: ToolId, home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        match tool {
            ToolId::Claude => Self {
                tool,
                instructions: Some(home.join(".claude/CLAUDE.md")),
                skills_dir: home.join(".claude/skills"),
                mcp_config: home.join(".claude.json"),
                home,
            },
            ToolId::Codex => Self {
                tool,
                instructions: Some(home.join(".codex/AGENTS.md")),
                skills_dir: home.join(".codex/skills"),
                mcp_config: home.join(".codex/config.toml"),
                home,
            },
            ToolId::OpenCode => Self {
                tool,
                instructions: Some(home.join(".config/opencode/AGENTS.md")),
                skills_dir: home.join(".config/opencode/skills"),
                mcp_config: home.join(".config/opencode/opencode.json"),
                home,
            },
            ToolId::Cursor => Self {
                tool,
                instructions: None,
                skills_dir: home.join(".cursor/skills"),
                mcp_config: home.join(".cursor/mcp.json"),
                home,
            },
        }
    }

    /// Whether this tool supports syncing a global instruction file.
    pub fn supports_instructions(&self) -> bool {
        self.instructions.is_some()
    }

    pub fn skill_dir(&self, name: &str) -> PathBuf {
        self.skills_dir.join(name)
    }

    pub fn skill_file(&self, name: &str) -> PathBuf {
        self.skill_dir(name).join("SKILL.md")
    }
}

/// Ensure parent directories exist for a file path.
pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn claude_paths() {
        let p = ToolPaths::for_tool_in_home(ToolId::Claude, Path::new("/tmp/home"));
        assert_eq!(
            p.instructions.as_deref(),
            Some(Path::new("/tmp/home/.claude/CLAUDE.md"))
        );
        assert_eq!(p.skills_dir, Path::new("/tmp/home/.claude/skills"));
        assert_eq!(p.mcp_config, Path::new("/tmp/home/.claude.json"));
    }

    #[test]
    fn codex_paths() {
        let p = ToolPaths::for_tool_in_home(ToolId::Codex, Path::new("/tmp/home"));
        assert_eq!(
            p.instructions.as_deref(),
            Some(Path::new("/tmp/home/.codex/AGENTS.md"))
        );
        assert_eq!(p.skills_dir, Path::new("/tmp/home/.codex/skills"));
        assert_eq!(p.mcp_config, Path::new("/tmp/home/.codex/config.toml"));
    }

    #[test]
    fn opencode_paths() {
        let p = ToolPaths::for_tool_in_home(ToolId::OpenCode, Path::new("/tmp/home"));
        assert_eq!(
            p.instructions.as_deref(),
            Some(Path::new("/tmp/home/.config/opencode/AGENTS.md"))
        );
        assert_eq!(
            p.skills_dir,
            Path::new("/tmp/home/.config/opencode/skills")
        );
        assert_eq!(
            p.mcp_config,
            Path::new("/tmp/home/.config/opencode/opencode.json")
        );
    }

    #[test]
    fn cursor_paths_skip_instructions() {
        let p = ToolPaths::for_tool_in_home(ToolId::Cursor, Path::new("/tmp/home"));
        assert!(p.instructions.is_none());
        assert!(!p.supports_instructions());
        assert_eq!(p.skills_dir, Path::new("/tmp/home/.cursor/skills"));
        assert_eq!(p.mcp_config, Path::new("/tmp/home/.cursor/mcp.json"));
    }
}
