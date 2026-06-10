---
name: darkrun-setup
description: Configure darkrun for this project — auto-detect VCS, hosting, CI/CD, and default branch, confirm with the user, and write .darkrun/settings.yml
---

Call `darkrun_setup` to auto-detect the project environment (VCS, hosting, CI/CD, default branch) and available MCP providers, present what it found to the user via `AskUserQuestion`, adjust, then write `.darkrun/settings.yml`. It's additive and idempotent — only the confirmed changes are written. Then suggest `/darkrun:darkrun-new`.

## External-system providers

Beyond the environment fields, `.darkrun/settings.yml` can declare **providers** — external systems whose behavior contracts get spliced into the engine's prompts when configured:

- `providers.ticketing` — issue trackers (jira, linear, github-issues, gitlab-issues): tickets per unit, status sync, epic linkage.
- `providers.spec` — PRD/RFC sources (notion, confluence, google-docs): the Run aligns to the external spec.
- `providers.knowledge` — org memory (notion, confluence, google-docs): patterns/decisions inform elaboration.
- `providers.design` — design tools (figma, pencil, openpencil, penpot, excalidraw, canva): tokens + component inventory drive UI units.

If the user's MCP tool surface shows one of these systems connected, OFFER the matching provider block. Config shapes are schema-checked (`plugin/schemas/settings.schema.json` + `plugin/schemas/providers/*.schema.json`); `darkrun_setup` reports any `settings_problems` in the existing file. Example:

```yaml
providers:
  ticketing:
    type: linear
    config:
      project_key: DR
```

The git provider is always-on in a git repo — no settings entry needed.

