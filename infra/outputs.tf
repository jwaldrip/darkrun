output "web_url" {
  description = "The Cloud Run service URL (the run.app URL; the custom domain serves the same)."
  value       = module.web.web_url
}

output "web_service_account" {
  description = "The darkrun-web Cloud Run service account email."
  value       = module.web.service_account
}

output "artifact_registry" {
  description = "The Docker repo to push the darkrun-web image to."
  value       = module.registry.registry_path
}

output "dns_name_servers" {
  description = "Set these as your registrar's nameservers to delegate the domain to Cloud DNS."
  value       = module.dns.name_servers
}

output "dns_zone" {
  description = "The Cloud DNS managed-zone resource name."
  value       = module.dns.zone_name
}

# Per-surface Sentry DSNs. The web DSN is already wired into Cloud Run; the cli +
# desktop DSNs feed the release pipeline (set them as GitHub Actions secrets so the
# build bakes them into the binaries); the site DSN feeds the browser SDK.
output "sentry_dsns" {
  description = "Per-surface Sentry DSNs (web / cli / desktop / site)."
  value       = module.sentry.dsns
  sensitive   = true
}
