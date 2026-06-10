---
name: Lock full-brief scope and non-goals; separate protocol-green from build-green
unit_type: ''
status: in_progress
depends_on:
- frame-protocol-problem
worker: distiller
station: frame
branch: darkrun/darkrun-sim/units/frame/frame-scope-nongoals
started_at: 2026-06-10T06:39:41.048462+00:00
iterations:
- worker: framer
  started_at: 2026-06-10T06:39:41.048462+00:00
  completed_at: 2026-06-10T06:39:41.048462+00:00
  result: advance
  note: 'Make pass verified frame.md''s "Non-goals" + "Scope decision (operator)" against this unit''s criterion: full-brief scope is locked and non-goals are enumerated with protocol-green separated from build-green. Confirmed non-goals: (1) build quality — dumb-agent code won''t compile, software-factory quality gates stubbed, protocol fidelity is the only test; (2) not proving artifacts shippable; (3) no live engine+model per website visitor (record locally, deploy static replay; no WebGPU); (4) not replacing the existing e2e cursor-progression test (additive). Scope: operator chose all four brief phases; dumb agent runs locally to build the walk, persisted, deployed for replay. The protocol-green-vs-build-green split is explicit in both the Success metric and non-goal (1). For challenger: probe whether the non-goals do real bounding work (is full-brief scope still achievable, or does a non-goal leak?), and whether protocol-green is unambiguously distinguished from build-green. Do NOT rewrite the locked artifact.'
- worker: challenger
  started_at: 2026-06-10T06:40:07.780048+00:00
  completed_at: 2026-06-10T06:40:07.780048+00:00
  result: advance
  note: 'Challenge pass attacked the scope/non-goals. Non-goals do real bounding work: stubbing software-factory quality gates is what makes full-brief scope achievable (without it, "dumb agent builds shippable code" would be impossible and the metric unreachable) — feasibility reviewer confirmed none of the four phases hides an impossibility. Protocol-green vs build-green is unambiguous: stated in both the Success metric ("green = protocol flowed, never it compiles") and non-goal (1). Probed for leaks: full-brief scope is large but each phase is independently demoable and the non-goals fence off exactly the expensive impossibilities (per-visitor inference, shippable code). One residual tension, non-blocking: "full brief" + "nothing cut" sits against a large surface, but that''s a Shape/Specify sizing concern, not a frame defect — the frame''s job is to bound wrong-thing, which it does. No defect filed; advancing to distiller.'
reviews:
  feasibility:
    at: 2026-06-09T23:10:04.256406+00:00
  value:
    at: 2026-06-09T23:09:26.862330+00:00
---

# Lock full-brief scope and non-goals; separate protocol-green from build-green
