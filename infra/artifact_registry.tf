# Docker repository for the darkrun-web image.
resource "google_artifact_registry_repository" "darkrun" {
  location      = var.gcp_region
  repository_id = "darkrun"
  format        = "DOCKER"
  description   = "darkrun container images (darkrun-web)."

  depends_on = [google_project_service.services]
}
