---
name: darkrun-backlog
description: Parking lot for ideas not yet ready to become a Run — add, list, review, or promote a backlog item into a Run
---

# Backlog

A holding place for ideas that aren't ready to become a Run. Capture them now, shape them later.

Call `darkrun_backlog` with an optional `action` and `description`, then follow the returned
instructions.

- `list` (default) — show the current backlog items.
- `add` — capture a new idea from `description`.
- `review` — walk the backlog and surface items worth promoting.
- `promote` — turn a backlog item into a Run. After it resolves, hand off to
  `/darkrun:darkrun-start` (or `/darkrun:darkrun-quick` for small items) to scaffold the lifecycle.

Use the backlog for "we should do this eventually" thoughts that would otherwise clutter an active
Run. When an item is ripe, promote it rather than re-typing it.
