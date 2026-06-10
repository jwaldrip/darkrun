{% include "_shared/announcement.md" %}

# Save Work in Progress — `{{ branch }}`

You have **uncommitted work** in the project tree. The manager will not advance the run with loose changes on `{{ branch }}` — commit them first, then retick. This is purely mechanical: no human intervention needed.

{% include "_shared/contracts.md" %}

## Why the engine won't commit this for you

The engine commits its own `.darkrun/` bookkeeping on every tick — but it never authors **your** commits. You know what you just did; a generic engine "wip" dump can't tell the story of the work. Commit messages are part of the record the run leaves behind.

## Uncommitted paths

{% for p in dirty_files %}- `{{ p }}`
{% endfor %}

## What to do

1. **Group related changes** into separate, coherent commits — one logical step each, not a single catch-all dump.
2. **Write messages that explain the why** of each change, not just the what.
3. Commit everything listed above on `{{ branch }}` (`git add … && git commit …`).
4. Re-run `darkrun_tick`. The manager re-checks the tree and resumes the run.

If a listed file is scratch output you never meant to keep, delete it (or gitignore it) instead of committing it — the gate clears either way.

## Done when

The project tree is clean apart from the engine's own `.darkrun/` state, and `darkrun_tick` advances past this action.
