# Other harnesses

darkrun is an MCP server. It works with any MCP-capable agent harness, and it
**detects which harness it's running in** — adapting its tools, instructions,
prompts, and model tiers to that harness's capabilities. The durable Run state
under `.darkrun/` is harness-agnostic, so a Run started in one tool **resumes
cleanly in another**: the manager is a pure read of on-disk state, and the
harness only changes the surface, not the step.

Select the harness with `--harness <name>` (or the `DARKRUN_HARNESS` env).
Supported: `claude-code`, `cursor`, `windsurf`, `gemini-cli`, `opencode`,
`kiro`, `codex`.

## Per-harness setup

Each config launches `darkrun mcp --harness <name>`. The samples use
`npx -y darkrun` so they resolve the published per-arch binary regardless of
install location.

### Cursor

`.cursor/mcp.json` in your project:

```json
{
  "mcpServers": {
    "darkrun": { "command": "npx", "args": ["-y", "darkrun", "mcp", "--harness", "cursor"] }
  }
}
```

Cursor caps tools at ~40, so darkrun trims its visual tools to fit. Parallel
subagents and MCP elicitation are supported; review gates use elicitation rather
than the desktop app.

### Windsurf

`~/.codeium/windsurf/mcp_config.json`. No subagents (Units run sequentially) and
no elicitation (gates are workflow-enforced).

### Gemini CLI

`~/.gemini/settings.json`, or install the bundled extension
(`gemini-extension.json` ships the MCP config, a `GEMINI.md` context file, and
`.toml` slash commands). Skills surface as `/darkrun:*` slash commands; the
`haiku`/`sonnet`/`opus` model tiers map to `flash`/`pro`/`pro`.

### OpenCode

`opencode.json`. Subagents run sequentially; MCP prompts aren't surfaced, so the
agent reaches the tools directly.

### Kiro

`.kiro/agents/darkrun.yaml`. Like Claude Code: hooks, MCP elicitation, parallel
subagents, and slash commands are all supported.

### Codex

`~/.codex/config.toml` (or a project `.codex/config.toml`):

```toml
[mcp_servers.darkrun]
command = "npx"
args = ["-y", "darkrun", "mcp", "--harness", "codex"]
```

Single-agent (no parallel subagents), no hooks, native plan mode. Decisions are
made inline; reasoning effort maps to the `low`/`medium`/`high` tiers.

## Capability comparison

| Feature | Claude Code | Cursor | Windsurf | Gemini CLI | OpenCode | Kiro | Codex |
|---|---|---|---|---|---|---|---|
| Skills as slash commands | native | via prompts | via prompts | slash commands | direct tools | slash commands | direct tools |
| Parallel subagents | yes | yes | no | yes | no | yes | no |
| Hook system | yes | no | no | no | no | yes | no |
| MCP elicitation | yes | yes | no | no | no | yes | no |
| Browser review UI | yes | no | no | no | no | no | no |
| Tool budget | — | ~40 | ~100 | — | — | — | — |

## What every response tells you

Outside Claude Code, the hook-driven conveniences become manual — and each
`darkrun_run_next` response ends with a **Harness note** spelling out exactly
what applies to your harness:

- **No auto-context injection** — call `darkrun_run_next` at the start of each
  session to load the active Run.
- **No automatic output tracking** — register a Unit's outputs explicitly.
- **No browser review UI** — review gates fall back to elicitation or an inline
  text decision.
- **No parallel subagents** on some harnesses — Units run one at a time.

The engine still drives the Run; you do the bookkeeping the hooks would have.
