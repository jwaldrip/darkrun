# Root composition: project-level API enablement + the three modules. Everything
# targets the single GCP project `darkrun` (var.gcp_project).

locals {
  # The OAuth client id/secret pairs, by Secret Manager secret_id. These exist in
  # Secret Manager only (operator-managed via bootstrap.sh); Terraform references
  # them, never their values. The web service mounts each as an env var of the
  # same name.
  oauth_secret_ids = [
    "GITHUB_CLIENT_ID",
    "GITHUB_CLIENT_SECRET",
    "GITLAB_CLIENT_ID",
    "GITLAB_CLIENT_SECRET",
  ]
}

# Enable the GCP services the stack needs. Non-destroying so a `terraform destroy`
# doesn't disable APIs other things might share.
resource "google_project_service" "services" {
  for_each = toset([
    "run.googleapis.com",
    "artifactregistry.googleapis.com",
    "secretmanager.googleapis.com",
    "iam.googleapis.com",
    "dns.googleapis.com",
  ])
  service            = each.value
  disable_on_destroy = false
}

module "sentry" {
  source = "./modules/sentry"

  enable       = var.enable_sentry
  organization = var.sentry_organization
  team         = var.sentry_team
}

# The registry is a bootstrap resource (created by gcloud/bootstrap.sh); this
# module only references it, so no depends_on on API enablement is needed.
module "registry" {
  source = "./modules/registry"

  project = var.gcp_project
  region  = var.gcp_region
}

module "web" {
  source = "./modules/web"

  project             = var.gcp_project
  region              = var.gcp_region
  web_image           = var.web_image
  web_base            = var.web_base
  web_domain          = var.web_domain
  min_instances       = var.min_instances
  max_instances       = var.max_instances
  enable_sentry       = var.enable_sentry
  sentry_dsn          = try(module.sentry.dsns["web"], "")
  external_secret_ids = local.oauth_secret_ids
  manage_www          = var.manage_www

  depends_on = [google_project_service.services]
}

# The authoritative Cloud DNS zone for the domain + apex/www records pointing at
# Cloud Run. Decoupled from the domain mapping so you can provision DNS first;
# the module output exposes the nameservers to set at your registrar.
module "dns" {
  source = "./modules/dns"

  enable     = var.manage_dns && var.web_domain != ""
  domain     = var.web_domain
  zone_name  = var.dns_zone_name
  manage_www = var.manage_www

  depends_on = [google_project_service.services]
}

# Push the cli/desktop Sentry DSNs into the repo's GitHub Actions secrets so the
# release workflow bakes them into the binaries. Gated on Sentry being on AND the
# toggle (which needs a GITHUB_TOKEN). The DSNs are known after apply.
module "release_secrets" {
  source = "./modules/release-secrets"

  enable      = var.enable_sentry && var.manage_release_secrets
  repository  = var.github_repository
  cli_dsn     = try(module.sentry.dsns["cli"], "")
  desktop_dsn = try(module.sentry.dsns["desktop"], "")
}
