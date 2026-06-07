# Secret Manager holds the values that must never live in the image or in plain
# Cloud Run env: the OAuth client secrets and the web Sentry DSN. The Cloud Run
# service account is granted accessor on each.

locals {
  # name => secret value. The Sentry DSN flows in from the provisioned project
  # (no manual copy). A secret with an empty value is created but left without a
  # version, so you can populate it later via `gcloud secrets versions add`.
  secret_values = {
    GITHUB_CLIENT_SECRET = var.github_client_secret
    GITLAB_CLIENT_SECRET = var.gitlab_client_secret
    DARKRUN_SENTRY_DSN   = try(local.sentry_dsn["web"], "")
  }
}

resource "google_secret_manager_secret" "web" {
  for_each  = local.secret_values
  secret_id = each.key
  replication {
    auto {}
  }
  depends_on = [google_project_service.services]
}

# Only create a version when a non-empty value is supplied (avoids an empty
# secret version; populate empties out-of-band with `gcloud secrets versions add`).
resource "google_secret_manager_secret_version" "web" {
  for_each    = { for k, v in local.secret_values : k => v if trimspace(v) != "" }
  secret      = google_secret_manager_secret.web[each.key].id
  secret_data = each.value
}

# Let the Cloud Run service account read each secret.
resource "google_secret_manager_secret_iam_member" "web_accessor" {
  for_each  = google_secret_manager_secret.web
  secret_id = each.value.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.web.email}"
}
