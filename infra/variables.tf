# Root inputs. Note what is NOT here: the OAuth client ids/secrets and the Sentry
# auth token. Those never pass through Terraform — the OAuth pairs live in Secret
# Manager (operator-managed), and the Sentry token is a TFC/env credential. So no
# secret can land in tfvars or state.

# ── GCP ──────────────────────────────────────────────────────────────────
variable "gcp_project" {
  description = "GCP project id that hosts the whole stack."
  type        = string
  default     = "darkrun"
}

variable "gcp_region" {
  description = "Region for Cloud Run + Artifact Registry."
  type        = string
  default     = "us-central1"
}

variable "web_image" {
  description = "Fully-qualified container image for darkrun-web. The deploy pipeline overrides this with the freshly-pushed tag."
  type        = string
  default     = "us-central1-docker.pkg.dev/darkrun/darkrun/darkrun-web:latest"
}

variable "web_base" {
  description = "Public base URL the OAuth callbacks are registered against."
  type        = string
  default     = "https://darkrun.ai"
}

variable "web_domain" {
  description = "Custom domain to map the Cloud Run service to. Empty disables the mapping (use the run.app URL)."
  type        = string
  default     = "darkrun.ai"
}

variable "min_instances" {
  description = "Cloud Run minimum instances. 0 = scale to zero."
  type        = number
  default     = 0
}

# ── DNS ──────────────────────────────────────────────────────────────────
variable "manage_dns" {
  description = "Create the Cloud DNS zone + apex/www records for web_domain. Set false to manage DNS elsewhere."
  type        = bool
  default     = true
}

variable "dns_zone_name" {
  description = "The Cloud DNS managed-zone resource name ([a-z0-9-])."
  type        = string
  default     = "darkrun-ai"
}

variable "manage_www" {
  description = "Map + point the www subdomain at the service (web mapping + dns CNAME)."
  type        = bool
  default     = true
}

variable "max_instances" {
  description = "Cloud Run maximum instances."
  type        = number
  default     = 4
}

# ── Sentry ───────────────────────────────────────────────────────────────
# The auth token is NOT a variable — the provider reads SENTRY_AUTH_TOKEN from the
# environment (a TFC workspace variable, or your shell). Only the non-secret slugs
# live here.
variable "sentry_organization" {
  description = "Sentry organization slug. Unused when enable_sentry = false."
  type        = string
  default     = ""
}

variable "sentry_team" {
  description = "Sentry team slug the projects are created under. Unused when enable_sentry = false."
  type        = string
  default     = ""
}

variable "enable_sentry" {
  description = "Provision the Sentry projects. Set false to deploy Cloud Run before Sentry is set up."
  type        = bool
  default     = true
}

# ── GitHub (release-secrets wiring) ──────────────────────────────────────
# Used only to push the cli/desktop Sentry DSNs into the repo's Actions secrets.
# The token is the GITHUB_TOKEN env (a TFC workspace variable), never a TF var.
variable "github_owner" {
  description = "GitHub org/user that owns the repo whose Actions secrets are set."
  type        = string
  default     = "darkrun-ai"
}

variable "github_repository" {
  description = "Repo name (under github_owner) whose Actions secrets receive the DSNs."
  type        = string
  default     = "darkrun"
}

variable "manage_release_secrets" {
  description = "Have Terraform push the cli/desktop Sentry DSNs into the repo's GitHub Actions secrets. Needs GITHUB_TOKEN set. Turn off to manage those secrets by hand."
  type        = bool
  default     = true
}
