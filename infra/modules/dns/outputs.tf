output "name_servers" {
  description = "Set these as your registrar's nameservers to delegate the domain to this zone."
  value       = var.enable ? google_dns_managed_zone.primary[0].name_servers : []
}

output "zone_name" {
  description = "The Cloud DNS managed-zone resource name."
  value       = var.enable ? google_dns_managed_zone.primary[0].name : ""
}
