
> **Run** `darkrun-sim` · **Station** `frame` · **Phase** `audit`

> Eliminates: _wrong-thing_


# Audit — `frame` output

The Units are manufactured. Now audit the *output* against the *spec* **and run the quality checks**. Manufacture proves the thing was built; audit proves the *right* thing was built — and proves it with evidence, not just judgment. (Audit folds in what a separate tests phase used to do: reviewers give judgment, the checks give evidence. Both happen here.)


**Contract**

- Do exactly the work this action describes — no more, no less. Don't skip ahead to a later phase.
- Treat the locked artifact (`frame.md`) as the source of truth. Read it before you act; never silently rewrite a locked decision.
- Every claim you make must be backed by something you actually ran, read, or wrote. No assumed results.
- Be specific and committed. **No placeholders** — a `TBD`, `similar to …`, `add error handling`, `etc.`, or `…` is a hole, not a decision; name the actual, checkable condition. **No hedging** — when you report work done, use a verb of completed action (`added`, `implemented`, `fixed`), never `should`, `seems`, `probably`, `might`, or `looks like`. Hedging is the tell of unfinished work.
- When the action is finished, record your output where the station expects it, then call `darkrun_tick` again for the next instruction. The manager — not you — decides what comes next.



**Explorers** (2): `context`, `value`


**Workers** (3): `framer` → `challenger` → `distiller`


**Reviewers** (2): `value`, `feasibility`


## Sub-steps

Audit walks two beats, in order:

### 1. spec — verify against the locked spec

Dispatch the reviewers (`value`, `feasibility`) **in parallel** — one subagent each, concurrently — over the manufactured output. Each asks:

- Does the output meet every Unit's **completion criteria** — not approximately, exactly?
- Did manufacture drift from the locked spec? Any silent scope creep?
- Does the combined output actually eliminate **wrong-thing**?
- Is anything fragile, unhandled, or quietly broken?

When a reviewer clears, it records its own approval with **`darkrun_review_stamp`** (`kind: approval`, its `role`) — stamping only its role without advancing the run, so the parallel pass never contends on the tick. A reviewer that finds a real problem files it with `darkrun_feedback_create` **instead of** stamping. You `darkrun_tick` once, after every reviewer returns.

**A reviewer verifies — it does not rebuild.** Each reviewer MUST NOT propose new requirements beyond the Unit's completion criteria, MUST NOT redesign the output or reopen the locked spec, and MUST NOT flag stylistic preference. It checks the output against the criteria and files what genuinely fails — nothing more.


Units to audit:

- `frame-protocol-problem`

- `frame-reconciliation-bound`

- `frame-scope-nongoals`



### 2. adversarial — adversarial reviewer pass + the checks

- Run the full check suite for this station's output — tests, type checks, lints, builds, whatever this station's discipline demands. Run it **completely**, not a subset. A partial or skipped run is not a pass; green means green across the board.
- If anything fails, it is yours to fix — failures here block the checkpoint. No "pre-existing" excuses; if it's red while this station owns the floor, fix it.
- Adversarially attack the output: where would this break? What did every reviewer miss? Capture the real evidence.



## The verdict — one state, no partial pass

Every check and every reviewer lands on exactly one verdict — never a partial, never a "mostly":

- **PASS** — you ran it and *observed* it meet the criteria. Not "looks right", not "should work" — observed.
- **FAIL** — you ran it and it does not meet the criteria. "3 of 4 green" is FAIL until the fourth is green or explicitly explained away.
- **BLOCKED** — you could not run it. BLOCKED is **not** a PASS; the checkpoint holds until it can run.
- **SKIP** — genuinely not applicable to this surface, with the reason stated.

When in doubt, **FAIL** — a false PASS ships broken work; a false FAIL costs one more look.

## Done when

Every reviewer has signed off or filed feedback, the full check suite passes, and the evidence is recorded against the station. Filed feedback becomes a fix-worker track — it does **not** get hand-waved past. Record the audit verdict (PASS only when every check passed and every reviewer cleared), then call `darkrun_tick`.