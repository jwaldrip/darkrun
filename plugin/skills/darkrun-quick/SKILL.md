---
name: darkrun-quick
description: Quick single-Station Run — create a Run the manager auto-sizes to one Station, then advance it through that Station's phases
---

A quick task is an ordinary Run right-sized down to the build + prove stations with auto gates. Create it with `darkrun_run_start { slug, title, factory, mode: "quick" }`, then drive it with `darkrun_run_next` until it seals — do exactly what each returned action says.

If the work clearly needs multiple Stations, use `/darkrun:darkrun-start` instead; if it's trivial and you want zero state, use `/darkrun:darkrun-zap`.
