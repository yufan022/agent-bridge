//! Core library for agent-bridge: sync instructions, skills, and MCP configs
//! across Claude Code, Codex, OpenCode, and Cursor (user-global scope).

pub mod adapters;
pub mod error;
pub mod fsutil;
pub mod instructions;
pub mod mcp;
pub mod paths;
pub mod skills;
pub mod sync;
pub mod tool;

pub use adapters::ToolAdapter;
pub use error::{Error, Result};
pub use paths::ToolPaths;
pub use sync::{diff, list, status, sync, SyncOptions, SyncReport};
pub use tool::{SyncKinds, ToolId, WriteMode};
