# agent-bridge

Sync **instructions**, **skills**, and **MCP** configs across AI coding tools at **user-global** scope.

Supported tools:

| Tool | Instructions | Skills | MCP |
|------|--------------|--------|-----|
| `claude` | `~/.claude/CLAUDE.md` | `~/.claude/skills/` | `~/.claude.json` → `mcpServers` |
| `codex` | `~/.codex/AGENTS.md` | `~/.agents/skills/` | `~/.codex/config.toml` → `[mcp_servers.*]` |
| `opencode` | `~/.config/opencode/AGENTS.md` | `~/.config/opencode/skills/` | `~/.config/opencode/opencode.json` → `mcp` |
| `cursor` | *(unsupported)* | `~/.cursor/skills/` | `~/.cursor/mcp.json` → `mcpServers` |

Design:

- **Arbitrary direction**: pick any `--from` and one or more `--to` tools (no fixed source of truth).
- **Skills**: target side gets a **symlink** to the source skill's real path.
- **MCP**: converted through an internal IR (JSON / TOML / OpenCode `local|remote`), with env syntax rewritten (`${VAR}` ↔ `${env:VAR}` ↔ `{env:VAR}`).
- **Safe merge**: only MCP-related keys are updated; other fields in shared config files are preserved.
- **Scope**: user-global only (no project-level paths in this release).

## Install

```bash
cargo install --path crates/agent-bridge
```

Or run from the repo:

```bash
cargo run -p agent-bridge -- --help
```

## Usage

```bash
# Sync everything from Claude Code to the other three tools
agent-bridge sync --from claude --to cursor,codex,opencode

# Preview only
agent-bridge sync --from claude --to cursor --dry-run

# Skills + MCP only; replace conflicting skill paths
agent-bridge sync --from cursor --to claude --only skills,mcp --force

# Strict MCP/skill mirror (delete extras on targets)
agent-bridge sync --from claude --to cursor --prune

# Diff / status / list
agent-bridge diff --from claude --to cursor
agent-bridge status
agent-bridge list --tool claude
```

### Flags

| Flag | Meaning |
|------|---------|
| `--from <tool>` | Source tool (`claude`, `codex`, `opencode`, `cursor`) |
| `--to <a,b,...>` | Target tools (must not include `--from`) |
| `--only <kinds>` | `instructions`, `skills`, `mcp` (default: all) |
| `--dry-run` | Print plan / diffs; write nothing |
| `--prune` | Remove target MCP servers and attributable skill symlinks absent from source |
| `--force` | Replace conflicting skill directories/symlinks |

## Notes

- Cursor User Rules (Settings → Rules) have no stable file API and are cloud-backed; agent-bridge therefore **does not sync instructions for Cursor**. Skills and MCP for Cursor are still synced. When `instructions` is requested and Cursor is source or target, that kind is skipped with a clear status line.
- Claude user-scope MCP lives in the top-level `mcpServers` key of `~/.claude.json` (project-local entries under path keys are left alone).
- Symlinks always point at the **canonical** source skill directory to avoid A→B→C chains.
- Unix only for skill symlinks in this release.

## Development

```bash
cargo test
cargo run -p agent-bridge -- status
```

## License

MIT
