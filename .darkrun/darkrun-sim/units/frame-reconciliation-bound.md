---
name: Bound the dead-TS-to-darkrun reconciliation so later stations don't port the brief literally
unit_type: ''
status: in_progress
depends_on:
- frame-protocol-problem
worker: challenger
station: frame
branch: darkrun/darkrun-sim/units/frame/frame-reconciliation-bound
started_at: 2026-06-10T06:39:35.599643+00:00
iterations:
- worker: framer
  started_at: 2026-06-10T06:39:35.599643+00:00
  completed_at: 2026-06-10T06:39:35.599643+00:00
  result: advance
  note: 'Make pass verified frame.md''s "Reconciliation note" against this unit''s criterion: the dead-TS→darkrun mapping table maps each dead brief assumption to its darkrun replacement, each row citing a real seam. Confirmed rows: TS package→crates/darkrun-sim; runWorkflowTick/buildRunInstructions→run_tick/derive_position/render_prompt/adapt_tick; handleStateTool→rmcp #[tool_router] on DarkrunServer; <subagent> blocks + next_subagent_dispatch_block relay→DOES NOT EXIST (engine emits RunAction + rendered prompt; agent spawns subagents itself); resolveStatuslineState/StatuslineState→absent, ANSI rendered inline from StateStore (darkrun-cli/src/statusline.rs); web-llm/WebGPU→dead (no wasm, zero LLM code), dumb agent runs locally at record time; payloadFor() website sync→absent, replay ships into web/site Dioxus SSG; git CLI scratch repo→pure-Rust gitoxide darkrun-git/StateStore. The single most load-bearing row (the missing subagent-block relay) was independently confirmed in Rust by the feasibility reviewer. For challenger: probe whether any mapping is wrong or any dead assumption is unlisted that could let a later station port the brief literally. Do NOT rewrite the locked artifact.'
reviews:
  feasibility:
    at: 2026-06-09T23:10:04.256406+00:00
  value:
    at: 2026-06-09T23:09:26.862330+00:00
---

# Bound the dead-TS-to-darkrun reconciliation so later stations don't port the brief literally
