# Push the per-surface Sentry DSNs into the repo's GitHub Actions secrets, so the
# release workflow bakes them into the distributed binaries at compile time
# (darkrun-telemetry reads option_env!("DARKRUN_SENTRY_DSN")). This closes the
# loop: `terraform apply` provisions the Sentry projects AND wires their DSNs into
# the build — no manual copy.
#
# A DSN is a write-only ingest key that ships inside a public binary anyway, so
# this isn't leaking a secret; the Actions secret just keeps it out of CI logs.
#
# count is gated on the static `enable` bool only — the DSN value is known after
# apply, so it can't drive count.

resource "github_actions_secret" "cli_dsn" {
  count       = var.enable ? 1 : 0
  repository  = var.repository
  secret_name = "DARKRUN_CLI_SENTRY_DSN"
  value       = var.cli_dsn
}

resource "github_actions_secret" "desktop_dsn" {
  count       = var.enable ? 1 : 0
  repository  = var.repository
  secret_name = "DARKRUN_DESKTOP_SENTRY_DSN"
  value       = var.desktop_dsn
}
