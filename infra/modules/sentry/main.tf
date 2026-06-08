# One Sentry project per app surface, so errors triage per-surface and each gets
# its own DSN. The web DSN is wired into Cloud Run (the web module); the cli +
# desktop DSNs feed the release pipeline (compiled into the binaries); the site
# DSN feeds the browser SDK. All gated on var.enable.

locals {
  surfaces = var.enable ? {
    web     = { name = "darkrun-web", platform = "rust" }
    cli     = { name = "darkrun-cli", platform = "rust" }
    desktop = { name = "darkrun-desktop", platform = "rust" }
    site    = { name = "darkrun-site", platform = "javascript" }
  } : {}
}

resource "sentry_project" "surface" {
  for_each = local.surfaces

  organization = var.organization
  teams        = [var.team]
  name         = each.value.name
  slug         = each.value.name
  platform     = each.value.platform
}

data "sentry_all_keys" "surface" {
  for_each = sentry_project.surface

  organization = var.organization
  project      = each.value.slug
}

locals {
  # public DSN per surface (empty map when disabled).
  dsns = {
    for k, keys in data.sentry_all_keys.surface :
    k => try(keys.keys[0].dsn["public"], "")
  }
}
