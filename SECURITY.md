# Security Policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub:
**[Security → Report a vulnerability](https://github.com/darkrun-ai/darkrun/security/advisories/new)**.

Please don't open a public issue for anything you believe is exploitable. Include what you
found, where it lives (crate/binary/surface), and how to reproduce it — a working proof of
concept beats a long description.

Reports are acknowledged and triaged promptly; confirmed vulnerabilities are fixed with
priority ordered by severity and exploitability, and credited in the release notes unless you
ask otherwise.

## Supported versions

darkrun is pre-1.0. Security fixes land on `main` and ship in the **latest release** — there
are no maintained back-port branches. If you're on an older 0.x, the fix path is upgrading.

| Version | Supported |
|---|---|
| latest release | yes |
| anything older | no — upgrade |

## Scope

In scope:

- **The engine** — the `darkrun` binary: MCP server, the HTTP/WS server, the orchestrator,
  and its git machinery (pure-Rust gitoxide; no `git`/`gh`/`glab` shell-out).
- **The desktop app** — the review surface (Dioxus/Wry).
- **darkrun.ai** — the website and the **auth broker** (the OAuth handoff that parks
  short-lived provider credentials by one-time nonce).
- **The plugin** — prompts, skills, schemas, and factory content shipped in this repo.

Out of scope:

- Vulnerabilities in the projects darkrun operates on (report those to their owners).
- Behavior of the agent/harness driving darkrun (Claude Code etc.) — report harness issues
  upstream. Prompt-injection findings against the *engine's own guardrails* (gates, scope
  enforcement, write guards) are in scope and welcome.
- Third-party dependency advisories with no reachable path in darkrun — we track these via
  Dependabot and disposition each one with a written reason.

## How darkrun holds your data

- **Local-first state.** Run state lives in `.darkrun/` inside your repository and on your
  git remotes. There is no darkrun backend holding your work.
- **No telemetry by default.** The engine emits events to a local `events.jsonl` per run.
  Network export happens only if you configure an OTLP endpoint
  (`DARKRUN_OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT`).
- **Credentials.** Provider tokens are stored at `~/.darkrun/credentials` with `0600`
  permissions, or supplied per-call. The engine forces non-interactive git credential paths
  (`GIT_TERMINAL_PROMPT=0`, batch-mode SSH) so a missing credential fails fast instead of
  prompting — and never logs token values.
- **The auth broker** holds a credential only between OAuth callback and a one-time
  nonce claim; a claim consumes it.
- **TLS** is pure-Rust (`rustls`) across the engine's HTTP clients.

## Hardening expectations for contributions

- No new C dependencies in the git/network path without discussion.
- Anything that executes project-defined commands (quality gates, boot scripts) runs with the
  operator's own privileges by design — never widen that to remote-supplied input.
- Secrets must never appear in engine output, prompts, committed state, or test fixtures
  (deterministic dummy literals in `#[cfg(test)]` are fine; real-looking tokens are not).
