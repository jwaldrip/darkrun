# darkrun infrastructure — provider + Terraform version pins.
#
# Two providers:
#   - google: the GCP project `darkrun` (Cloud Run, Artifact Registry, Secret
#     Manager) that hosts the `darkrun-web` server (OAuth broker + static site).
#   - sentry: provisions one Sentry project per app surface and exposes their
#     DSNs as outputs (the web DSN is wired straight into Cloud Run; the CLI +
#     desktop DSNs are read by the release pipeline and compiled into the
#     distributed binaries).

terraform {
  required_version = ">= 1.6.0"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 6.0"
    }
    sentry = {
      source  = "jianyuan/sentry"
      version = "~> 0.14"
    }
  }

  # State lives in a GCS bucket in the darkrun project (created by the bootstrap
  # step — see infra/README.md). Comment this block out to use local state.
  backend "gcs" {
    bucket = "darkrun-tfstate"
    prefix = "infra"
  }
}
