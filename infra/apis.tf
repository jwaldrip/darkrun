# Enable the GCP services darkrun-web needs. Kept non-destroying so a
# `terraform destroy` doesn't disable APIs other things might share.
resource "google_project_service" "services" {
  for_each = toset([
    "run.googleapis.com",
    "artifactregistry.googleapis.com",
    "secretmanager.googleapis.com",
    "iam.googleapis.com",
  ])
  service            = each.value
  disable_on_destroy = false
}
