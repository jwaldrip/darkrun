---
name: darkrun-show
description: Display a darkrun Run's state — Stations, Units, completion criteria, and Checkpoint status
---

Call `darkrun_run_show` (optional slug/id). It raises the Run in the **darkrun desktop app** — launching the app if one isn't already connected — and returns the structured state. Summarize the state lead-with-what's-next: current Station and phase → Units done/in-progress/blocked → the next Checkpoint; use `darkrun_unit_list { run, station }` for a Station's Units and completion criteria. Relay the `showing.desktop.status`: `building` means it's compiling `darkrun-desktop` for this arch and the window opens when done; `not_found` means relay the hint (set `DARKRUN_DESKTOP`, or build it). Offer `/darkrun:darkrun-pickup` to advance.
