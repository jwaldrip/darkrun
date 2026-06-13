---
name: darkrun-new
description: Start a new darkrun Run — describe what you want to accomplish and the factory manager scaffolds a right-sized lifecycle for it
---

Capture the intent cleanly, then right-size the lifecycle to the work. If the request is vague, prelaborate it into a crisp description first — that's free-text elaboration, so `AskUserQuestion` is fine for it.

The two operator SELECTIONS — factory and review mode — are surfaced in the desktop as **visual pickers**, not inline text asks. This is a darkrun convention: an operator choice with a fixed option set is a `darkrun_picker` so it shows in the control room (and brings the desktop up if it isn't open), exactly as the predecessor surfaced studio/mode selection. Derive the run `slug` first (you need it to scope the picker), then:

1. **Factory** — if it isn't obvious from the request, `darkrun_factory_list` for the catalog, then surface `darkrun_picker { slug, kind: "factory", title, prompt, options }` and read the choice with `darkrun_picker_result`. (Skip the picker only when the factory is unambiguous, e.g. `software`.)
2. **Review mode** — surface `darkrun_picker { slug, kind: "mode", title, prompt, options: [solo, team, dark] }` and read the choice. Don't ask for the mode with `AskUserQuestion` — it's a fixed-option operator selection, so it belongs in the picker.

Then `darkrun_run_new { slug, title, factory, mode, size }` with the picked values. Drive the loop with `/darkrun:darkrun-resume`.

Right-size with `--size full|quick|bugfix|refactor` (the station plan) — this is YOUR judgment from the problem during prelaboration, not an operator picker:

- `full` (default) — the whole Frame→Harden line.
- `quick` — build + prove, for small self-contained work.
- `bugfix` — specify + build + prove.
- `refactor` — shape + build + prove.

The review `mode` options (orthogonal to size), for the picker you surface:

- `solo` (default) — each station asks for local review before advancing.
- `team` — each station opens a PR/MR the team reviews and merges (`darkrun/<slug>/<station>` → `darkrun/<slug>/main`); the manager advances when you merge it. Needs a hosting client (`gh`/`glab` on PATH, configured via `/darkrun:darkrun-setup`); without one, a station gate falls back to a manual review gate you resolve by hand.
- `dark` — pre-elaborate up front, then run without stopping for review.
