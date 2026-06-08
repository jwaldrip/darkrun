# Changelog

All notable changes to darkrun are recorded here. Versions follow semver.

## [0.2.0](https://github.com/darkrun-ai/darkrun/compare/v0.1.0...v0.2.0) (2026-06-08)


### Features

* darkrun — factory-orchestration engine, design system, website, and plugin ([f6365d8](https://github.com/darkrun-ai/darkrun/commit/f6365d812cf4bd730c9af79147954fd3bf9356cd))
* **git:** complete Phase 1 gix reads — ls_tree, unresolved_paths, list_worktrees ([b33e131](https://github.com/darkrun-ai/darkrun/commit/b33e1318ec367af7571dff1536c4d81cd4b8a434))
* **git:** gix add_paths + checkout_paths — Phase 2 complete ([47ff4b3](https://github.com/darkrun-ai/darkrun/commit/47ff4b364f32ecd0344704c1861f5f17f0c9bcf2))
* **git:** gix create_branch (Phase 2 start) — idempotent ref write ([b5d7267](https://github.com/darkrun-ai/darkrun/commit/b5d7267bfb6fca912b6e6230e2b639c113661cc6))
* **git:** gix engine-protected three-way merge (Phase 4 — the core safety net) ([5346434](https://github.com/darkrun-ai/darkrun/commit/5346434f2807bd7d6ed4c189914e0868a17b380e))
* **git:** gix linked-worktree create/remove (Phase 3 — first gitoxide gap) ([233a16b](https://github.com/darkrun-ai/darkrun/commit/233a16bbae8deb584dc49f30138d7a3fbec6ec64))
* **git:** gix native fetch (Phase 5) — pure-Rust transport, C-free ([6955549](https://github.com/darkrun-ai/darkrun/commit/69555491cb7cec5f78d6db39809cbdbc0dca62c2))
* **git:** gix reads — is_ancestor, refs_have_identical_trees, merge_in_progress ([c074ed0](https://github.com/darkrun-ai/darkrun/commit/c074ed05688954c32044741d1da3f832e717c4d7))
* **git:** hand-rolled write-tree + gix commit (fork-A internals) ([a7e01b6](https://github.com/darkrun-ai/darkrun/commit/a7e01b6dd23af36a49a9ff22f791803a8981c435))
* **git:** scaffold pure-Rust gitoxide backend (Phase 1 foundation) ([885462d](https://github.com/darkrun-ai/darkrun/commit/885462dd8be770255219ee5193bef7720c27a206))
* phase redesign + engine-driven prompts + hooks; Apache-2.0; dark-factory positioning ([5ccf3e9](https://github.com/darkrun-ai/darkrun/commit/5ccf3e9fb6bde91532201c6e33304725c61d8eb2))
* **verify:** objective surface-routed verification + view/visual-review + proof ([60062d9](https://github.com/darkrun-ai/darkrun/commit/60062d96dd94aca99f7cdf8a8d47bfc76b35a5b8))
* **visual:** visual-question + design-direction sessions, screens, and a visual-design step ([db0500e](https://github.com/darkrun-ai/darkrun/commit/db0500e3bc079ca7639471c0939b9a7b2ec3bd3d))


### Bug Fixes

* 0-byte outputs don't satisfy the gate; verify gate/drift loop immunity (predecessor BUGs 1, 3, drift A/B) ([9220710](https://github.com/darkrun-ai/darkrun/commit/92207108953e3bf732d31118726026b03efe607e))
* darkrun show deep-links to the run; stations render in factory order ([655d7a0](https://github.com/darkrun-ai/darkrun/commit/655d7a087c8e7c144e885cd444e4df4312a43195))
* derived_station_phase test needs a unit with a Pass signal (was asserting None-case) ([770ed56](https://github.com/darkrun-ai/darkrun/commit/770ed564a31248490c7c9ac0b5947e14a5792471))
* **plugin:** implement the darkrun hook subcommand so hooks never block tools ([5c3eb12](https://github.com/darkrun-ai/darkrun/commit/5c3eb125cc5105887be856d4b5b831e5e783289e))
* **site:** embed factory corpus in wasm builds + dx 0.7 config ([f8a05f3](https://github.com/darkrun-ai/darkrun/commit/f8a05f3957c4fd09594c3572d0bfff8b04fa7e3e))
* **ui:** stack the run walkthrough — station walker over a centered phase machine ([f4d040b](https://github.com/darkrun-ai/darkrun/commit/f4d040b66b53f3aa54ac84016d75d464c8877a48))

## 0.1.0 — unreleased

The first darkrun: a native Rust engine that drives Runs through a factory's
stations (Frame → Specify → Shape → Build → Prove → Harden for the software
factory), with a desktop review app and a Claude Code plugin.

- **Manager** — a pure-read cursor over `.darkrun/` state, walking the
  six-phase station machine (spec → review → manufacture → audit → reflect →
  checkpoint) across a three-track priority (drift → feedback → run).
- **Full action set** — validation (units-invalid, escalate, safe-repair),
  repair/rollback, external review, and the seal tail.
- **Objective verification** — surface-routed proof (`darkrun verify web`,
  `darkrun bench`) instead of eyeballed review.
- **Reflection** — durable run-level retrospectives.
- **Auto-tune** — run-start right-sizing (quick / bugfix / refactor / full).
- **Drift sweep** — detects mutated locked artifacts and self-heals on revert.
- **Multi-harness** — Claude Code, Cursor, Windsurf, Gemini CLI, OpenCode,
  Kiro, and Codex, each adapted from one capability registry.
