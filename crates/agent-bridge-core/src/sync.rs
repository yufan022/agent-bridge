//! Sync engine: copy/link resources from one tool to others.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::adapters::ToolAdapter;
use crate::error::{Error, Result};
use crate::instructions;
use crate::mcp::{normalize_mcp_for_tool, sse_conversions_for_codex};
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

    let source_supports_instructions = source.supports_instructions();
    let source_instructions_path = if opts.kinds.instructions && source_supports_instructions {
        source.instructions_real_path()?
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
        if !source_supports_instructions {
            report.lines.push(format!(
                "  instructions: skipped ({} has no file-based instructions)",
                source.id()
            ));
        } else {
            match &source_instructions_path {
                Some(path) => report.lines.push(format!(
                    "  instructions source: {}",
                    path.display()
                )),
                None => report
                    .lines
                    .push("  instructions: source file missing (skip links)".into()),
            }
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
            if !target.supports_instructions() {
                report.lines.push(format!(
                    "  instructions: skipped ({} has no file-based instructions)",
                    target.id()
                ));
            } else if let Some(source_real) = &source_instructions_path {
                if let Some(link_path) = target.instructions_path() {
                    let target_real = target.instructions_real_path()?;
                    if target_real.as_ref() == Some(source_real) {
                        report.lines.push("  instructions: unchanged".into());
                    } else if opts.dry_run {
                        report.lines.push(format!(
                            "  instructions: would symlink {} -> {}",
                            link_path.display(),
                            source_real.display()
                        ));
                        let src_body = source.read_instructions()?.unwrap_or_default();
                        let dst_body = target.read_instructions()?.unwrap_or_default();
                        let comparison = instructions::format_instructions_comparison(
                            &format!("{} (source)", source.id()),
                            &format!("{} (target)", target.id()),
                            source.instructions_path(),
                            target.instructions_path(),
                            Some(source_real.as_path()),
                            target_real.as_deref(),
                            &src_body,
                            &dst_body,
                        );
                        for line in comparison.lines() {
                            report.lines.push(line.to_string());
                        }
                    } else {
                        match target.link_instructions(source_real, opts.force) {
                            Ok(action) => report.lines.push(format!(
                                "  instructions: {:?} {} -> {}",
                                action,
                                link_path.display(),
                                source_real.display()
                            )),
                            Err(e) => {
                                report.errors.push(format!(
                                    "{} instructions: {e}",
                                    target.id()
                                ));
                                report.lines.push(format!("  instructions: ERROR {e}"));
                            }
                        }
                    }
                }
            } else {
                report
                    .lines
                    .push("  instructions: skipped (source missing or unsupported)".into());
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
                let source_skills = source.paths().skills_dir.clone();
                if opts.dry_run {
                    let orphans = skills::list_orphan_skill_links(
                        &target.paths().skills_dir,
                        &keep,
                        Some(&source_skills),
                    )?;
                    if orphans.is_empty() {
                        report
                            .lines
                            .push("  skills prune: no orphan symlinks".into());
                    } else {
                        for name in orphans {
                            report
                                .lines
                                .push(format!("  skill '{name}': would prune symlink"));
                        }
                    }
                } else {
                    let removed = skills::prune_skill_links(
                        &target.paths().skills_dir,
                        &keep,
                        Some(&source_skills),
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
            let to_write = normalize_mcp_for_tool(*target_id, &mcp);
            if *target_id == ToolId::Codex {
                for name in sse_conversions_for_codex(&mcp) {
                    report.lines.push(format!(
                        "  mcp '{name}': converting SSE → streamable HTTP for Codex"
                    ));
                }
            }
            if existing == to_write && write_mode == WriteMode::Safe {
                report.lines.push("  mcp: unchanged".into());
            } else if opts.dry_run {
                report.lines.push(format!(
                    "  mcp: would write {} server(s) ({write_mode:?})",
                    to_write.servers.len()
                ));
                let src_names: BTreeSet<_> = to_write.servers.keys().cloned().collect();
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
                    if existing.servers.get(n) != to_write.servers.get(n) {
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
    match (source.supports_instructions(), target.supports_instructions()) {
        (false, false) => {
            let _ = writeln!(
                out,
                "(skipped: neither {from} nor {to} supports file-based instructions)"
            );
        }
        (false, true) => {
            let _ = writeln!(
                out,
                "(skipped: {from} does not support file-based instructions)"
            );
        }
        (true, false) => {
            let _ = writeln!(
                out,
                "(skipped: {to} does not support file-based instructions)"
            );
        }
        (true, true) => {
            let src_real = source.instructions_real_path()?;
            let dst_real = target.instructions_real_path()?;
            let src_i = source.read_instructions()?.unwrap_or_default();
            let dst_i = target.read_instructions()?.unwrap_or_default();
            out.push_str(&instructions::format_instructions_comparison(
                &format!("{from}"),
                &format!("{to}"),
                source.instructions_path(),
                target.instructions_path(),
                src_real.as_deref(),
                dst_real.as_deref(),
                &src_i,
                &dst_i,
            ));
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
    // Compare using the target-normalized source so SSE→streamable HTTP for Codex
    // does not show as a perpetual drift.
    let src_for_target = normalize_mcp_for_tool(to, &src_mcp);
    let src_names: BTreeSet<_> = src_for_target.servers.keys().cloned().collect();
    let dst_names: BTreeSet<_> = dst_mcp.servers.keys().cloned().collect();
    for n in src_names.difference(&dst_names) {
        let _ = writeln!(out, "+ {n}");
    }
    for n in dst_names.difference(&src_names) {
        let _ = writeln!(out, "- {n}");
    }
    for n in src_names.intersection(&dst_names) {
        if src_for_target.servers.get(n) == dst_mcp.servers.get(n) {
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
    let _ = writeln!(
        out,
        "\nInstructions: {}",
        adapter.instructions_path_display()
    );
    if !adapter.supports_instructions() {
        let _ = writeln!(out, "  unsupported (no stable file API)");
    } else {
        match adapter.read_instructions()? {
            Some(body) => {
                let _ = writeln!(out, "  present ({} chars)", body.chars().count());
            }
            None => {
                let _ = writeln!(out, "  missing");
            }
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
        assert!(
            report.render().contains("instructions: skipped (cursor has no file-based instructions)"),
            "expected cursor instructions skip, got:\n{}",
            report.render()
        );

        let cursor = ToolAdapter::in_home(ToolId::Cursor, home);
        assert!(cursor.read_instructions().ok().flatten().is_none());
        assert!(!home.join(".cursor/rules/agent-bridge.mdc").exists());

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

        let codex = ToolAdapter::in_home(ToolId::Codex, home);
        let codex_instr_path = home.join(".codex/AGENTS.md");
        assert!(
            codex_instr_path
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false),
            "codex instructions should be a symlink"
        );
        let codex_instr = match codex.read_instructions() {
            Ok(Some(s)) => s,
            other => panic!("codex instructions: {other:?}"),
        };
        assert!(codex_instr.contains("Always test"));

        let claude_real = match ToolAdapter::in_home(ToolId::Claude, home).instructions_real_path()
        {
            Ok(Some(p)) => p,
            other => panic!("claude real path: {other:?}"),
        };
        let codex_real = match codex.instructions_real_path() {
            Ok(Some(p)) => p,
            other => panic!("codex real path: {other:?}"),
        };
        assert_eq!(claude_real, codex_real);

        let codex_raw = match fs::read_to_string(home.join(".codex/config.toml")) {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        assert!(codex_raw.contains("mcp_servers") || codex_raw.contains("[mcp_servers"));
    }

    #[test]
    fn sync_claude_sse_to_codex_converts_protocol() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let home = dir.path();
        let _ = fs::create_dir_all(home.join(".claude"));
        let _ = fs::write(
            home.join(".claude.json"),
            r#"{
  "mcpServers": {
    "asana": {
      "type": "sse",
      "url": "https://mcp.asana.com/sse"
    },
    "notion": {
      "type": "http",
      "url": "https://mcp.notion.com/mcp"
    }
  }
}
"#,
        );

        let report = match sync(&SyncOptions {
            from: ToolId::Claude,
            to: vec![ToolId::Codex],
            kinds: SyncKinds {
                instructions: false,
                skills: false,
                mcp: true,
            },
            dry_run: false,
            prune: false,
            force: false,
            home: Some(home.to_path_buf()),
        }) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        assert!(report.success(), "{}", report.render());
        assert!(
            report
                .render()
                .contains("converting SSE → streamable HTTP for Codex"),
            "expected conversion notice, got:\n{}",
            report.render()
        );

        let codex = ToolAdapter::in_home(ToolId::Codex, home);
        let codex_mcp = match codex.read_mcp() {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        match &codex_mcp.servers["asana"].transport {
            McpTransport::Http { protocol, url, .. } => {
                assert_eq!(*protocol, crate::mcp::HttpProtocol::StreamableHttp);
                assert_eq!(url, "https://mcp.asana.com/mcp");
            }
            _ => panic!("expected http"),
        }

        // Second sync should be a no-op (no perpetual rewrite from protocol mismatch).
        let report2 = match sync(&SyncOptions {
            from: ToolId::Claude,
            to: vec![ToolId::Codex],
            kinds: SyncKinds {
                instructions: false,
                skills: false,
                mcp: true,
            },
            dry_run: false,
            prune: false,
            force: false,
            home: Some(home.to_path_buf()),
        }) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        assert!(
            report2.render().contains("mcp: unchanged"),
            "expected unchanged after conversion, got:\n{}",
            report2.render()
        );
    }

    #[test]
    fn sync_instructions_requires_force_for_real_file() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let home = dir.path();
        let _ = fs::create_dir_all(home.join(".claude"));
        let _ = fs::create_dir_all(home.join(".codex"));
        let _ = fs::write(home.join(".claude/CLAUDE.md"), "from claude\n");
        let _ = fs::write(home.join(".codex/AGENTS.md"), "old codex copy\n");

        let report = match sync(&SyncOptions {
            from: ToolId::Claude,
            to: vec![ToolId::Codex],
            kinds: SyncKinds {
                instructions: true,
                skills: false,
                mcp: false,
            },
            dry_run: false,
            prune: false,
            force: false,
            home: Some(home.to_path_buf()),
        }) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        assert!(!report.success(), "expected conflict without --force");
        assert!(
            report.render().contains("instructions: ERROR"),
            "expected error line, got:\n{}",
            report.render()
        );

        let report_force = match sync(&SyncOptions {
            from: ToolId::Claude,
            to: vec![ToolId::Codex],
            kinds: SyncKinds {
                instructions: true,
                skills: false,
                mcp: false,
            },
            dry_run: false,
            prune: false,
            force: true,
            home: Some(home.to_path_buf()),
        }) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        assert!(report_force.success(), "{}", report_force.render());
        let link = home.join(".codex/AGENTS.md");
        assert!(
            link.symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        );
    }

    #[test]
    fn diff_instructions_shows_paths_then_content() {
        let dir = match tempdir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let home = dir.path();
        let _ = fs::create_dir_all(home.join(".claude"));
        let _ = fs::create_dir_all(home.join(".codex"));
        let _ = fs::write(home.join(".claude/CLAUDE.md"), "alpha\n");
        let _ = fs::write(home.join(".codex/AGENTS.md"), "beta\n");

        let out = match diff(ToolId::Claude, ToolId::Codex, Some(home.to_path_buf())) {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        assert!(out.contains("## Instructions"), "{out}");
        assert!(out.contains("claude:"), "{out}");
        assert!(out.contains("codex:"), "{out}");
        assert!(out.contains("alpha") || out.contains("beta"), "{out}");
    }
}
