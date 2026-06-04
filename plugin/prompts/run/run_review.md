{% include "_shared/announcement.md" %}

# Run review — the whole-run audit

Every station has locked. Before the run seals, the **whole-Run reviewers** audit the *integrated* result — not one station's output, but the run end-to-end: the seams between stations, regressions in flows no station owned, and the attacker's view of the complete thing. This is the last guard against work that passed every station yet doesn't hold together.

{% include "_shared/contracts.md" %}

## Dispatch the run reviewers — in parallel

Dispatch the run reviewers{% if reviewers %} ({% for r in reviewers %}`{{ r }}`{% if not loop.last %}, {% endif %}{% endfor %}){% endif %} **in parallel** — one subagent each, concurrently. Each judges the integrated run on its dimension:

- **Integration** — do the stations' artifacts actually compose? Are the seams sound, or did each station satisfy its own spec while the joints leak?
- **Regression** — did building this break a flow no single station owned? What collateral damage is there to untouched behaviour?
- **Security / readiness** — the attacker's view of the *whole* result, and whether it's genuinely fit to ship.

A run reviewer that clears records its sign-off with **`darkrun_run_review_stamp`** (its `role`) — that stamps only its role without advancing, so the parallel pass never contends on the tick. A run reviewer that finds a cross-station problem files it with `darkrun_feedback_create` **instead of** stamping — the finding routes back as a fix, and the run holds here until it's clean. You call `darkrun_tick` once, after every run reviewer returns.

## Done when

Every declared run reviewer has stamped its sign-off (or filed a cross-station finding that's been resolved). Then call `darkrun_tick` — once the run-level review is clean, the run seals (or holds at its final `seal:` gate).
