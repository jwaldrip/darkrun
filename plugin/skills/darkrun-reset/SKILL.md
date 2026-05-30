---
name: darkrun-reset
description: Wipe one Station of a Run (its Units, outputs, artifacts, decomposition, feedback, and branch) so the manager re-enters it from scratch — or reset/archive the whole Run. Other Stations stay untouched.
---

# Reset

Wipe a Station and let the manager re-enter it at its spec phase on the next tick. The Run's other
Stations, their Checkpoint approvals, and the Run's main history all stay put. Reset is destructive
and confirmed before anything is removed.

## When to use

The Run is sound, but one Station produced output that no longer matches what the user wants —
usually because the Factory's Worker instructions, Reviewers, or Station config were updated *after*
the Station ran, and the user wants that Station redone with the new instructions.

**Not the right tool when:**

- The whole Run's premise was wrong from the start — reset the whole Run (scope `run`) or archive it.
- A Station's output has a few specific problems to flag — that's a feedback revisit
  (`darkrun_feedback_create`), not a reset.
- The user wants to roll back the Run's git history — that's a manual `git revert` / `git reset`,
  out of scope here.

## How to drive it

1. **Identify the Run.** If no slug is known from context, call `darkrun_run_list`. If several are
   active and the user didn't name one, ask which before proceeding.
2. **Station scope:** call `darkrun_run_reset { run: "<slug>", station: "<station>" }`. The tool
   confirms via a picker and lists exactly what will be deleted.
   **Run scope:** omit `station` to reset the whole Run, or `darkrun_run_archive { run: "<slug>" }`
   to retire it entirely.
3. After the user confirms, the tool performs the wipe.
4. The tool returns a message to call `darkrun_run_next` — the next tick re-enters the Station at its
   spec phase and the manager re-runs its Worker sequence.

## What gets wiped (Station scope)

- The Station's Units, outputs, artifacts, and decision log.
- The Station's decomposition/elaboration notes.
- The Station's feedback records.
- The Station's Explorer/discovery outputs (the Factory's template files are kept; produced outputs
  are wiped).
- The Station's git branch — the next `darkrun_run_next` forks it from the Run's main as needed.

## What stays

- The Run's identity and the Checkpoint approvals that belong to **other** Stations.
- The Run main's commits — the Station's prior merge stays in history; the new work supersedes it via
  the normal merge path.
- Every other Station's Units, outputs, approvals, and branches.
- The Station's *declaration* in the Run's station list — it still exists as a phase; it just starts
  over.
