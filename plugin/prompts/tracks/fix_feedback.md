{% include "_shared/announcement.md" %}

# Fix Feedback — `{{ feedback_id }}`

Open feedback preempts forward run progress. Something a reviewer or operator flagged is unresolved, and it routes to a **fix-worker** before the line moves on.

{% if fix_workers %}
Dispatch one of **this station's** fix-workers — {% for w in fix_workers %}`{{ w }}`{% if not loop.last %}, {% endif %}{% endfor %} — the repairers specialized for this station's class of work.
{% endif %}

{% include "_shared/contracts.md" %}

{% if fix_worktree %}
## This fix has its own worktree — work in it

The repair is isolated on its own branch + worktree, forked off the station branch: **`{{ fix_worktree }}`**. Make the fix **inside that worktree** so its diff never tangles with in-flight unit work; the manager lands it back onto the station branch when you resolve the feedback. Don't commit the fix to the station branch yourself.
{% endif %}

## What to do

1. **Read the feedback item** `{{ feedback_id }}` in full{% if station %} (station `{{ station }}`){% endif %}. Understand the actual complaint, not your guess at it.
2. **Reproduce or locate** the problem in the real artifact. Don't fix what you can't first see.
3. **Make the smallest correct change** that resolves it. Don't rewrite unrelated work to scratch an itch.
4. **Re-verify** against the feedback's criteria — the fix isn't done until the original concern is demonstrably gone.
5. **Close the loop**: record what you changed and why on `{{ feedback_id }}`, and resolve it.

## Done when

`{{ feedback_id }}` is resolved with evidence, the artifact is corrected, and nothing else regressed. Then call `darkrun_tick` — if more feedback is open, the manager routes the next item; otherwise the run resumes.
