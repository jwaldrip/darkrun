output "web_url" {
  description = "The Cloud Run service URL (the run.app URL; the custom domain serves the same)."
  value       = google_cloud_run_v2_service.web.uri
}

output "web_service_account" {
  description = "The darkrun-web Cloud Run service account email."
  value       = google_service_account.web.email
}

output "artifact_registry" {
  description = "The Docker repo to push the darkrun-web image to."
  value       = "${var.gcp_region}-docker.pkg.dev/${var.gcp_project}/${google_artifact_registry_repository.darkrun.repository_id}"
}

# Sentry DSNs. The web DSN is already wired into Cloud Run; the cli + desktop
# DSNs feed the release pipeline (set them as GitHub Actions secrets so the
# build bakes them into the distributed binaries); the site DSN feeds the
# browser SDK. Sensitive so they aren't printed in plan/apply logs.
output "sentry_dsns" {
  description = "Per-surface Sentry DSNs (web / cli / desktop / site)."
  value       = local.sentry_dsn
  sensitive   = true
}
