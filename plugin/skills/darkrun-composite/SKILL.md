---
name: darkrun-composite
description: Create a composite Run combining stations from multiple factories with sync points
---

Create a composite Run that coordinates work across **two or more factories** in parallel, with sync points where one factory's line must wait on another's.

## Process

1. Gather from the user:
   - The work description
   - The factory selection — **2+ required** (use `darkrun_factory_list` for what's available; one factory = a normal `darkrun_run_new`)
   - The station subset per factory (`darkrun_factory_detail` shows each line; empty = the full line)
   - Where the factories must synchronize

2. Suggest sync points from the artifact chains: when one factory's station consumes what another's produces, that's a `wait`/`then` edge. Handles are `factory:station`.

3. Create it:

```
darkrun_run_composite {
  slug, title,
  parts: [
    { factory: "software", stations: ["build", "prove"] },
    { factory: "legal",    stations: [] }
  ],
  sync: [
    { wait: ["software:prove"], then: ["legal:shape"] }
  ]
}
```

4. Report the created Run with the part/station overview and the sync map.

## Coordinating a composite Run

A composite Run is **not single-walkable** — `darkrun_tick` surfaces the topology instead of walking it. Drive each part as its own Run (`darkrun_run_new` per part, scoped with `size`/station subset), honor the sync points (don't start a `then` part before its `wait` handles complete), and record progress on the ledger as parts move:

```
darkrun_composite_stamp { slug, handle: "software:build", note: "completed" }
```

The ledger (`composite_state`) is the coordination record the operator reads to see where the composite stands.
