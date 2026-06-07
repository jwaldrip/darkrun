# Provider configuration. Auth is ambient:
#   - google: Application Default Credentials (`gcloud auth application-default
#     login`, or a deploy service account in CI via GOOGLE_APPLICATION_CREDENTIALS).
#   - sentry: the auth token from `var.sentry_auth_token` (or the SENTRY_AUTH_TOKEN
#     env, which the provider reads automatically).

provider "google" {
  project = var.gcp_project
  region  = var.gcp_region
}

provider "sentry" {
  # Falls back to SENTRY_AUTH_TOKEN in the environment when the var is unset.
  token = var.sentry_auth_token != "" ? var.sentry_auth_token : null
}
