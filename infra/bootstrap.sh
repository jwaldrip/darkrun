#!/usr/bin/env bash
# One-time bootstrap for the darkrun Terraform state + required GCP APIs.
#
# Terraform can create everything EXCEPT its own state bucket (chicken/egg), so
# this script creates the GCS state bucket and turns on the APIs Terraform needs
# to authenticate against. Run it once, with an identity that has owner/editor on
# the `darkrun` project, BEFORE the first `terraform init`.
#
#   ./infra/bootstrap.sh
#
# Idempotent: re-running it is safe (skips anything already present).
set -euo pipefail

PROJECT="${GCP_PROJECT:-darkrun}"
REGION="${GCP_REGION:-us-central1}"
STATE_BUCKET="${TF_STATE_BUCKET:-darkrun-tfstate}"

echo "==> Project:       ${PROJECT}"
echo "==> Region:        ${REGION}"
echo "==> State bucket:  gs://${STATE_BUCKET}"
echo

# Guard: make sure the active gcloud identity is actually pointed at darkrun.
active_account="$(gcloud config get-value account 2>/dev/null || true)"
active_project="$(gcloud config get-value project 2>/dev/null || true)"
echo "==> Active gcloud account: ${active_account:-<none>}"
echo "==> Active gcloud project: ${active_project:-<none>}"
if [ "${active_project}" != "${PROJECT}" ]; then
  echo
  echo "!! Active project is '${active_project}', not '${PROJECT}'."
  echo "!! Set it first:  gcloud config set project ${PROJECT}"
  echo "!! (and confirm the account above is darkrun-scoped, not a borrowed CI SA)."
  read -r -p "Continue anyway? [y/N] " ok
  [ "${ok}" = "y" ] || { echo "Aborted."; exit 1; }
fi

# 1. Enable the APIs Terraform needs (Terraform also declares these in apis.tf,
#    but the very first apply needs them on already to read state + plan).
echo
echo "==> Enabling required APIs..."
gcloud services enable \
  run.googleapis.com \
  artifactregistry.googleapis.com \
  secretmanager.googleapis.com \
  iam.googleapis.com \
  cloudresourcemanager.googleapis.com \
  --project "${PROJECT}"

# 2. Create the Terraform state bucket (versioned, uniform access). The `gcs`
#    backend in versions.tf points here.
echo
if gcloud storage buckets describe "gs://${STATE_BUCKET}" --project "${PROJECT}" >/dev/null 2>&1; then
  echo "==> State bucket gs://${STATE_BUCKET} already exists, skipping create."
else
  echo "==> Creating state bucket gs://${STATE_BUCKET}..."
  gcloud storage buckets create "gs://${STATE_BUCKET}" \
    --project "${PROJECT}" \
    --location "${REGION}" \
    --uniform-bucket-level-access \
    --public-access-prevention
fi

echo "==> Enabling object versioning on the state bucket..."
gcloud storage buckets update "gs://${STATE_BUCKET}" --versioning --project "${PROJECT}"

echo
echo "==> Bootstrap complete. Next:"
echo "      cd infra"
echo "      cp terraform.tfvars.example terraform.tfvars   # fill in secrets"
echo "      terraform init"
echo "      terraform apply"
