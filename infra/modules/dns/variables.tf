variable "enable" {
  description = "Create the managed zone + records. False yields no DNS resources."
  type        = bool
  default     = true
}

variable "domain" {
  description = "The apex domain to manage (e.g. darkrun.ai), no trailing dot."
  type        = string
}

variable "zone_name" {
  description = "The Cloud DNS managed-zone resource name ([a-z0-9-])."
  type        = string
  default     = "darkrun-ai"
}

variable "manage_www" {
  description = "Also create a www CNAME -> Cloud Run (needs the www domain mapping to serve)."
  type        = bool
  default     = true
}

variable "ttl" {
  description = "TTL (seconds) for the records."
  type        = number
  default     = 3600
}

# Google's anycast front-end IPs for Cloud Run / GCLB custom-domain mappings.
# The apex can't be a CNAME, so it points at these A/AAAA records; Cloud Run's
# domain mapping then routes the verified domain to the service.
variable "cloud_run_a_records" {
  description = "Apex A records (Google domain-mapping anycast IPs)."
  type        = list(string)
  default     = ["216.239.32.21", "216.239.34.21", "216.239.36.21", "216.239.38.21"]
}

variable "cloud_run_aaaa_records" {
  description = "Apex AAAA records (Google domain-mapping anycast IPs)."
  type        = list(string)
  default = [
    "2001:4860:4802:32::15",
    "2001:4860:4802:34::15",
    "2001:4860:4802:36::15",
    "2001:4860:4802:38::15",
  ]
}
