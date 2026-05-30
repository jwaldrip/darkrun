{% include "_shared/announcement.md" %}

# Hold — nothing to dispatch

{{ message | default("The line is mid-wave. Outstanding subagents are still working; there is no new action to take this tick.") }}

Do **not** invent work to fill the gap. Let the in-flight work finish, then call `run_next` again for the next real action.
