{% include "_shared/announcement.md" %}

# Spec — `{{ label }}`

You are opening station **{{ station }}**. Its job is to eliminate a whole class of risk: **{{ kills }}**. Nothing downstream is allowed to proceed until that risk is named and bounded here.

{% include "_shared/contracts.md" %}

{% include "_shared/roster.md" %}

Spec runs **elaboration and discovery in tandem** — they are NOT two sequential
steps. The moment the station opens, kick off both at once: dispatch the explorers
in parallel *while* you frame the problem. They sharpen each other. Only once both
have landed do you decompose.

## elaborate — frame the problem (concurrently with discovery)

State plainly what this station must achieve to kill **{{ kills }}**: the intent, the inputs it inherits from upstream, and the boundary of what is explicitly *out of scope* so later phases don't drift into it. This is the frame the explorers work against — but do NOT wait on a finished frame to start them; the frame and the exploration are written in parallel and inform each other.

## discover — run the explorers in parallel (concurrently with elaboration)

Dispatch **all** explorers{% if explorers %} ({% for e in explorers %}`{{ e }}`{% if not loop.last %}, {% endif %}{% endfor %}){% endif %} **at once, in parallel** — one subagent each, fanned out concurrently, never one-after-another. Explorers don't build — they surface unknowns, constraints, prior art, and traps. They run alongside your framing; neither blocks the other.

{% if knowledge %}
**Project knowledge (priors from earlier runs)** — build on these, don't re-discover them:
{% for k in knowledge %}
- **{{ k.topic }}** — {{ k.body }}
{% endfor %}
{% endif %}

When discovery surfaces a durable project fact worth carrying into **future** runs — a constraint, prior art, a convention, a trap — persist it with **`darkrun_knowledge_record`** (`topic` + `body`). That's the project's shared memory; re-recording a topic updates it. Keep it project-level (cross-run truths), not this run's transient details.

## decompose — once elaboration + discovery have both landed

Turn the framed, explored problem into the smallest set of independently completable **Units** that, together, kill the risk above. A Unit's **body is the spec the executing subagent works from — it gets no other context**. A one-line body is a slug, not a definition; the work that comes back from a thin Unit is thin.

Write every Unit with `darkrun_unit_create`, with the full anatomy:

- **`body`** — the real definition, in markdown:
  - the goal: what this Unit produces and why it exists in this station,
  - **completion criteria, EACH paired with the literal command that verifies it.** Inspect the project's manifest (`Cargo.toml` / `package.json` / `pyproject.toml` / `go.mod` …) *during decompose* and write commands against THIS project's actual stack — never a placeholder.
    - Good: "all endpoints return correct status codes (200/400/401/404)" → `cargo test -p api contracts` exits 0.
    - Bad: "API works correctly", "tests are written" — no check, no criterion.
  - for build-class Units: the **success path, the failure path, and the edge cases** the criteria must cover,
  - for knowledge/document Units: substantive criteria — what claims the artifact must ground, with sources,
  - the **files touched** (so review knows the blast radius),
  - what is explicitly **out of scope** (so the Unit doesn't sprawl).
- **`depends_on`** — every cross-Unit prerequisite, DECLARED, never left in prose. The wave scheduler sequences **only** on `depends_on`; a dependency mentioned in the body but not declared is invisible — the Unit gets co-scheduled with its own prerequisite and handed inputs that don't exist yet. A body that says "stub it until unit-X lands" is the symptom of a missing `depends_on` edge: declare the edge instead of writing the stub.
- **`inputs` / `outputs`** — the paths consumed and produced. A sibling-produced input path requires that sibling in `depends_on`.
- **`quality_gates`** — executable `{name, command}` checks proving the criteria. Required for any Unit that declares outputs. Each gate must pass **in the Unit's own isolated worktree at the time it runs** — a gate that needs a sibling's unmerged code, with no `depends_on` edge to order it, is not a gate, it's a Unit scheduled to fail. Circular gates (zero-match `! grep`, prose substrings against the Unit's own output) are rejected.
- **`model`** — match the tier to the risk: `opus` for architectural or cascading-failure work, `sonnet` (default) for known patterns plus judgment, `haiku` only for purely mechanical edits.

{% if units %}
### Units already on record
{% for u in units %}
- `{{ u }}`
{% endfor %}
Reconcile these against what the explorers found — extend, split, or tighten them; don't blindly accept them.
{% else %}
There are no Units yet. You are creating them.
{% endif %}

{% if user_facing %}
### User-facing surfaces

This work touches a **user-facing surface**. For every Unit that renders a screen, flow, component, or page, mark it as visual so Shape's design step knows to act: its UI must not be built until the operator has chosen a design direction (via `darkrun_question` / `darkrun_direction`). Make the surface and its acceptance criteria explicit here; non-visual Units carry no such requirement.
{% endif %}

{% if needs_collaboration %}
## Collaborate with the operator — required before this spec locks

This run is in a **collaborative mode**, and the station will not advance to Review until you have actually involved the operator in shaping the spec. Do not author the whole spec solo and surface it only at the gate — bring the operator in *now*, while the frame is still soft:

- Surface the open framing questions and the consequential choices to the operator with `darkrun_question` (a decision) or `darkrun_direction` (a direction to steer), and fold their answers into the spec.
- When the spec genuinely reflects that collaboration, call **`darkrun_elaborate_seal`** for this station — that clears the hold and the next tick advances to Review.

If you advance without involving the operator, the station stays in Spec; a stalled, non-collaborative Spec escalates to the operator rather than slipping past them. (`dark` mode pre-elaborates once up front and doesn't gate here.)
{% endif %}

## Done when

The spec names the risk, lists Units with testable completion criteria and dependencies, marks what's out of scope,{% if needs_collaboration %} the operator has been involved and `darkrun_elaborate_seal` is called,{% endif %} and it's written to the station's spec artifact. Then call `darkrun_tick`.
