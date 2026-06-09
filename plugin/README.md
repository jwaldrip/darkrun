# darkrun (Claude Code plugin)

**darkrun** structures AI-assisted work as a **factory**: a **Run** moves through **Stations**,
each staffed by **Workers** (with **Explorers** gathering context and **Reviewers** verifying
output) and gated by a **Checkpoint**. The engine is a single native Rust binary — no JS runtime.

Hierarchy: **Factory › Station › Unit › Pass**.

## Install

```
/plugin marketplace add darkrun-ai/darkrun
/plugin install darkrun
```

This installs the `darkrun` npm package, which carries the **per-arch native binary** as an
optionalDependency (`@darkrun/<os>-<arch>`). The `bin/darkrun` shim execs the matching binary;
nothing is interpreted at runtime.

Standalone (outside Claude Code):

```
npm i -g darkrun           # native binary, no JS app
```

## Commands → MCP tools

| Command | MCP tool | Purpose |
|---|---|---|
| `/darkrun:darkrun-new` | `darkrun_run_new` | Start a new Run; the manager scaffolds the lifecycle |
| `/darkrun:darkrun-resume` | `darkrun_advance` | Advance — the manager returns the next action |
| `/darkrun:darkrun-inspect` | `darkrun_run_inspect`, `darkrun_unit_list` | Show Run state, Stations, Units |
| `/darkrun:darkrun-factories` | `darkrun_factory_list` | List available factories |
| `/darkrun:darkrun-checkpoint` | `darkrun_checkpoint_decide` | Review and decide a Station's Checkpoint |

The MCP server is started by Claude Code via `.mcp.json` (`darkrun mcp`). The **manager** (the
run/station loop) drives everything from `darkrun_advance`; you follow the actions it returns.

## Software factory stations

`Frame → Specify → Shape → Build → Prove → Harden` — ordered by cost-of-late-discovery, each
killing one class of rework and locking one durable artifact. Run-start auto right-sizes small
Runs (a one-liner collapses to Build → Prove).
