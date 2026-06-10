---
name: Bound the dead-TS-to-darkrun reconciliation so later stations don't port the brief literally
unit_type: ''
status: completed
depends_on:
- frame-protocol-problem
worker: distiller
station: frame
branch: darkrun/darkrun-sim/units/frame/frame-reconciliation-bound
started_at: 2026-06-10T06:39:35.599643+00:00
completed_at: 2026-06-10T06:40:37.058336+00:00
iterations:
- worker: framer
  started_at: 2026-06-10T06:39:35.599643+00:00
  completed_at: 2026-06-10T06:39:35.599643+00:00
  result: advance
  note: 'Make pass verified frame.md''s "Reconciliation note" against this unit''s criterion: the dead-TS→darkrun mapping table maps each dead brief assumption to its darkrun replacement, each row citing a real seam. Confirmed rows: TS package→crates/darkrun-sim; runWorkflowTick/buildRunInstructions→run_tick/derive_position/render_prompt/adapt_tick; handleStateTool→rmcp #[tool_router] on DarkrunServer; <subagent> blocks + next_subagent_dispatch_block relay→DOES NOT EXIST (engine emits RunAction + rendered prompt; agent spawns subagents itself); resolveStatuslineState/StatuslineState→absent, ANSI rendered inline from StateStore (darkrun-cli/src/statusline.rs); web-llm/WebGPU→dead (no wasm, zero LLM code), dumb agent runs locally at record time; payloadFor() website sync→absent, replay ships into web/site Dioxus SSG; git CLI scratch repo→pure-Rust gitoxide darkrun-git/StateStore. The single most load-bearing row (the missing subagent-block relay) was independently confirmed in Rust by the feasibility reviewer. For challenger: probe whether any mapping is wrong or any dead assumption is unlisted that could let a later station port the brief literally. Do NOT rewrite the locked artifact.'
- worker: challenger
  started_at: 2026-06-10T06:40:01.928930+00:00
  completed_at: 2026-06-10T06:40:01.928930+00:00
  result: advance
  note: 'Challenge pass attacked the reconciliation table. Each mapping is correct and seam-cited; the load-bearing one (no <subagent>/next_subagent_dispatch_block relay) was verified in Rust by both an explorer and the feasibility reviewer. Probed for unlisted dead assumptions that could let a later station port the brief literally: checked the brief''s other TS-isms — the record/replay split (decision #6) is preserved not dead (operator kept it: record locally, replay deployed), and the six-panel player is mapped to Dioxus, not dropped. No missing row that would mislead Shape. One nit, non-blocking: the table doesn''t explicitly call out that the brief''s "harness runs an agent tool-loop / spawns <subagent> blocks the instruction contains" is the SAME dead mechanism as the relay row — but the relay row already covers it, so Shape can''t be misled. No defect filed; advancing to distiller.'
- worker: distiller
  started_at: 2026-06-10T06:40:20.861034+00:00
  completed_at: 2026-06-10T06:40:20.861034+00:00
  result: advance
  note: 'Resolve pass reconciled make + challenge. The reconciliation table maps every dead brief assumption to its darkrun replacement with a cited seam; challenge confirmed no mapping is wrong and no dead assumption is unlisted that could mislead a later station. The one nit (the dead "harness spawns <subagent> blocks" mechanism) is already subsumed by the relay row, so no rewrite needed — and the locked artifact passed value+feasibility review unchanged. Unit frame-reconciliation-bound satisfies its criterion: the dead-TS→darkrun reconciliation is bounded so later stations don''t port the brief literally. Done.'
reviews:
  feasibility:
    at: 2026-06-09T23:10:04.256406+00:00
  value:
    at: 2026-06-09T23:09:26.862330+00:00
approvals:
  value:
    at: 2026-06-10T06:41:51.201191+00:00
---

# Bound the dead-TS-to-darkrun reconciliation so later stations don't port the brief literally
