# The Docker repository for the darkrun-web image. It's a BOOTSTRAP resource
# (created out-of-band by gcloud / bootstrap.sh) because both Cloud Build (pushes
# images) and Cloud Run (pulls them) need it to exist before either runs — so it
# must not be created inside the same apply that consumes it. Terraform only
# references it.
data "google_artifact_registry_repository" "this" {
  location      = var.region
  repository_id = var.repository_id
}
