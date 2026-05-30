{% include "_shared/announcement.md" %}

# Run Completion — `{{ run }}`

Every station is locked. Before the run seals, do the run-level pass: the whole-run reviewers and the reflections. This is the last chance to catch something that no single station could see on its own.

{% include "_shared/contracts.md" %}

## Run-level review

{% if reviewers %}
Dispatch the whole-run reviewers — they look *across* stations, not within one:
{% for r in reviewers %}
- `{{ r }}`
{% endfor %}
Each checks for cross-station seams: integration gaps, regressions, security holes that only appear when the parts meet.
{% else %}
No run-level reviewers are configured for this factory.
{% endif %}

## Reflections

{% if reflections %}
Run the reflections — these capture what the run *learned*, for the next one:
{% for r in reflections %}
- `{{ r }}`
{% endfor %}
{% else %}
No reflections are configured for this factory.
{% endif %}

## Done when

Run-level reviewers have signed off (or routed feedback), reflections are recorded, and nothing cross-station is left open. Then call `run_next` to seal the run.
