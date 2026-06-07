# darkrun infrastructure

Terraform for the `darkrun` GCP project + Sentry. It provisions one thing you run
(the `darkrun-web` server) and the observability behind every surface:

- **Cloud Run** `darkrun-web` — the OAuth broker **and** the static site, one
  container, scale-to-zero, public.
- **Artifact Registry** — the Docker repo the image is pushed to.
- **Secret Manager** — the OAuth client secrets + the web Sentry DSN, mounted into
  Cloud Run as env (never baked into the image).
- **Sentry** — one project per surface (`web`, `cli`, `desktop`, `site`), each with
  its own DSN. The web DSN flows straight into Cloud Run; the cli/desktop DSNs are
  outputs you feed to the release pipeline; the site DSN feeds the browser SDK.

## Layout

| file | what |
|---|---|
| `versions.tf` | provider pins + the GCS state backend (`darkrun-tfstate`) |
| `providers.tf` | google + sentry provider config (ambient auth) |
| `variables.tf` | all inputs (project, domain, OAuth ids/secrets, Sentry) |
| `apis.tf` | enables run / artifactregistry / secretmanager / iam |
| `artifact_registry.tf` | the Docker repo |
| `sentry.tf` | per-surface Sentry projects + DSN lookup |
| `secrets.tf` | Secret Manager secrets + accessor IAM for the web SA |
| `cloud_run.tf` | the `darkrun-web` service, public invoker, domain mapping |
| `outputs.tf` | service URL, SA email, registry path, per-surface DSNs |
| `bootstrap.sh` | one-time: creates the state bucket + enables APIs |

## First-time setup

> **Identity check.** Everything here mutates the `darkrun` GCP project. Make sure
> the active gcloud account is darkrun-scoped before you start —
> `gcloud config get-value account` / `project`. A borrowed CI service account from
> another project will create the resources in the wrong place. `bootstrap.sh`
> guards on this but confirm it yourself.

```bash
# 0. Point gcloud at darkrun and authenticate for Terraform.
gcloud config set project darkrun
gcloud auth application-default login

# 1. Create the Terraform state bucket + turn on the APIs (once).
./infra/bootstrap.sh

# 2. Fill in inputs.
cd infra
cp terraform.tfvars.example terraform.tfvars   # gitignored; add your secrets

# 3. Plan + apply.
terraform init
terraform apply
```

### What you provide

- **OAuth apps** (you create these; Terraform only wires them in):
  - GitHub OAuth App — callback `https://darkrun.ai/auth/github/callback`, scope `repo`.
  - GitLab Application — callback `https://darkrun.ai/auth/gitlab/callback`, scope
    `api`, Confidential = Yes.
  - Put the ids + secrets in `terraform.tfvars`.
- **Sentry** — an org slug, a team slug, and an auth token with `project:write`
  (Settings → Developer Settings → internal integration, or a personal token).
  Set `sentry_organization`, `sentry_team`, `sentry_auth_token`.
  - Not ready for Sentry yet? Set `enable_sentry = false` to ship Cloud Run first;
    the DSN secret is created empty and the web service still boots (telemetry just
    no-ops without a DSN).

## Wiring the DSNs into the binaries

The CLI + desktop binaries bake their DSN at **compile time** (`option_env!`), so
the release build needs them as env. After `terraform apply`:

```bash
terraform output -json sentry_dsns
# -> { "web": "...", "cli": "...", "desktop": "...", "site": "..." }
```

- Set the `cli` DSN as the GitHub Actions secret `DARKRUN_CLI_SENTRY_DSN`.
- Set the `desktop` DSN as `DARKRUN_DESKTOP_SENTRY_DSN`.
- `release.yml` reads both and compiles them in. The `web` DSN needs no manual
  step (Secret Manager → Cloud Run). The `site` DSN goes into the browser SDK.

## Deploying the web service from CI

`.github/workflows/deploy-web.yml` builds the image, pushes it to Artifact
Registry, and runs `terraform apply` with the freshly-tagged image. It auths via
**Workload Identity Federation** (no JSON key). Configure once:

- A WIF pool + provider trusting this repo, and a `deploy@darkrun.iam` service
  account with `run.admin`, `artifactregistry.writer`, `secretmanager.admin`,
  `iam.serviceAccountUser`, and `storage.objectAdmin` on the state bucket.
- Repo **secrets**: `GCP_WORKLOAD_IDENTITY_PROVIDER`, `GCP_DEPLOY_SA`,
  `GITHUB_CLIENT_SECRET`, `GITLAB_CLIENT_SECRET`, `SENTRY_AUTH_TOKEN`.
- Repo **vars**: `GITHUB_CLIENT_ID`, `GITLAB_CLIENT_ID`, `SENTRY_ORGANIZATION`,
  `SENTRY_TEAM`.

## Custom domain

`web_domain` (default `darkrun.ai`) creates a Cloud Run domain mapping. The domain
must be verified for the project first (`gcloud domains verify`, or Search Console),
then point the DNS records Cloud Run reports at it. Set `web_domain = ""` to skip
and use the `run.app` URL.

## Notes

- **C-free** holds end-to-end: the server image uses rustls (no openssl); the only
  apt package in the runtime is `ca-certificates`.
- The container serves the site from `/srv/site` (`DARKRUN_SITE_DIR`); the build
  normalizes the `dx bundle` output into it regardless of dx's exact path.
- State is versioned in `gs://darkrun-tfstate`. Never commit `terraform.tfvars`,
  `*.tfstate`, or `.terraform/` — the root `.gitignore` already excludes them.
