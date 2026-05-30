---
name: darkrun-start
description: Start a new darkrun Run — describe what you want to accomplish and the factory manager scaffolds a right-sized lifecycle for it
---

Capture the intent cleanly, then right-size the lifecycle to the work. If the request is vague, prelaborate it into a crisp description first (ask via `AskUserQuestion`). Call `darkrun_factory_list` if the factory isn't obvious, then `darkrun_run_start { slug, title, factory, mode }`. Pick `mode` from the work: `full` (default — the whole Frame→Harden line), `quick` (build + prove, for small self-contained work), `bugfix` (specify + build + prove), or `refactor` (shape + build + prove). Drive the loop with `/darkrun:darkrun-pickup`.
