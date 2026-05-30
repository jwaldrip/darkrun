---
name: shape
description: Shape the structure — decide the architecture and prove the risky assumptions cheaply before they get expensive.
explorers: [surface, architecture, risk]
workers: [designer, visual_designer, spiker, pressure_tester, resolver]
reviewers: [fit, reversibility, simplicity]
checkpoint: ask
locked_artifact: design.md
inputs: [frame.md, spec.md]
---

# Shape

Shape decides *how* the spec gets satisfied structurally. It kills the most
expensive class of late discovery: **structural reversal** — finding out, after
the code is written, that the whole approach was wrong and has to be torn out.
Structure is the most expensive thing to change late, so Shape pays a small cost
now (a throwaway spike) to avoid an enormous one later (a rewrite).

## Risk class eliminated

*Expensive structural reversal.* The spec is clear, but the chosen architecture
collides with reality only after significant code exists — wrong data model,
unworkable integration, an assumption that does not hold at scale.

## What this station produces

- **The classified surface** — the one shape the run delivers (library / api /
  web-ui / tui / cli / desktop / mobile / data), recorded onto the run via
  `darkrun_run_surface`. This is the linchpin: it routes both *how Shape
  designs* (a library gets public-API design; an api gets a contract; a web-ui
  gets visual + component design; a tui gets terminal layout; a cli gets
  command/output UX; data gets structural design) *and how Prove/Audit verify*
  (visual surfaces → headless-browser proof; bench surfaces → criterion + load;
  terminal surfaces → output snapshot).
- **The design** — components, boundaries, data flow, the integration points,
  and the key decisions with their rationale, shaped to the classified surface.
- **The visual direction** — for user-facing work, the chosen UI/UX archetype the
  operator picked, captured with its option images and annotations. (For internal or
  headless work there is no visual surface, so this is empty.)
- **Spike results** — the output of a throwaway proof that the riskiest
  assumptions actually hold. Spikes are deleted after; only their findings survive.

## The pass-loop

- **Designer** classifies the surface from the Surface Explorer's finding, records it with `darkrun_run_surface`, then proposes the structure that satisfies the spec with the least machinery — designing *for that surface* (public-API / contract / visual+component / terminal layout / command-output UX / structural).
- **VisualDesigner** owns the visual/UX facet for user-facing work: it generates two to four design options (mockups / option images) and uses `darkrun_question` or `darkrun_direction` to get the operator's visual decision *before* any UI is built. For non-UI work there is no surface to shape, so this beat is skipped.
- **Spiker** builds a *throwaway* proof of the riskiest assumptions — the thing most likely to be wrong — and reports what it learned. The spike code is discarded; the knowledge is kept.
- **PressureTester** attacks the design under load, failure, and change: what reverses this, what does not scale, what is hard to undo?
- **Resolver** reconciles the spike findings and pressure tests into the final design.

## Locked artifact

`design.md` + spike results — the structure plus evidence the risky parts work.
Build inherits this and may not re-litigate the architecture; a structural change
is drift that routes back here.

## Checkpoint

**ask.** A human signs off on the structure (and may route it **external** for a
formal design review on high-stakes systems) before Build commits to it.
