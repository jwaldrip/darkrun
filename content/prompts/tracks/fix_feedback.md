{% include "_shared/announcement.md" %}

# Fix Feedback — `{{ feedback_id }}`

Open feedback preempts forward run progress. Something a reviewer or operator flagged is unresolved, and it routes to a **fix-worker** before the line moves on.

{% include "_shared/contracts.md" %}

## What to do

1. **Read the feedback item** `{{ feedback_id }}` in full{% if station %} (station `{{ station }}`){% endif %}. Understand the actual complaint, not your guess at it.
2. **Reproduce or locate** the problem in the real artifact. Don't fix what you can't first see.
3. **Make the smallest correct change** that resolves it. Don't rewrite unrelated work to scratch an itch.
4. **Re-verify** against the feedback's criteria — the fix isn't done until the original concern is demonstrably gone.
5. **Close the loop**: record what you changed and why on `{{ feedback_id }}`, and resolve it.

## Done when

`{{ feedback_id }}` is resolved with evidence, the artifact is corrected, and nothing else regressed. Then call `run_next` — if more feedback is open, the manager routes the next item; otherwise the run resumes.
