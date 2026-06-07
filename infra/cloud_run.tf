# The darkrun-web Cloud Run service: the OAuth broker + the static site, one
# container. Runs as a dedicated least-privilege service account, scales to zero,
# pulls the three secrets from Secret Manager, and is publicly invocable (it's a
# website + an OAuth callback the browser hits).

resource "google_service_account" "web" {
  account_id   = "darkrun-web"
  display_name = "darkrun-web Cloud Run service account"
}

resource "google_cloud_run_v2_service" "web" {
  name     = "darkrun-web"
  location = var.gcp_region
  # Public site: allow direct internet ingress.
  ingress = "INGRESS_TRAFFIC_ALL"

  template {
    service_account = google_service_account.web.email

    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    containers {
      image = var.web_image

      # The server binds DARKRUN_WEB_ADDR; Cloud Run injects PORT, so bind it.
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
      env {
        name  = "GITHUB_CLIENT_ID"
        value = var.github_client_id
      }
      env {
        name  = "GITLAB_CLIENT_ID"
        value = var.gitlab_client_id
      }

      # Secrets from Secret Manager (latest version).
      dynamic "env" {
        for_each = google_secret_manager_secret.web
        content {
          name = env.key
          value_source {
            secret_key_ref {
              secret  = env.value.secret_id
              version = "latest"
            }
          }
        }
      }
    }
  }

  depends_on = [
    google_project_service.services,
    google_secret_manager_secret_iam_member.web_accessor,
  ]
}

# Public, unauthenticated invocations (a website + OAuth callbacks).
resource "google_cloud_run_v2_service_iam_member" "public" {
  name     = google_cloud_run_v2_service.web.name
  location = google_cloud_run_v2_service.web.location
  role     = "roles/run.invoker"
  member   = "allUsers"
}

# Map the custom domain (darkrun.ai) to the service. Requires the domain to be
# verified for the project (Search Console / `gcloud domains verify`). Disabled
# when var.web_domain is empty.
resource "google_cloud_run_domain_mapping" "web" {
  count    = var.web_domain != "" ? 1 : 0
  name     = var.web_domain
  location = var.gcp_region
  metadata {
    namespace = var.gcp_project
  }
  spec {
    route_name = google_cloud_run_v2_service.web.name
  }
}
