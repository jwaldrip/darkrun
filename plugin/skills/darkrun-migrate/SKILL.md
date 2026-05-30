---
name: darkrun-migrate
description: Migrate legacy lifecycle state into darkrun's .darkrun/ StateStore — always dry-run first, get explicit approval, then apply one Run at a time
---

# Migrate

Convert legacy lifecycle state into darkrun's `.darkrun/` StateStore format.

**Never apply a migration without showing a dry-run first.** Even with `--all` or a named slug:
dry-run, get the user's OK, then apply.

## Steps

1. **List candidates.** Inspect the legacy state directory. If the user named a slug, use it.
   Otherwise show the list and ask which one(s).
2. **Dry-run** with `darkrun migrate <slug>` (dry-run is the default). Show the output — what would
   be written, where, and how many files.
3. **Get explicit approval.** Don't infer consent from prior context.
4. **Apply** with `darkrun migrate <slug> --apply`. One slug at a time unless the user explicitly
   approved `--all`.
5. After migration, suggest `/darkrun:darkrun-pickup <slug>` to resume the Run.

## Flags

- `--apply` — actually write. Default is dry-run.
- `--all` — migrate every legacy Run. Pair with `--apply` to commit.
- `--force` — re-migrate Runs that already exist under `.darkrun/`. Use sparingly.
- `--allow-dirty` — skip the git-clean precheck. Don't pass it without user approval; a dirty tree
  tangles migration output with unrelated in-progress work.

## Why these rules exist

A bare `darkrun migrate --apply` would rewrite every legacy Run at once — in a monorepo that one
commit lands in every open PR. The dry-run-then-confirm flow exists so it can't happen by accident.
