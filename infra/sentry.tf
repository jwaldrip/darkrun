# One Sentry project per app surface, so errors are triaged per-surface and each
# gets its own DSN. The web DSN is wired straight into Cloud Run (secrets.tf);
# the cli + desktop DSNs are outputs the release pipeline compiles into the
# distributed binaries; the site DSN feeds the browser SDK in index.html.
#
# Gated on var.enable_sentry so Cloud Run can be deployed before Sentry exists.

locals {
  sentry_surfaces = var.enable_sentry ? {
    web     = { name = "darkrun-web", platform = "rust" }
    cli     = { name = "darkrun-cli", platform = "rust" }
    desktop = { name = "darkrun-desktop", platform = "rust" }
    site    = { name = "darkrun-site", platform = "javascript" }
  } : {}
}

resource "sentry_project" "surface" {
  for_each = local.sentry_surfaces

  organization = var.sentry_organization
  teams        = [var.sentry_team]
  name         = each.value.name
  slug         = each.value.name
  platform     = each.value.platform
}

# The default client key (DSN) for each project.
data "sentry_all_keys" "surface" {
  for_each = sentry_project.surface

  organization = var.sentry_organization
  project      = each.value.slug
}

locals {
  # public DSN per surface (empty map when Sentry is disabled).
  sentry_dsn = {
    for k, keys in data.sentry_all_keys.surface :
    k => try(keys.keys[0].dsn_public, "")
  }
}
