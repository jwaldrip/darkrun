---
name: darkrun-changelog
description: Show the darkrun changelog / release notes, optionally for a specific version
---

# Changelog

Surface darkrun's release notes.

Call `darkrun_changelog` with an optional `version` argument:

- No `version` — show the most recent release notes (and recent history).
- `version: "x.y.z"` — show the notes for that specific release.

Present the result to the user as-is, leading with the newest changes. If a pending update is
reported, mention it.
