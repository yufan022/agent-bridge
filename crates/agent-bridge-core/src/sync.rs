//! Sync engine: copy/link resources from one tool to others.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::adapters::ToolAdapter;
use crate::error::{Error, Result};
use crate::instructions;
use crate::skills;
use crate::tool::{SyncKinds, ToolId, WriteMode};

/// Options for a sync run.
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub from: ToolId,
    pub to: Vec<ToolId>,
    pub kinds: SyncKinds,
    pub dry_run: bool,
    pub prune: bool,
    pub force: bool,
    /// Optional home override for tests.
    pub home: Option<PathBuf>,
}

/// Human-readable report of a sync.
#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub lines: Vec<String>,
    pub errors: Vec<String>,
}

impl SyncReport {
    pub fn success(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let _ = writeln!(out, "{line}");
        }
        if !self.errors.is_empty() {
            let _ = writeln!(out, "\nErrors:");
            for e in &self.errors {
                let _ = writeln!(out, "  - {e}");
            }
        }
        out
    }
}

fn adapter(tool: ToolId, home: &Option<PathBuf>) -> Result<ToolAdapter> {
    match home {
        Some(h) => Ok(ToolAdapter::in_home(tool, h)),
        None => ToolAdapter::new(tool),
    }
}

/// Run a sync according to `opts`.
pub fn sync(opts: &SyncOptions) -> Result<SyncReport> {
    if opts.to.is_empty() {
        return Err(Error::Message("at least one --to target is required".into()));
    }
    for t in &opts.to {
        if *t == opts.from {
            return Err(Error::SameSourceAndTarget(opts.from.to_string()));
        }
    }

    let source = adapter(opts.from, &opts.home)?;
    let mut report = SyncReport::default();
    report.lines.push(format!(
        "Source: {} ({})",
        source.id(),
        source.paths().home.display()
    ));

    let instructions = if opts.kinds.instructions {
        source.read_instructions()?
    } else {
        None
    };
    let skills = if opts.kinds.skills {
        source.list_skills()?
    } else {
        Vec::new()
    };
    let mcp = if opts.kinds.mcp {
        source.read_mcp()?
    } else {
        Default::default()
    };

    if opts.kinds.instructions {
        match &instructions {
            Some(body) => report.lines.push(format!(
                "  read instructions ({} chars)",
                body.chars().count()
            )),
            None => report
                .lines
                .push("  instructions: source file missing (skip writes)".into()),
        }
    }
    if opts.kinds.skills {
        report
            .lines
            .push(format!("  read {} skill(s)", skills.len()));
    }
    if opts.kinds.mcp {
        report
            .lines
            .push(format!("  read {} MCP server(s)", mcp.servers.len()));
    }

    let write_mode = if opts.prune {
        WriteMode::Prune
    } else {
        WriteMode::Safe
    };

    for target_id in &opts.to {
        let target = adapter(*target_id, &opts.home)?;
        report.lines.push(format!("\nTarget: {}", target.id()));

        if opts.kinds.instructions {
            if let Some(body) = &instructions {
                let existing = target.read_instructions()?.unwrap_or_default();
                if existing.trim_end() == body.trim_end() {
                    report.lines.push("  instructions: unchanged".into());
                } else if opts.dry_run {
                    report.lines.push("  instructions: would update".into());
                    let diff = instructions::instructions_diff(
                        &format!("{} (source)", source.id()),
                        &format!("{} (target)", target.id()),
                        body,
                        &existing,
                    );
                    if !diff.trim().is_empty() {
                        report.lines.push(diff);
                    }
                } else {
                    target.write_instructions(body)?;
                    report.lines.push(format!(
                        "  instructions: wrote {}",
                        target.paths().instructions.display()
                    ));
                }
            } else {
                report
                    .lines
                    .push("  instructions: skipped (source missing)".into());
            }
        }

        if opts.kinds.skills {
            let keep: BTreeSet<String> = skills.iter().map(|s| s.name.clone()).collect();
            for skill in &skills {
                if opts.dry_run {
                    let link = target.paths().skill_dir(&skill.name);
                    report.lines.push(format!(
                        "  skill '{}': would symlink {} -> {}",
                        skill.name,
                        link.display(),
                        skill.real_path.display()
                    ));
                    continue;
                }
                match target.link_skill(&skill.name, &skill.real_path, opts.force) {
                    Ok(action) => report.lines.push(format!(
                        "  skill '{}': {:?}",
                        skill.name, action
                    )),
                    Err(e) => {
                        report.errors.push(format!("{} skill '{}': {e}", target.id(), skill.name));
                        report.lines.push(format!("  skill '{}': ERROR {e}", skill.name));
                    }
                }
            }
            if opts.prune {
                if opts.dry_run {
                    report.lines.push(
                        "  skills prune: would remove orphan symlinks not in source".into(),
                    );
                } else {
                    let removed = skills::prune_skill_links(
                        &target.paths().skills_dir,
                        &keep,
                        Some(&source.paths().skills_dir),
                    )?;
                    for name in removed {
                        report
                            .lines
                            .push(format!("  skill '{name}': pruned symlink"));
                    }
                }
            }
        }

        if opts.kinds.mcp {
            let existing = target.read_mcp()?;
            if existing == mcp && write_mode == WriteMode::Safe {
                report.lines.push("  mcp: unchanged".into());
            } else if opts.dry_run {
                report.lines.push(format!(
                    "  mcp: would write {} server(s) ({write_mode:?})",
                    mcp.servers.len()
                ));
                let src_names: BTreeSet<_> = mcp.servers.keys().cloned().collect();
                let dst_names: BTreeSet<_> = existing.servers.keys().cloned().collect();
                for n in src_names.difference(&dst_names) {
                    report.lines.push(format!("    + {n}"));
                }
                for n in dst_names.difference(&src_names) {
                    if write_mode == WriteMode::Prune {
                        report.lines.push(format!("    - {n}"));
                    }
                }
                for n in src_names.intersection(&dst_names) {
                    if existing.servers.get(n) != mcp.servers.get(n) {
                        report.lines.push(format!("    ~ {n}"));
                    }
                }
            } else {
                target.write_mcp(&mcp, write_mode)?;
                report.lines.push(format!(
                    "  mcp: wrote {} ({:?})",
                    target.paths().mcp_config.display(),
                    write_mode
                ));
            }
        }
    }

    Ok(report)
}

