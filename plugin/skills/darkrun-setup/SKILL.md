---
name: darkrun-setup
description: Configure darkrun for this project — auto-detect VCS, hosting, CI/CD, and default branch, confirm with the user, and write .darkrun/settings.yml
---

# Setup

Configure `.darkrun/settings.yml` by auto-detecting the project environment and confirming with the
user before writing.

1. **Detect.** VCS (`git`/`jj`), hosting (from the remote URL), CI/CD (workflow files present), and
   the default branch.
2. **Discover providers.** Use ToolSearch to find available MCP providers for ticketing, spec,
   design, and comms that darkrun can wire into Stations.
3. **Confirm.** Present the detected settings to the user via `AskUserQuestion` and let them adjust.
4. **Configure providers.** For each confirmed provider, collect the config it needs.
5. **Tune the workflow.** Ask about defaults — preferred factory, decomposition granularity,
   Checkpoint posture (auto/ask/external/await), and which Reviewers run by default.
6. **Write.** Save `.darkrun/settings.yml`, preserving any existing fields. Commit it.
7. **Done.** Show a summary and suggest `/darkrun:darkrun-start` to create the first Run.

Setup is additive and idempotent — re-running it re-detects, shows what would change, and only writes
what the user confirms.
