//! Tool identifiers and sync resource kinds.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// Supported AI coding tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    Claude,
    Codex,
    OpenCode,
    Cursor,
}

impl ToolId {
    pub const ALL: [ToolId; 4] = [
        ToolId::Claude,
        ToolId::Codex,
        ToolId::OpenCode,
        ToolId::Cursor,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ToolId::Claude => "claude",
            ToolId::Codex => "codex",
            ToolId::OpenCode => "opencode",
            ToolId::Cursor => "cursor",
        }
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ToolId {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Ok(ToolId::Claude),
            "codex" => Ok(ToolId::Codex),
            "opencode" | "open-code" => Ok(ToolId::OpenCode),
            "cursor" => Ok(ToolId::Cursor),
            other => Err(Error::UnknownTool(other.to_string())),
        }
    }
}

/// Which resource categories to sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncKinds {
    pub instructions: bool,
    pub skills: bool,
    pub mcp: bool,
}

impl SyncKinds {
    pub fn all() -> Self {
        Self {
            instructions: true,
            skills: true,
            mcp: true,
        }
    }

    pub fn from_list(items: &[String]) -> Result<Self> {
        if items.is_empty() {
            return Ok(Self::all());
        }
        let mut kinds = Self {
            instructions: false,
            skills: false,
            mcp: false,
        };
        for item in items {
            match item.trim().to_ascii_lowercase().as_str() {
                "instructions" | "instruction" | "instr" => kinds.instructions = true,
                "skills" | "skill" => kinds.skills = true,
                "mcp" => kinds.mcp = true,
                other => {
                    return Err(Error::Message(format!(
                        "unknown sync kind '{other}' (expected instructions, skills, mcp)"
                    )));
                }
            }
        }
        Ok(kinds)
    }
}

/// MCP write behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Create and update servers; never delete.
    Safe,
    /// Mirror source: delete target servers absent from source.
    Prune,
}