/// Diff instructions / skills / mcp between two tools.
pub fn diff(from: ToolId, to: ToolId, home: Option<PathBuf>) -> Result<String> {
    if from == to {
        return Err(Error::SameSourceAndTarget(from.to_string()));
    }
    let source = adapter(from, &home)?;
    let target = adapter(to, &home)?;
    let mut out = String::new();

    let _ = writeln!(out, "## Instructions");
    let src_i = source.read_instructions()?.unwrap_or_default();
    let dst_i = target.read_instructions()?.unwrap_or_default();
    if src_i.trim_end() == dst_i.trim_end() {
        let _ = writeln!(out, "(identical)");
    } else {
        out.push_str(&instructions::instructions_diff(
            &format!("{from}"),
            &format!("{to}"),
            &src_i,
            &dst_i,
        ));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }

    let _ = writeln!(out, "\n## Skills");
    let src_skills: BTreeSet<_> = source
        .list_skills()?
        .into_iter()
        .map(|s| s.name)
        .collect();
    let dst_skills: BTreeSet<_> = target
        .list_skills()?
        .into_iter()
        .map(|s| s.name)
        .collect();
    for n in src_skills.difference(&dst_skills) {
        let _ = writeln!(out, "+ {n}");
    }
    for n in dst_skills.difference(&src_skills) {
        let _ = writeln!(out, "- {n}");
    }
    for n in src_skills.intersection(&dst_skills) {
        let _ = writeln!(out, "= {n}");
    }
    if src_skills.is_empty() && dst_skills.is_empty() {
        let _ = writeln!(out, "(none)");
    }

    let _ = writeln!(out, "\n## MCP");
    let src_mcp = source.read_mcp()?;
    let dst_mcp = target.read_mcp()?;
    let src_names: BTreeSet<_> = src_mcp.servers.keys().cloned().collect();
    let dst_names: BTreeSet<_> = dst_mcp.servers.keys().cloned().collect();
    for n in src_names.difference(&dst_names) {
        let _ = writeln!(out, "+ {n}");
    }
    for n in dst_names.difference(&src_names) {
        let _ = writeln!(out, "- {n}");
    }
    for n in src_names.intersection(&dst_names) {
        if src_mcp.servers.get(n) == dst_mcp.servers.get(n) {
            let _ = writeln!(out, "= {n}");
        } else {
            let _ = writeln!(out, "~ {n}");
        }
    }
    if src_names.is_empty() && dst_names.is_empty() {
        let _ = writeln!(out, "(none)");
    }

    Ok(out)
}

