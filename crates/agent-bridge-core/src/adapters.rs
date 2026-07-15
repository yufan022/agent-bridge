//! Per-tool adapters over global paths.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::instructions::{self, read_instructions, write_instructions};
use crate::mcp::{self, McpDocument};
use crate::paths::ToolPaths;
use crate::skills::{self, LinkAction, SkillEntry};
use crate::tool::{ToolId, WriteMode};

/// Adapter bound to a concrete home directory (real or test sandbox).
#[derive(Debug, Clone)]
pub struct ToolAdapter {
    paths: ToolPaths,
}

impl ToolAdapter {
    pub fn new(tool: ToolId) -> Result<Self> {
        Ok(Self {
            paths: ToolPaths::for_tool(tool)?,
        })
    }

    pub fn in_home(tool: ToolId, home: impl Into<PathBuf>) -> Self {
        Self {
            paths: ToolPaths::for_tool_in_home(tool, home),
        }
    }

    pub fn id(&self) -> ToolId {
        self.paths.tool
    }

    pub fn paths(&self) -> &ToolPaths {
        &self.paths
    }

    pub fn supports_instructions(&self) -> bool {
        self.paths.supports_instructions()
    }

    pub fn instructions_path(&self) -> Option<&Path> {
        self.paths.instructions.as_deref()
    }

    pub fn read_instructions(&self) -> Result<Option<String>> {
        let Some(path) = &self.paths.instructions else {
            return Ok(None);
        };
        read_instructions(path)
    }

    /// Canonical real path of this tool's instruction file, if it resolves.
    pub fn instructions_real_path(&self) -> Result<Option<PathBuf>> {
        let Some(path) = &self.paths.instructions else {
            return Ok(None);
        };
        instructions::instructions_real_path(path)
    }

    pub fn write_instructions(&self, body: &str) -> Result<()> {
        let Some(path) = &self.paths.instructions else {
            return Err(Error::Message(format!(
                "{} does not support file-based instruction sync \
                 (Cursor User Rules have no stable file API)",
                self.id()
            )));
        };
        write_instructions(path, body)
    }

    /// Symlink this tool's instruction path to `source_path` (canonicalized).
    pub fn link_instructions(&self, source_path: &Path, force: bool) -> Result<LinkAction> {
        let Some(link_path) = &self.paths.instructions else {
            return Err(Error::Message(format!(
                "{} does not support file-based instruction sync \
                 (Cursor User Rules have no stable file API)",
                self.id()
            )));
        };
        instructions::link_instructions(link_path, source_path, force)
    }

    pub fn list_skills(&self) -> Result<Vec<SkillEntry>> {
        skills::list_skills(&self.paths.skills_dir)
    }

    pub fn link_skill(&self, name: &str, source_real_path: &Path, force: bool) -> Result<LinkAction> {
        let link = self.paths.skill_dir(name);
        skills::link_skill(&link, source_real_path, force)
    }

    pub fn read_mcp(&self) -> Result<McpDocument> {
        mcp::read_mcp(self.paths.tool, &self.paths.mcp_config)
    }

    pub fn write_mcp(&self, doc: &McpDocument, mode: WriteMode) -> Result<()> {
        mcp::write_mcp(self.paths.tool, &self.paths.mcp_config, doc, mode)
    }

    pub fn instructions_path_display(&self) -> String {
        match &self.paths.instructions {
            Some(path) => path.display().to_string(),
            None => "(unsupported)".into(),
        }
    }

    pub fn status_lines(&self) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        let instr_supported = self.supports_instructions();
        let instr = self
            .paths
            .instructions
            .as_ref()
            .is_some_and(|p| p.exists());
        let skills = self.paths.skills_dir.exists();
        let mcp = self.paths.mcp_config.exists();
        lines.push(format!(
            "{}: instructions={} skills_dir={} mcp={}",
            self.id(),
            if instr_supported {
                bool_mark(instr)
            } else {
                "n/a"
            },
            bool_mark(skills),
            bool_mark(mcp)
        ));
        lines.push(format!(
            "  instructions: {}",
            self.instructions_path_display()
        ));
        if let Some(real) = self.instructions_real_path()? {
            lines.push(format!("  instructions_real: {}", real.display()));
        }
        lines.push(format!("  skills:       {}", self.paths.skills_dir.display()));
        lines.push(format!("  mcp:          {}", self.paths.mcp_config.display()));
        if instr_supported {
            if let Some(body) = self.read_instructions()? {
                let chars = body.chars().count();
                lines.push(format!("  instructions_chars: {chars}"));
            }
        }
        let skill_list = self.list_skills()?;
        lines.push(format!("  skill_count: {}", skill_list.len()));
        let mcp_doc = self.read_mcp()?;
        lines.push(format!("  mcp_server_count: {}", mcp_doc.servers.len()));
        Ok(lines)
    }
}

fn bool_mark(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}
