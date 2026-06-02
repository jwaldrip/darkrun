# darkrun merge engine — the tuned conflict dynamics to match

The predecessor's branch/merge engine was finely tuned, much of it scar tissue from
specific production bugs. darkrun's per-station hierarchy must reproduce these
**nine mechanics** exactly. Each is stated as the rule + the bug it prevents +
where darkrun implements it (`crates/darkrun-git/src/merge.rs` + `change.rs` +
`position.rs`).

## 1. Engine-protected merge (the core)
**Rule:** never a plain `git merge`. Always: `merge --no-ff --no-commit` → re-assert
**every** engine-owned state file from the **target's** pre-merge `HEAD` → then
`commit`. The *target* (the branch merged INTO) is canonical for engine state in
every engine merge — station→run-main, run-main→base, and the downstream syncs.
**Re-assert mechanic:** `git checkout <target-ref> -- <path>` then `git add -- <path>`
for each engine-owned path. `checkout … -- path` overwrites index **and** worktree
**regardless of whether the merge conflicted or silently auto-resolved** — this is
the key: `checkout --ours` only touches conflicted paths and misses the silent
auto-resolves that actually bite.
**Bug it prevents (BUG-2/3, "downstream-sync-clobber"):** when the *source* carries
a diverged copy of a state file the *target* didn't touch since the fork, git's
3-way merge silently resolves to the source (stale) side with **no conflict
marker**, reverting authoritative frontmatter — a closed fix silently reopens, a
rich unit reverts to a skeleton that re-fires the migrator every tick.
**darkrun:** `engine_protected_merge` + `restore_engine_state_from_base`; engine-owned =
`ENGINE_STATE_PREFIX` (`.darkrun/<slug>/…`) via `is_engine_owned_state_path`.

## 2. Engine-state vs user-code conflict split
**Rule:** after the re-assert, any path still unmerged (`git diff --name-only
--diff-filter=U`) is **genuine agent/user content** (code, artifacts). Engine state
is force-settled to the target; only real content conflicts remain.
**darkrun:** `unresolved_paths` after `restore_engine_state_from_base`.

## 3. User-code conflict handling + the mid-merge guard suspension
**Rule:** a real conflict does **not** abort silently — the merge is **left in-tree**
(`MERGE_HEAD` set, conflict markers present) and surfaced as a structured
`merge_conflict` action listing `conflict_paths`. The agent/human resolves the
markers; the engine resumes on the next tick.
**Critical:** while a merge is in progress (`MERGE_HEAD`/`REBASE_HEAD`/`CHERRY_PICK_HEAD`/
`REVERT_HEAD`/`rebase-merge`/`rebase-apply` markers exist), the engine **suspends its
lifecycle / ownership / branch-enforcement write guards** so the agent *can* write
the conflicted engine files to resolve them — schema validation stays on. Without
this, the guards refuse the very writes needed to resolve the conflict.
**darkrun:** add a `merge_conflict` RunAction (paths + which branch) + an
`is_merge_in_progress` check that the write-guard path honors (suspend ownership/
lifecycle guards mid-merge, keep schema validation).

## 4. Merge-debt / no-op short-circuit (loop guard)
**Rule:** before merging, skip if there's no debt: **trees identical**
(`<refA>^{tree}` == `<refB>^{tree}`) OR source is already an **ancestor** of target.
**Bug it prevents:** a `--no-ff` no-op merge still mints an empty commit; that makes
the *other* side look "behind," triggering the opposite-direction sync next tick →
an infinite alternating merge loop. Gate **both** the cursor's merge synthesis AND
the in-handler.
**darkrun:** `refs_have_identical_trees` + `is_ancestor` → a `has_no_merge_debt`
predicate, gated in `position.rs` (cursor) and the merge handler.

## 5. Downstream sync before merging up
**Rule:** before merging a station **up**, keep it **current** by merging **down**
first, two steps, each engine-protected and each debt-gated:
(1) base/mainline → run-main, (2) run-main → the active station branch. Keeping
branches fresh minimizes conflicts at land time. Report which step conflicted
(`mainline_to_run_main` | `run_main_to_station`) + the conflict branch so recovery
points at the right place.
**darkrun:** a `sync_branch_downstream(slug, station)` run each tick before a land.

## 6. Temp-worktree merge isolation
**Rule:** if the agent is **not** on the target branch, do the merge in a **detached
temp worktree** checked out on the target, so the agent's working tree is never
disturbed. If already on the target, merge in-place (a temp worktree on a
branch already checked out elsewhere fails "branch already used").
**darkrun:** `with_worktree_on_branch(target, |path| merge…)` using the existing
worktree backend; pick in-place vs temp by `current_branch()`.

## 7. Non-fast-forward push recovery
**Rule:** on a rejected push, **narrowly** match genuine NFF errors
(`non-fast-forward` / `fetch first` / `behind the remote`) — NOT a bare "rejected"
(that also matches protected-branch / pre-receive-hook / permission failures, where
rebasing is wrong). On NFF: `fetch origin <branch>` → `rebase origin/<branch>` →
retry push; rebase failure → `rebase --abort` + report.
**darkrun:** in the push path (discrete-mode PR push + any remote push).

## 8. "Complete but never merged" recovery
**Rule:** if a station's worktree is gone but its branch still has **unmerged
commits**, do NOT short-circuit to "done" — that ships verified work as a no-op
(real bug: blocker-level controls shipped empty). Merge the durable **branch**
directly. Only short-circuit when there's genuinely nothing to merge: no branch, or
it's already an ancestor of the target.
**darkrun:** the station-merge path checks `branch_exists` + `is_ancestor` before
treating a missing worktree as a no-op.

## 9. Base-branch resolution
**Rule:** `origin/HEAD` symbolic ref → local/remote `main`/`master` →
`git config init.defaultBranch` → `"main"`. (darkrun also honors
`.darkrun/settings.yml` `default_branch` first.)
**darkrun:** already partly present (authorship base resolution) — unify here.

---
**Net:** engine state is always force-held to the target; only true content
conflicts surface, and they surface *loudly* and *resolvably* (guards suspended
mid-merge); no-op merges are short-circuited so the cursor can't loop; branches are
kept fresh to minimize conflicts; merges never touch the agent's tree; pushes
recover from NFF; and a missing worktree never silently drops landed work.
