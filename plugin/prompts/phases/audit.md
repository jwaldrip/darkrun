{% include "_shared/announcement.md" %}

# Audit — `{{ station }}` output

The Units are manufactured. Now audit the *output* against the *spec* **and run the quality checks**. Manufacture proves the thing was built; audit proves the *right* thing was built — and proves it with evidence, not just judgment. (Audit folds in what a separate tests phase used to do: reviewers give judgment, the checks give evidence. Both happen here.)

{% include "_shared/contracts.md" %}

{% include "_shared/roster.md" %}

## Sub-steps

Audit walks two beats, in order:

### 1. spec — verify against the locked spec

Dispatch each reviewer{% if reviewers %} ({% for r in reviewers %}`{{ r }}`{% if not loop.last %}, {% endif %}{% endfor %}){% endif %} over the manufactured output. For each:

- Does the output meet every Unit's **completion criteria** — not approximately, exactly?
- Did manufacture drift from the locked spec? Any silent scope creep?
- Does the combined output actually eliminate **{{ kills }}**?
- Is anything fragile, unhandled, or quietly broken?

{% if units %}
Units to audit:
{% for u in units %}
- `{{ u }}`
{% endfor %}
{% endif %}

### 2. adversarial — adversarial reviewer pass + the checks

- Run the full check suite for this station's output — tests, type checks, lints, builds, whatever this station's discipline demands. Run it **completely**, not a subset. A partial or skipped run is not a pass; green means green across the board.
- If anything fails, it is yours to fix — failures here block the checkpoint. No "pre-existing" excuses; if it's red while this station owns the floor, fix it.
- Adversarially attack the output: where would this break? What did every reviewer miss? Capture the real evidence.

{% if user_facing %}
- **Surface = `{{ surface }}` (visual).** Verify objectively through a real headless browser: run `darkrun verify web` against the running output and capture the screenshot, the web vitals (LCP / FCP / CLS / TTFB / INP), and the a11y / contrast / touch-target / reduced-motion audits. Attach the measured `WebProof` with `darkrun_proof_attach`. An eyeballed "looks fine" is not evidence; the numbers are.
{% elif bench_surface %}
- **Surface = `{{ surface }}` (bench).** Verify objectively with `darkrun bench` plus the doc-tests: capture the latency percentiles (p50 / p95 / p99), throughput, and sample count, and attach the measured `BenchProof` with `darkrun_proof_attach`. A claim with no measured numbers is not evidence.
{% elif terminal_surface %}
- **Surface = `{{ surface }}` (terminal).** Verify objectively with a terminal/output snapshot of the real invocation, and attach it as a screenshot-bearing proof with `darkrun_proof_attach`.
{% endif %}

## Done when

Every reviewer has signed off or filed feedback, the full check suite passes, {% if surface %}the surface-routed objective proof is attached, {% endif %}and the evidence is recorded against the station. Filed feedback becomes a fix-worker track — it does **not** get hand-waved past. Record the audit verdict, then call `run_next`.
