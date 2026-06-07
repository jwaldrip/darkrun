# The darkrun-web Cloud Run service: the OAuth broker + the static site in one
# container. Runs as a dedicated least-privilege service account, scales to zero,
# mounts all config + secrets from Secret Manager, and is publicly invocable
# (a website + an OAuth callback the browser hits).
#
# Secrets model: the OAuth client id/secret pairs live ENTIRELY in Secret Manager
# (operator-managed; bootstrap.sh / `gcloud secrets versions add`). Terraform only
# references them by name — no Terraform variable carries a value, so nothing
# sensitive lands in tfvars or state. The web Sentry DSN is the one exception: a
# public ingest key derived from the sentry module, written here directly.

resource "google_service_account" "web" {
  account_id   = "darkrun-web"
  display_name = "darkrun-web Cloud Run service account"
}

# Reference the operator-managed OAuth secrets (must already exist). A missing one
# fails the plan loudly — the right signal that bootstrap wasn't run.
data "google_secret_manager_secret" "external" {
  for_each  = toset(var.external_secret_ids)
  secret_id = each.value
}

# The web Sentry DSN: created + versioned by Terraform (public key, derived from
# the sentry module). Only when Sentry is enabled.
resource "google_secret_manager_secret" "sentry_dsn" {
  count     = var.enable_sentry ? 1 : 0
  secret_id = "DARKRUN_SENTRY_DSN"
  replication {
    auto {}
  }
}

# Gated on the static enable_sentry only — the DSN value isn't known until apply
# (it's read from the freshly-created Sentry project), so it can't drive `count`.
# When Sentry is enabled the DSN is always populated, so this is safe.
resource "google_secret_manager_secret_version" "sentry_dsn" {
  count       = var.enable_sentry ? 1 : 0
  secret      = google_secret_manager_secret.sentry_dsn[0].id
  secret_data = var.sentry_dsn
}

locals {
  # env var name => Secret Manager secret_id mounted into Cloud Run.
  secret_env = merge(
    { for s in var.external_secret_ids : s => s },
    var.enable_sentry ? { DARKRUN_SENTRY_DSN = "DARKRUN_SENTRY_DSN" } : {},
  )
}

# Grant the service account accessor on every secret it consumes.
resource "google_secret_manager_secret_iam_member" "accessor" {
  for_each = merge(
    { for s in var.external_secret_ids : s => data.google_secret_manager_secret.external[s].id },
    var.enable_sentry ? { DARKRUN_SENTRY_DSN = google_secret_manager_secret.sentry_dsn[0].id } : {},
  )
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.web.email}"
}

resource "google_cloud_run_v2_service" "web" {
  name     = "darkrun-web"
  location = var.region
  ingress  = "INGRESS_TRAFFIC_ALL"

  template {
    service_account = google_service_account.web.email

    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    containers {
      image = var.web_image

      ports {
        container_port = 8080
      }

      # Non-secret config.
      env {
        name  = "DARKRUN_WEB_ADDR"
        value = "0.0.0.0:8080"
      }
      env {
        name  = "DARKRUN_WEB_BASE"
        value = var.web_base
      }
      env {
        name  = "DARKRUN_ENV"
        value = "production"
      }

      # Everything from Secret Manager (latest version): the OAuth id/secret pairs
      # and (when enabled) the web Sentry DSN.
      dynamic "env" {
        for_each = local.secret_env
        content {
          name = env.key
          value_source {
            secret_key_ref {
              secret  = env.value
              version = "latest"
            }
          }
        }
      }
    }
  }

  depends_on = [google_secret_manager_secret_iam_member.accessor]
}

# Public, unauthenticated invocations (a website + OAuth callbacks).
resource "google_cloud_run_v2_service_iam_member" "public" {
  name     = google_cloud_run_v2_service.web.name
  location = google_cloud_run_v2_service.web.location
  role     = "roles/run.invoker"
  member   = "allUsers"
}

# Map the custom domain. Requires the domain verified for the project. Disabled
# when var.web_domain is empty (use the run.app URL).
resource "google_cloud_run_domain_mapping" "web" {
  count    = var.web_domain != "" ? 1 : 0
  name     = var.web_domain
  location = var.region
  metadata {
    namespace = var.project
  }
  spec {
    route_name = google_cloud_run_v2_service.web.name
  }
}

# www subdomain mapping (paired with the www CNAME in the dns module). A
# subdomain under the verified apex needs no separate verification.
resource "google_cloud_run_domain_mapping" "www" {
  count    = var.web_domain != "" && var.manage_www ? 1 : 0
  name     = "www.${var.web_domain}"
  location = var.region
  metadata {
    namespace = var.project
  }
  spec {
    route_name = google_cloud_run_v2_service.web.name
  }
}
