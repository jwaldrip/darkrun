{# Non-negotiable rules every action obeys. Included near the top of each phase. #}
**Contract**

- Do exactly the work this action describes — no more, no less. Don't skip ahead to a later phase.
- Treat the locked artifact{% if locked_artifact %} (`{{ locked_artifact }}`){% endif %} as the source of truth. Read it before you act; never silently rewrite a locked decision.
- Every claim you make must be backed by something you actually ran, read, or wrote. No assumed results.
- When the action is finished, record your output where the station expects it, then call `run_next` again for the next instruction. The manager — not you — decides what comes next.
