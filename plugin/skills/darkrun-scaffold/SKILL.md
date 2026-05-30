---
name: darkrun-scaffold
description: Scaffold custom darkrun artifacts — Factories, Stations, Workers, and Reviewers — as editable templates under .darkrun/
---

# Scaffold

Generate editable templates for custom darkrun artifacts. Ask the user which kind and what to name
it, then write the skeleton under `.darkrun/` and wire it into its parent.

- **Factory** — `.darkrun/factories/{name}/FACTORY.md` plus an empty `stations/` directory. The
  FACTORY.md declares the Station sequence the manager will traverse.
- **Station** — `.darkrun/factories/{factory}/stations/{name}/STATION.md` plus `workers/` and
  `reviewers/` directories. Add the new Station to the parent Factory's station list, in order.
- **Worker** — `.darkrun/factories/{factory}/stations/{station}/workers/{name}.md` with **Focus**,
  **Produces**, **Reads**, and **Anti-patterns** sections. Add it to the parent Station's worker
  list (Workers run Make → Challenge → Resolve).
- **Reviewer** — `.darkrun/factories/{factory}/stations/{station}/reviewers/{name}.md` describing
  what the Reviewer inspects and the findings it emits at the Station's audit phase.

After scaffolding, point the user at the file(s) to fill in, then suggest
`/darkrun:darkrun-factories` to confirm the new artifact shows up in the registry.
