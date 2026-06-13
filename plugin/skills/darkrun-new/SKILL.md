---
name: darkrun-new
description: Start a new darkrun Run — describe what you want to accomplish and the factory manager scaffolds a right-sized lifecycle for it
---

Capture the intent cleanly. If the request is vague, prelaborate it into a crisp `title` first — that's free-text, so `AskUserQuestion` is fine for it.

Then `darkrun_run_new { slug, title }` and drive `darkrun_advance`. **Don't pass `factory`, `mode`, or `size`** — those are engine-managed operator selections. The engine elicits them as visual pickers in the desktop (factory → mode → size), one per `darkrun_advance`, then materializes the run and walks the line. Follow the tool's instructions: it reports which selection the operator is making and when to advance.

Drive the loop with `/darkrun:darkrun-resume`.
