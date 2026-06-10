---
name: Frame the protocol-fidelity problem, user, value, and single success metric
unit_type: ''
status: in_progress
depends_on: []
worker: distiller
station: frame
branch: darkrun/darkrun-sim/units/frame/frame-protocol-problem
started_at: 2026-06-10T06:35:07.265893+00:00
iterations:
- worker: framer
  started_at: 2026-06-10T06:35:07.265893+00:00
  completed_at: 2026-06-10T06:35:07.265893+00:00
  result: advance
  note: 'Make pass verified frame.md''s Problem/User/Value/Success-metric sections against this unit''s criterion. Problem: the only test of darkrun''s no-agent-mechanics bet fakes the agent (run_to_seal stamps state directly, never reads the prompt). User: the engine developer hardening the protocol. Value: closes the one untested gap; protects real users from stranded runs; bonus replayable demo. Success metric is single + observable: a no-privileged-knowledge agent reaches RunAction::Sealed from emitted prompts alone, deadlock::check never fires, no empty prompt/handoff/nonce, walk persisted + replayed; green = protocol flowed, never "it compiles." For challenger: attack the metric for measurability holes (is "stranded" fully operationalized?), confirm Problem is evidenced not asserted, and check the User is concrete. Do NOT rewrite the locked artifact — it already passed value+feasibility review; only file a real defect if one exists.'
- worker: challenger
  started_at: 2026-06-10T06:36:22.538970+00:00
  completed_at: 2026-06-10T06:36:22.538970+00:00
  result: advance
  note: 'Challenge pass red-teamed the framing on three axes. (1) Metric measurability: "stranded" IS operationalized — frame.md''s "Stranded-agent signal (concrete)" section names four checkable triggers (deadlock::check fires, N consecutive ticks with no state change, advance called without satisfying the prior action''s predicate, or a tool call nonsensical for the current action tag). No hand-waving. (2) Problem evidenced not asserted: backed by a named file (crates/darkrun-e2e/tests/common/mod.rs::run_to_seal) and the verifiable fact that it mutates state directly without reading the rendered prompt — feasibility reviewer independently confirmed this against the Rust. (3) User concrete: "the darkrun engine developer hardening the protocol" with a specific job-to-be-done (a red/green followability signal). One residual nit, NOT a blocker: the metric bundles four pass/fail conditions under one "green" — acceptable because they share a single observable (the run reaches Sealed without any strand trigger firing). No defect filed; advancing to distiller. Distiller: confirm make+challenge reconciled, nothing to rewrite in the locked artifact.'
reviews:
  feasibility:
    at: 2026-06-09T23:10:04.256406+00:00
  value:
    at: 2026-06-09T23:09:26.862330+00:00
---

# Frame the protocol-fidelity problem, user, value, and single success metric
