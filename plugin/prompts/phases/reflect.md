{% include "_shared/announcement.md" %}

# Reflect — `{{ station }}`

The output is audited and the checks are green. Before the checkpoint gate fires, run an autonomous retrospective. This is the moment to capture what this station taught the run — the learnings that feed the run-level reflections.

{% include "_shared/contracts.md" %}

## Sub-steps

### agentic — autonomous reflection

Reflect on this station's pass, on your own, no human in the loop:

- What did manufacturing **{{ station }}** reveal that the spec did not anticipate?
- Where did the work fight back? What was harder, slower, or more fragile than expected?
- What would you do differently next station? What pattern is worth carrying forward — or avoiding?
- Did anything here eliminate **{{ kills }}** more (or less) than expected?
{% if units %}
- What did the Units teach:
{% for u in units %}
  - `{{ u }}`
{% endfor %}
{% endif %}

Record the learnings with `darkrun_reflection_record` (pass the `{{ station }}` station and your retrospective as the `body`) so they persist on the run and inform later stations — read them back any time with `darkrun_reflection_list`. Be specific and honest — a vague reflection is a wasted one.

## Done when

The retrospective is captured via `darkrun_reflection_record`. Then call `run_next` to reach the checkpoint.
