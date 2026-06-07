output "repository_id" {
  description = "The Artifact Registry repository id."
  value       = data.google_artifact_registry_repository.this.repository_id
}

output "registry_path" {
  description = "The fully-qualified registry path images are pushed to."
  value       = "${var.region}-docker.pkg.dev/${var.project}/${data.google_artifact_registry_repository.this.repository_id}"
}
