---
name: darkrun-report
description: Submit feedback or a bug report about darkrun itself — synthesize the user's experience into a structured, actionable report
---

# Report

Send feedback or a bug report about darkrun to the maintainers.

1. Ask the user what they want to share — what happened, what they expected, and any steps to
   reproduce.
2. Synthesize it into a clear, actionable report. Do **not** submit their words verbatim — fold them
   into a structured summary with context: what they were doing, what went wrong, and the expected
   behavior.
3. Show the user the summary and confirm it looks right before submitting.
4. Call `darkrun_report` with the synthesized `message`. Include `contact_email` and `name` only if
   the user offers them.

This is for feedback about darkrun the tool — not for routing rework inside a Run. To file rework
against a Station, use the manager's feedback track (`darkrun_feedback_create`).
