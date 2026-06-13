---
name: darkrun-new
description: Start a new darkrun Run — describe what you want to accomplish and the factory manager scaffolds a right-sized lifecycle for it
---

Call `darkrun_run_new` and follow the instructions it returns. The engine drives setup: with no name it returns the pre-elaboration cue (turn the request into a concise `title` + url-safe `slug`, then call back with them); with a name it creates the run and elicits factory → mode → size as visual pickers in the desktop, one per `darkrun_advance`, then starts the line.

Don't pass `factory`, `mode`, or `size` — those are engine-managed operator selections. Then drive the loop with `/darkrun:darkrun-resume`.