/// Status for all tools (or one).
pub fn status(tool: Option<ToolId>, home: Option<PathBuf>) -> Result<String> {
    let tools: Vec<ToolId> = match tool {
        Some(t) => vec![t],
        None => ToolId::ALL.to_vec(),
    };
    let mut out = String::new();
    for t in tools {
        let adapter = adapter(t, &home)?;
        for line in adapter.status_lines()? {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out);
    }
    Ok(out)
}

/// List skills and MCP server names for a tool.
pub fn list(tool: ToolId, home: Option<PathBuf>) -> Result<String> {
    let adapter = adapter(tool, &home)?;
    let mut out = String::new();
    let _ = writeln!(out, "Tool: {tool}");
    let _ = writeln!(out, "\nSkills:");
    let skills = adapter.list_skills()?;
    if skills.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for s in skills {
            let _ = writeln!(out, "  - {} ({})", s.name, s.real_path.display());
        }
    }
    let _ = writeln!(out, "\nMCP servers:");
    let mcp = adapter.read_mcp()?;
    if mcp.servers.is_empty() {
        let _ = writeln!(out, "  (none)");
    } else {
        for name in mcp.names() {
            let _ = writeln!(out, "  - {name}");
        }
    }
    let _ = writeln!(out, "\nInstructions: {}", adapter.paths().instructions.display());
    match adapter.read_instructions()? {
        Some(body) => {
            let _ = writeln!(out, "  present ({} chars)", body.chars().count());
        }
        None => {
            let _ = writeln!(out, "  missing");
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{McpDocument, McpServer, McpTransport};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    fn setup_claude_home(home: &std::path::Path) {
        let _ = fs::create_dir_all(home.join(".claude/skills/demo"));
        let _ = fs::write(
            home.join(".claude/CLAUDE.md"),
            "# Claude rules\nAlways test.\n",
        );
        let _ = fs::write(
            home.join(".claude/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: demo skill\n---\n\nDo the demo.\n",
        );
        let mut env = BTreeMap::new();
        env.insert("KEY".into(), "${SECRET}".into());
        let mut doc = McpDocument::default();
        doc.servers.insert(
            "demo".into(),
            McpServer {
                name: "demo".into(),
                transport: McpTransport::Stdio {
                    command: "npx".into(),
                    args: vec!["-y".into(), "pkg".into()],
                    env,
                },
            },
        );
        let adapter = ToolAdapter::in_home(ToolId::Claude, home);
        let _ = adapter.write_mcp(&doc, WriteMode::Safe);
    }

    #[test]
    fn sync_claude_to_cursor_and_codex() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let home = dir.path();
        setup_claude_home(home);

        let report = match sync(&SyncOptions {
            from: ToolId::Claude,
            to: vec![ToolId::Cursor, ToolId::Codex, ToolId::OpenCode],
            kinds: SyncKinds::all(),
            dry_run: false,
            prune: false,
            force: false,
            home: Some(home.to_path_buf()),
        }) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        assert!(report.success(), "{}", report.render());

        let cursor = ToolAdapter::in_home(ToolId::Cursor, home);
        let instr = match cursor.read_instructions() {
            Ok(Some(s)) => s,
            other => panic!("cursor instructions: {other:?}"),
        };
        assert!(instr.contains("Always test"));

        let skill_link = home.join(".cursor/skills/demo");
        assert!(
            skill_link
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        );

        let cursor_mcp = match cursor.read_mcp() {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        assert!(cursor_mcp.servers.contains_key("demo"));

        let codex_raw = match fs::read_to_string(home.join(".codex/config.toml")) {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        assert!(codex_raw.contains("mcp_servers") || codex_raw.contains("[mcp_servers"));
    }
}
