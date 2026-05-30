---
name: darkrun-version
description: Show the running darkrun engine version, plugin version, build kind (compiled bundle vs dev source), runtime, and entry point — for triaging behavior that doesn't match the docs
---

# Version

Report what darkrun build is actually running.

Call `darkrun_version_info` and display:

- **Engine version** — the version baked into the running binary at build time (or `dev` when the
  binary wasn't built).
- **Plugin version** — the version from `plugin.json` on disk.
- **Build kind** — whether the running engine is the compiled production binary or a dev build run
  straight from source. These can disagree with the plugin version when the dev tree has uncommitted
  or unbuilt changes.
- **Runtime** — the toolchain and version running the engine (e.g. `rustc`/cargo profile) — useful
  when triaging "why does my behavior differ from the docs."
- **Entry** — the actual binary the process launched.

If a pending update is reported, mention it. When the build kind is `dev`, call out that working-tree
edits to the engine source are live only after a rebuild (`cargo build`) — there's no hot reload for
a compiled binary.
