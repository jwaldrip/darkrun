---
name: libdev
description: The library factory — ships a reusable library through the same six risk-eliminating stations, minus the UI work software needs.
inherits: software
surfaces: [library, api]
---

# Library Factory

`libdev` is the software factory, specialized for code that ships to *other
developers* rather than to end users. It changes exactly one thing and inherits
everything else.

Because a library has no UI, **Shape drops the visual-design beat**: there is no
screen to mock, no operator visual decision to make. The structure that matters
is the **public API** — the surface other code compiles against and cannot easily
change once published. Every other station, every role, every run-level reviewer
and reflection resolves through the `software` parent unchanged.

This is the whole point of `inherits`: a domain specialization is the *delta*,
not a copy. `libdev` is one station override and a parent pointer — the parent is
walkable in the resolution path, so the six-station spine, the rosters, the
fix-workers, the run reviewers, and the reflections all come from `software`.
