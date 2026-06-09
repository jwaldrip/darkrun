# Tools and commands

darkrun has two surfaces. You drive the run with **slash commands** in your agent
harness; under the hood, the manager drives the engine with **MCP tools**. You
rarely call the tools yourself — but knowing they exist is how you understand
what the manager is actually doing on your behalf.

:::columns
- **Slash commands** — what *you* type. `/darkrun:darkrun-new`, `/darkrun:darkrun-resume`, `/darkrun:darkrun-checkpoint`. High-level intents the manager turns into work.
- **MCP tools** — what the *manager* calls. `darkrun_run_start`, `darkrun_tick`, `darkrun_question`. The low-level contract the engine exposes over the local MCP server.
:::

## Commands you type

The commands are the run lifecycle. Start one, let the manager walk it, show up
at the gates. The full catalog lives in [Workflows](/guides/workflows); the ones
you'll reach for most:

:::keypoints title="The everyday loop"
- **darkrun-new** — describe what you want; the manager scaffolds a right-sized run.
- **darkrun-resume** — advance the run; returns the next concrete action.
- **darkrun-checkpoint** — decide a station's gate: approve to advance, or request changes (routed back as drift).
- **darkrun-inspect** — show the run's state: stations, units, criteria, checkpoint status.
- **darkrun-dark** — run lights-out: pre-elaborate up front, then advance without stopping except on external/await gates and real ambiguity.
- **darkrun-zap** — zero-ceremony single-unit execution; nothing written under `.darkrun/`.
:::

Setup and housekeeping — `darkrun-setup`, `darkrun-factories`, `darkrun-scaffold`,
`darkrun-backlog`, `darkrun-reset`, `darkrun-debug`, `darkrun-migrate`,
`darkrun-statusline`, `darkrun-version`, `darkrun-report`, `darkrun-changelog` —
round out the surface. Same names in every harness; see [Other harnesses](/docs/other-harnesses).

## Tools the manager calls

Every tool is namespaced `darkrun_*` and validated against a schema. They group
by what they touch:

:::keypoints title="Run lifecycle"
- **darkrun_run_start** — begin a run on a factory; records the brief, mode, and size.
- **darkrun_tick** — the cursor walk: returns the next action, you perform it, then re-tick.
- **darkrun_run_inspect** — push the live Review session (stations, units, phase) to the desktop app.
- **darkrun_run_surface** — record the run's classified surface (library / api / web-ui / cli / …), which routes how Shape designs and Prove verifies.
- **darkrun_run_list** / **darkrun_run_archive** / **darkrun_run_reset** — enumerate, retire, or wipe runs.
:::

:::keypoints title="Decomposition and passes"
- **darkrun_unit_create** / **darkrun_unit_list** / **darkrun_unit_get** — the unit dependency graph with completion criteria.
- **darkrun_unit_iterate** — run a unit's Make → Challenge → Resolve pass loop.
- **darkrun_unit_update** / **darkrun_unit_reset** — record progress or re-open a unit.
:::

:::keypoints title="The desktop surfaces (visual sessions)"
- **darkrun_question** — pose a visual multi/single-select decision; options carry `image_url` (+ optional `image_url_light` for theme-matched previews).
- **darkrun_direction** — present design archetypes to pick and annotate.
- **darkrun_picker** — a blocking selection among plain options.
- **darkrun_annotation_submit** — record pins + comments on a chosen direction.
- The `*_result` twins (`darkrun_question_result`, `darkrun_direction_result`, `darkrun_picker_result`) read the operator's answer back.
:::

:::keypoints title="Gates, feedback, and evidence"
- **darkrun_checkpoint_decide** / **darkrun_checkpoint_choose** — resolve a station's gate.
- **darkrun_gate_review** — the pre-checkpoint multi-reviewer code-review pass.
- **darkrun_feedback_create** / **_list** / **_move** / **_resolve** / **_reject** — the feedback track that routes rework as drift.
- **darkrun_review_stamp** / **darkrun_run_review_stamp** — a reviewer's per-role sign-off.
- **darkrun_proof_attach** / **darkrun_proof_get** / **darkrun_quality_gate_record** — the objective evidence Prove and Harden lock.
:::

:::callout
You don't memorize the tools. You drive with the commands, watch the manager
work in the desktop app, and decide at the gates. The tool list is just the
honest accounting of what "the manager keeps every station honest" actually means.
:::
