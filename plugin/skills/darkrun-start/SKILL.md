---
name: darkrun-start
description: Start a new darkrun Run — describe what you want to accomplish and the factory manager scaffolds a right-sized lifecycle for it
---

Capture the intent cleanly, then right-size the lifecycle to the work. If the request is vague, prelaborate it into a crisp description first (ask via `AskUserQuestion`). Call `darkrun_factory_list` if the factory isn't obvious, then `darkrun_run_start { slug, title, factory, mode }`. Pick `mode` from the work:

- `full` (default) — the whole Frame→Harden line.
- `quick` — build + prove, for small self-contained work.
- `bugfix` — specify + build + prove.
- `refactor` — shape + build + prove.
- `discrete` — the full line, but EVERY station's Checkpoint resolves on a human PR/MR merge: each station opens a draft PR (`darkrun/<slug>/<station>` → `darkrun/<slug>/main`) and the manager advances when you merge it. Pick this for high-oversight work where every station deserves explicit review.
- `discrete-hybrid` — continuous within stations, a per-station PR only on stations the factory marks for external review. Pick this for mostly-autonomous work that still wants a human merge on the release-critical station.

Discrete modes need a hosting client (`gh`/`glab` on PATH, configured via `/darkrun:darkrun-setup`); without one, a discrete Checkpoint falls back to a manual review gate you resolve by hand. Drive the loop with `/darkrun:darkrun-pickup`.
