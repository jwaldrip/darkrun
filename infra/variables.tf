# ── GCP ──────────────────────────────────────────────────────────────────
variable "gcp_project" {
  description = "GCP project id that hosts darkrun-web."
  type        = string
  default     = "darkrun"
}

variable "gcp_region" {
  description = "Region for Cloud Run + Artifact Registry."
  type        = string
  default     = "us-central1"
}

variable "web_image" {
  description = "Fully-qualified container image for darkrun-web (Artifact Registry). The deploy pipeline overrides this with the freshly-pushed tag."
  type        = string
  default     = "us-central1-docker.pkg.dev/darkrun/darkrun/darkrun-web:latest"
}

variable "web_base" {
  description = "Public base URL the OAuth callbacks are registered against."
  type        = string
  default     = "https://darkrun.ai"
}

variable "web_domain" {
  description = "Custom domain to map the Cloud Run service to. Empty disables the domain mapping (use the run.app URL)."
  type        = string
  default     = "darkrun.ai"
}

variable "min_instances" {
  description = "Cloud Run minimum instances. 0 = scale to zero (lowest cost + smallest live window)."
  type        = number
  default     = 0
}

variable "max_instances" {
  description = "Cloud Run maximum instances."
  type        = number
  default     = 4
}

# ── OAuth app credentials (you create the apps; these wire them in) ───────
# IDs are non-secret env; SECRETS go into Secret Manager. Provide them in a
# (gitignored) terraform.tfvars or via TF_VAR_* env in CI.
variable "github_client_id" {
  description = "GitHub OAuth app client id."
  type        = string
  default     = ""
}

variable "github_client_secret" {
  description = "GitHub OAuth app client secret."
  type        = string
  default     = ""
  sensitive   = true
}

variable "gitlab_client_id" {
  description = "GitLab application id."
  type        = string
  default     = ""
}

variable "gitlab_client_secret" {
  description = "GitLab application secret."
  type        = string
  default     = ""
  sensitive   = true
}

# ── Sentry ───────────────────────────────────────────────────────────────
variable "sentry_auth_token" {
  description = "Sentry internal-integration / user auth token. Or set SENTRY_AUTH_TOKEN in the environment."
  type        = string
  default     = ""
  sensitive   = true
}

variable "sentry_organization" {
  description = "Sentry organization slug."
  type        = string
}

variable "sentry_team" {
  description = "Sentry team slug the projects are created under."
  type        = string
}

variable "enable_sentry" {
  description = "Provision the Sentry projects. Set false to deploy Cloud Run before Sentry is set up."
  type        = bool
  default     = true
}
