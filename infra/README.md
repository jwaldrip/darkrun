# darkrun infrastructure

Terraform for the `darkrun` GCP project + Sentry. State and runs live in **HCP
Terraform (Terraform Cloud)**. Everything targets the **single** GCP project
`darkrun`; the `modules/` split is organization only.

## Layout

```
infra/
  versions.tf        # provider pins + the cloud {} (Terraform Cloud) backend
  providers.tf       # google + sentry provider config (ambient auth)
  variables.tf       # root inputs — NO secrets (by design)
  main.tf            # API enablement + the three module calls
  outputs.tf         # web URL, SA, registry path, per-surface DSNs
  terraform.tfvars.example
  bootstrap.sh       # one-time: enable APIs + create the OAuth secrets
  modules/
    sentry/          # one Sentry project per surface (web/cli/desktop/site) + DSNs
    registry/        # Artifact Registry Docker repo
    web/             # SA + secret refs/IAM + Cloud Run service + domain mapping
```

### Module graph

```
main.tf
├── google_project_service ×4        (run, artifactregistry, secretmanager, iam)
├── module.sentry                    → dsns { web, cli, desktop, site }
├── module.registry                  → registry_path
└── module.web   (sentry_dsn ← module.sentry.dsns["web"])
        ├── google_service_account "web"
        ├── data google_secret_manager_secret  (the 4 OAuth secrets, referenced)
        ├── google_secret_manager_secret "DARKRUN_SENTRY_DSN" (+ version)
        ├── google_secret_manager_secret_iam_member  (accessor for the SA)
        ├── google_cloud_run_v2_service "darkrun-web"
        ├── google_cloud_run_v2_service_iam_member  (allUsers invoker)
        └── google_cloud_run_domain_mapping  (darkrun.ai)
```

## The secrets model — nothing sensitive touches Terraform

- **OAuth client id + secret pairs** (`GITHUB_*`, `GITLAB_*`) live **only** in
  Secret Manager, created by `bootstrap.sh`. Terraform *references* them by name
  (a `data` source) and mounts them into Cloud Run. There is **no Terraform
  variable** that can carry a value, so none can land in tfvars or state.
- **Sentry auth token** — the provider reads `SENTRY_AUTH_TOKEN` from the
  environment (a TFC workspace variable). Never a Terraform variable.
- **Web Sentry DSN** — a *public* ingest key, derived from the sentry module and
  written into Secret Manager by Terraform. A public DSN in state is not a leak.

## First-time setup

> **Identity check.** This mutates the `darkrun` GCP project. Confirm the active
> gcloud account is darkrun-scoped (`gcloud config get-value account`) before you
> start. `bootstrap.sh` guards on project access.

```bash
# 1. Enable APIs + create the OAuth secrets in Secret Manager (once).
#    Values come from env vars of the same name, or interactive hidden prompts.
GITHUB_CLIENT_ID=... GITHUB_CLIENT_SECRET=... \
GITLAB_CLIENT_ID=... GITLAB_CLIENT_SECRET=... \
  ./infra/bootstrap.sh

# 2. Authenticate to HCP Terraform.
terraform login

# 3. Non-secret knobs + the Sentry token.
cd infra
cp terraform.tfvars.example terraform.tfvars      # edit org/team/domain
export SENTRY_AUTH_TOKEN=...                       # or a TFC workspace variable

# 4. Plan + apply (runs in your TFC workspace).
terraform init
terraform apply
```

### HCP Terraform workspace

The `cloud {}` block in `versions.tf` points at organization `darkrun-ai`,
workspace `darkrun-infra` (override via `TF_CLOUD_ORGANIZATION` / `TF_WORKSPACE`).
Runs execute **remotely** on TFC, so set these as **workspace variables** in the
`darkrun-infra` workspace:

- Environment: `SENTRY_AUTH_TOKEN` (sensitive), plus GCP credentials — use GCP
  **dynamic provider credentials** (recommended; no static key — TFC's workload
  identity federates into GCP), or `GOOGLE_CREDENTIALS` (a SA key, sensitive).
- Terraform: `sentry_organization`, `sentry_team` (or keep them in tfvars).

The first `terraform init` creates/binds the workspace and migrates state into it.

> Not ready for Sentry? `enable_sentry = false` ships Cloud Run first; the DSN
> secret is skipped and the server no-ops without a DSN.

## Wiring the DSNs into the binaries

The CLI + desktop binaries bake their DSN at **compile time** (`option_env!`).
After `apply`:

```bash
terraform output -json sentry_dsns   # { web, cli, desktop, site }
```

- `cli` DSN  → GitHub Actions secret `DARKRUN_CLI_SENTRY_DSN`
- `desktop` DSN → `DARKRUN_DESKTOP_SENTRY_DSN`

`release.yml` reads both and compiles them in. The `web` DSN needs no manual step
(Secret Manager → Cloud Run). The `site` DSN feeds the browser SDK.

## Deploying the web service from CI

`.github/workflows/deploy-web.yml` builds + pushes the image (GCP via Workload
Identity), then runs `terraform apply` against the TFC workspace with the new
image tag. Requires repo secrets `GCP_WORKLOAD_IDENTITY_PROVIDER`, `GCP_DEPLOY_SA`,
and `TF_API_TOKEN`; the TFC run's own GCP + Sentry credentials are workspace
variables.

## Custom domain

`web_domain` (default `darkrun.ai`) creates a Cloud Run domain mapping. Verify the
domain for the project first (`gcloud domains verify`), then point DNS at the
records Cloud Run reports. Set `web_domain = ""` to use the `run.app` URL.

## Notes

- **C-free** end-to-end: the server image is rustls (no openssl); the only runtime
  apt package is `ca-certificates`.
- The container serves the site from `/srv/site` (`DARKRUN_SITE_DIR`).
- Never commit `terraform.tfvars` or `.terraform/` — the root `.gitignore` covers
  them. State is in TFC, not on disk.
