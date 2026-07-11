//! Core library for agent-bridge: sync instructions, skills, and MCP configs
//! across Claude Code, Codex, OpenCode, and Cursor (user-global scope).
//!
//! Cursor does not support file-based instruction sync (User Rules have no
//! stable file API); only skills and MCP are synced for that tool.

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
