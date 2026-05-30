---
name: darkrun-zap
description: Zero-ceremony single-Unit execution — run one task straight through a Station's Worker loop with no Run, no decomposition, and no state written under .darkrun/
---

# Zap

Drive a single task straight through a Station's Worker loop — no Run record, no Unit
decomposition, no manager tick. It is **stateless**: nothing is written under `.darkrun/`. Reach for
it on bug fixes, typos, config tweaks, and small refactors where the cost of a mistake is "edit and
re-run."

## Process

1. Call `darkrun_zap { task: "<the task>", factory?, station? }`.
2. `factory` / `station` are optional. Omit them to default to the build-class Station of the
   software factory (Build).
3. If the tool returns `zap_factory_not_found` or `zap_station_not_found`, surface the
   `valid_factories` / `valid_stations` list with `AskUserQuestion`, let the user pick, and call
   `darkrun_zap` again with their choice.
4. Otherwise follow the returned `message` verbatim. It carries the resolved Factory/Station, the
   ordered Worker sequence (Make → Challenge → Resolve) with each Worker's role, a ready-to-spawn
   subagent prompt per Worker, and the full run/verify/commit procedure — preflight, sequential
   dispatch, PASS/FAIL parsing, retry cap, and commit-only-on-PASS.

## When not to zap

If the task spans more than one Station or needs its own completion criteria tracked over time,
stop and use `/darkrun:darkrun-quick` (single Station, stateful) or `/darkrun:darkrun-start` (full
Run) instead. Zap is for work that fits in one Worker loop.
