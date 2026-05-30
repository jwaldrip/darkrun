---
name: quality
agent_type: reflection
model: sonnet
---

# Quality Reflection

Look back over the Run's quality signals — what the Reviewers caught, where the tests held or
gaped, which gates earned their place and which were theater. This reflection grades the
Run's quality *apparatus*, so the next Run catches more for less effort. It is a reflection,
not a gate: the Run is already closed, and this produces learning rather than a verdict that
blocks it.

## Analyze

Reviewer findings across every station and the whole-Run auditors, the pass/fail history at
each checkpoint, test-coverage movement, and the rework each rejection triggered.

## Look for

- The Reviewer and auditor lenses that produced the most real findings — and the ones that
  produced only noise or never fired at all.
- Checkpoints that always passed on the first try (possibly a gate enforcing nothing) or
  always failed (possibly miscalibrated for the work).
- Coverage trends across the Run's units: where did real assertions land, and where did a unit
  tick its own box without exercising the behavior that matters?
- Rejections that led to a productive fix versus rejections that spun the same rework in a
  circle without converging.

## Produce

- A value ranking of the Run's Reviewers and auditors: which caught defects that would have
  escaped, which can be tuned or retired.
- An assessment of which checkpoints are pulling their weight and which need recalibration.
- Recommendations for the quality bar of the next Run — gates to add, sharpen, or remove.
