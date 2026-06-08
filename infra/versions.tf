# darkrun infrastructure — provider pins + the HCP Terraform (Terraform Cloud)
# backend. All resources live in the single GCP project `darkrun`; the modules/
# split is for organization only.

terraform {
  required_version = ">= 1.6.0"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 7.35"
    }
    sentry = {
      source  = "jianyuan/sentry"
      version = "~> 0.14"
    }
    github = {
      source  = "integrations/github"
      version = "~> 6.0"
    }
  }

  # State + runs in HCP Terraform (Terraform Cloud). The org/workspace can also
  # be supplied via the TF_CLOUD_ORGANIZATION / TF_WORKSPACE environment vars
  # (which override these). Auth: `terraform login` locally, or a
  # TF_TOKEN_app_terraform_io token in CI. Confirm the org slug matches your TFC
  # organization before the first `terraform init` (init migrates state here).
  cloud {
    organization = "darkrun-ai"
    workspaces {
      name = "darkrun-infra"
    }
  }
}
