# The authoritative Cloud DNS zone for the domain + the records that point the
# apex and www at the Cloud Run service. After apply, set the registrar's
# nameservers to this zone's name_servers (the module output).
#
# DNS is decoupled from the Cloud Run domain mapping on purpose: the zone +
# records can exist before the domain is verified, so you can provision DNS first.
# The mapping (in the web module) handles routing/TLS once the domain is verified.

resource "google_dns_managed_zone" "primary" {
  count       = var.enable ? 1 : 0
  name        = var.zone_name
  dns_name    = "${var.domain}."
  description = "darkrun authoritative zone for ${var.domain}"
}

# Apex -> Cloud Run (A + AAAA; the apex cannot be a CNAME).
resource "google_dns_record_set" "apex_a" {
  count        = var.enable ? 1 : 0
  managed_zone = google_dns_managed_zone.primary[0].name
  name         = "${var.domain}."
  type         = "A"
  ttl          = var.ttl
  rrdatas      = var.cloud_run_a_records
}

resource "google_dns_record_set" "apex_aaaa" {
  count        = var.enable ? 1 : 0
  managed_zone = google_dns_managed_zone.primary[0].name
  name         = "${var.domain}."
  type         = "AAAA"
  ttl          = var.ttl
  rrdatas      = var.cloud_run_aaaa_records
}

# www -> Cloud Run via CNAME (Cloud Run serves www once the www domain mapping
# in the web module is in place).
resource "google_dns_record_set" "www" {
  count        = var.enable && var.manage_www ? 1 : 0
  managed_zone = google_dns_managed_zone.primary[0].name
  name         = "www.${var.domain}."
  type         = "CNAME"
  ttl          = var.ttl
  rrdatas      = ["ghs.googlehosted.com."]
}
