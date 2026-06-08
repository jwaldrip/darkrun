#!/usr/bin/env bash
# One-time setup of the HCP Terraform (Terraform Cloud) -> GCP dynamic-credentials
# trust, so the `darkrun-infra` workspace's remote runs authenticate to GCP with
# NO static key. Run once with a darkrun-scoped owner/editor identity.
#
# It creates: a Workload Identity Pool + OIDC provider trusting app.terraform.io
# (locked to our org + workspace), a deploy service account, its project roles,
# and the impersonation binding. Idempotent.
#
# After running, set the printed workspace variables in TFC (see the tail output).
set -euo pipefail

PROJECT="${GCP_PROJECT:-darkrun}"
POOL="${TFC_POOL:-tfc-pool}"
PROVIDER="${TFC_PROVIDER:-tfc-provider}"
TF_ORG="${TFC_ORG:-darkrun-ai}"
TF_WORKSPACE="${TFC_WORKSPACE:-darkrun-infra}"
SA_ID="tfc-deploy"
SA_EMAIL="${SA_ID}@${PROJECT}.iam.gserviceaccount.com"

NUM="$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')"

echo "==> Enabling WIF/STS APIs..."
gcloud services enable \
  iam.googleapis.com iamcredentials.googleapis.com sts.googleapis.com \
  cloudresourcemanager.googleapis.com --project "$PROJECT"

echo "==> Workload Identity Pool..."
if ! gcloud iam workload-identity-pools describe "$POOL" \
       --location=global --project "$PROJECT" >/dev/null 2>&1; then
  gcloud iam workload-identity-pools create "$POOL" \
    --location=global --project "$PROJECT" --display-name="HCP Terraform"
fi

echo "==> OIDC provider (trusts app.terraform.io, locked to ${TF_ORG}/${TF_WORKSPACE})..."
if ! gcloud iam workload-identity-pools providers describe "$PROVIDER" \
       --location=global --workload-identity-pool="$POOL" --project "$PROJECT" >/dev/null 2>&1; then
  gcloud iam workload-identity-pools providers create-oidc "$PROVIDER" \
    --location=global --workload-identity-pool="$POOL" --project "$PROJECT" \
    --issuer-uri="https://app.terraform.io" \
    --attribute-mapping="google.subject=assertion.sub,attribute.terraform_organization_name=assertion.terraform_organization_name,attribute.terraform_workspace_name=assertion.terraform_workspace_name,attribute.terraform_run_phase=assertion.terraform_run_phase" \
    --attribute-condition="assertion.terraform_organization_name == \"${TF_ORG}\" && assertion.terraform_workspace_name == \"${TF_WORKSPACE}\""
fi

echo "==> Deploy service account..."
if ! gcloud iam service-accounts describe "$SA_EMAIL" --project "$PROJECT" >/dev/null 2>&1; then
  gcloud iam service-accounts create "$SA_ID" --project "$PROJECT" \
    --display-name="HCP Terraform deploy (${TF_WORKSPACE})"
fi

echo "==> Project roles (least-privilege for this stack)..."
for ROLE in \
  roles/run.admin \
  roles/artifactregistry.admin \
  roles/secretmanager.admin \
  roles/iam.serviceAccountAdmin \
  roles/iam.serviceAccountUser \
  roles/serviceusage.serviceUsageAdmin \
  roles/dns.admin ; do
  gcloud projects add-iam-policy-binding "$PROJECT" \
    --member="serviceAccount:${SA_EMAIL}" --role="$ROLE" \
    --condition=None --quiet >/dev/null
  echo "  + $ROLE"
done

echo "==> Impersonation binding (workspace ${TF_WORKSPACE} -> SA)..."
gcloud iam service-accounts add-iam-policy-binding "$SA_EMAIL" --project "$PROJECT" \
  --role="roles/iam.workloadIdentityUser" \
  --member="principalSet://iam.googleapis.com/projects/${NUM}/locations/global/workloadIdentityPools/${POOL}/attribute.terraform_workspace_name/${TF_WORKSPACE}" \
  --quiet >/dev/null

cat <<EOF

==> Done. Set these ENVIRONMENT variables on the TFC '${TF_WORKSPACE}' workspace:

  TFC_GCP_PROVIDER_AUTH              = true
  TFC_GCP_RUN_SERVICE_ACCOUNT_EMAIL = ${SA_EMAIL}
  TFC_GCP_WORKLOAD_PROVIDER_NAME     = projects/${NUM}/locations/global/workloadIdentityPools/${POOL}/providers/${PROVIDER}

Plus (also environment variables on the workspace):
  SENTRY_AUTH_TOKEN                 = <sentry token, sensitive>

That's it — remote runs will federate into GCP with no static key.
EOF
