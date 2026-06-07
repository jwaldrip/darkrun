#!/usr/bin/env bash
# One-time bootstrap for the darkrun stack. State + runs live in HCP Terraform
# (Terraform Cloud), so there is no state bucket to create — this script handles
# the two things Terraform can't do for itself:
#
#   1. Enable the GCP APIs the first plan needs to authenticate + read.
#   2. Create the operator-managed OAuth secrets in Secret Manager. Their VALUES
#      never pass through Terraform — they live only here, in Google.
#
# Run once, with a darkrun-scoped identity that has owner/editor on the project,
# BEFORE the first `terraform init`. Idempotent: re-running is safe.
#
#   ./infra/bootstrap.sh
#
# Secret values are read from env vars if set (CI-friendly), else prompted
# (hidden). Leave a prompt blank to create the empty secret and populate it later
# with `gcloud secrets versions add <NAME> --data-file=-`.
set -euo pipefail

PROJECT="${GCP_PROJECT:-darkrun}"

echo "==> Project: ${PROJECT}"

# Guard: make sure the active gcloud identity is darkrun-scoped.
active_account="$(gcloud config get-value account 2>/dev/null || true)"
echo "==> Active gcloud account: ${active_account:-<none>}"
if ! gcloud projects describe "${PROJECT}" >/dev/null 2>&1; then
  echo "!! Cannot access project '${PROJECT}' as '${active_account}'."
  echo "!! Authenticate as a darkrun-scoped identity first."
  exit 1
fi

# 1. Enable required APIs.
echo
echo "==> Enabling required APIs..."
gcloud services enable \
  run.googleapis.com \
  artifactregistry.googleapis.com \
  secretmanager.googleapis.com \
  iam.googleapis.com \
  cloudresourcemanager.googleapis.com \
  --project "${PROJECT}"

# 2. Artifact Registry repo (a bootstrap resource — both Cloud Build and Cloud Run
#    need it before either runs, so it's created here and only referenced by TF).
echo
REGION="${GCP_REGION:-us-central1}"
if gcloud artifacts repositories describe darkrun --location="$REGION" --project "$PROJECT" >/dev/null 2>&1; then
  echo "==> Artifact Registry repo 'darkrun' exists."
else
  echo "==> Creating Artifact Registry repo 'darkrun'..."
  gcloud artifacts repositories create darkrun --location="$REGION" --project "$PROJECT" \
    --repository-format=docker --description="darkrun container images (darkrun-web)."
fi

# 3. Operator-managed OAuth secrets (stored ONLY in Secret Manager).
create_secret() {
  local name="$1"
  if gcloud secrets describe "${name}" --project "${PROJECT}" >/dev/null 2>&1; then
    echo "==> Secret ${name} exists."
  else
    echo "==> Creating secret ${name}..."
    gcloud secrets create "${name}" --project "${PROJECT}" --replication-policy=automatic
  fi

  # Value from env var of the same name, else an interactive hidden prompt.
  local val="${!name:-}"
  if [ -z "${val}" ]; then
    read -r -s -p "    Value for ${name} (blank to skip): " val; echo
  fi
  if [ -n "${val}" ]; then
    printf '%s' "${val}" | gcloud secrets versions add "${name}" --project "${PROJECT}" --data-file=-
    echo "    + added a version to ${name}"
  else
    echo "    (skipped — add later: gcloud secrets versions add ${name} --data-file=-)"
  fi
}

echo
echo "==> OAuth secrets (stored only in Secret Manager, never in Terraform)..."
create_secret GITHUB_CLIENT_ID
create_secret GITHUB_CLIENT_SECRET
create_secret GITLAB_CLIENT_ID
create_secret GITLAB_CLIENT_SECRET

echo
echo "==> Bootstrap complete. Next:"
echo "      terraform login                       # auth to HCP Terraform"
echo "      cd infra"
echo "      cp terraform.tfvars.example terraform.tfvars   # non-secret knobs only"
echo "      export SENTRY_AUTH_TOKEN=...          # or set it as a TFC workspace var"
echo "      terraform init"
echo "      terraform apply"
