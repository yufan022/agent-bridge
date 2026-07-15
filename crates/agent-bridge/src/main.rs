//! agent-bridge CLI: sync AI coding agent global configs.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use agent_bridge_core::{diff, list, status, sync, SyncKinds, SyncOptions, ToolId};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agent-bridge",
    version,
    about = "Sync instructions, skills, and MCP configs across Claude Code, Codex, OpenCode, and Cursor (user-global)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Override home directory (for testing)
    #[arg(long, global = true, value_name = "DIR", hide = true)]
    home: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Sync resources from one tool to one or more targets
    Sync {
        /// Source tool: claude, codex, opencode, cursor
        #[arg(long)]
        from: String,

        /// Target tools (comma-separated)
        #[arg(long, value_delimiter = ',')]
        to: Vec<String>,

        /// Limit to kinds: instructions, skills, mcp (comma-separated; default all)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,

        /// Preview changes without writing
        #[arg(long)]
        dry_run: bool,

        /// Delete target MCP servers / skill symlinks absent from source
        #[arg(long)]
        prune: bool,

        /// Replace conflicting skill / instruction paths with symlinks
        #[arg(long)]
        force: bool,
    },
    /// Show a diff between two tools
    Diff {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    /// Probe global paths and counts
    Status {
        /// Optional single tool
        #[arg(long)]
        tool: Option<String>,
    },
    /// List skills and MCP servers for a tool
    List {
        #[arg(long)]
        tool: String,
    },
}

fn parse_tool(s: &str) -> Result<ToolId, String> {
    ToolId::from_str(s).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let home = cli.home;
    match cli.command {
        Commands::Sync {
            from,
            to,
            only,
            dry_run,
            prune,
            force,
        } => {
            let from = parse_tool(&from)?;
            let to = to
                .iter()
                .map(|s| parse_tool(s))
                .collect::<Result<Vec<_>, _>>()?;
            let kinds = SyncKinds::from_list(&only).map_err(|e| e.to_string())?;
            let report = sync(&SyncOptions {
                from,
                to,
                kinds,
                dry_run,
                prune,
                force,
                home,
            })
            .map_err(|e| e.to_string())?;
            print!("{}", report.render());
            if report.success() {
                Ok(())
            } else {
                Err("sync completed with errors".into())
            }
        }
        Commands::Diff { from, to } => {
            let from = parse_tool(&from)?;
            let to = parse_tool(&to)?;
            let text = diff(from, to, home).map_err(|e| e.to_string())?;
            print!("{text}");
            Ok(())
        }
        Commands::Status { tool } => {
            let tool = match tool {
                Some(t) => Some(parse_tool(&t)?),
                None => None,
            };
            let text = status(tool, home).map_err(|e| e.to_string())?;
            print!("{text}");
            Ok(())
        }
        Commands::List { tool } => {
            let tool = parse_tool(&tool)?;
            let text = list(tool, home).map_err(|e| e.to_string())?;
            print!("{text}");
            Ok(())
        }
    }
}
