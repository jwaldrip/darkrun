#!/usr/bin/env bash
# Bootstrap GitHub Actions -> GCP auth (Workload Identity Federation) so the
# `deploy-web.yml` workflow can build + push the darkrun-web image with no static
# key. Run this ONCE, as a project owner, then set the two repo secrets it prints
# (or let the assistant set them).
#
# It creates:
#   - a Workload Identity Pool + OIDC provider for GitHub Actions, RESTRICTED to
#     the darkrun-ai/darkrun repo (the attribute-condition is the security gate —
#     no other repo can mint a token this provider trusts),
#   - an IAM binding letting that repo impersonate the build service account,
#   - (idempotent) the Artifact Registry writer role on that SA so it can push.
#
# Safe to re-run: every step tolerates "already exists".
set -euo pipefail

PROJECT="darkrun"
PROJECT_NUM="32118591905"
REPO="darkrun-ai/darkrun"
SA="cloudbuild-web@darkrun.iam.gserviceaccount.com"
POOL="github-pool"
PROVIDER="github-provider"
LOCATION="global"

echo "==> Project: ${PROJECT} (${PROJECT_NUM}), repo: ${REPO}, build SA: ${SA}"

echo "==> 1/4 Workload Identity Pool '${POOL}'"
gcloud iam workload-identity-pools create "${POOL}" \
  --project="${PROJECT}" --location="${LOCATION}" \
  --display-name="GitHub Actions" \
  2>/dev/null || echo "    (pool already exists — ok)"

echo "==> 2/4 OIDC provider '${PROVIDER}' (restricted to ${REPO})"
gcloud iam workload-identity-pools providers create-oidc "${PROVIDER}" \
  --project="${PROJECT}" --location="${LOCATION}" \
  --workload-identity-pool="${POOL}" \
  --display-name="GitHub OIDC" \
  --issuer-uri="https://token.actions.githubusercontent.com" \
  --attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository" \
  --attribute-condition="assertion.repository=='${REPO}'" \
  2>/dev/null || echo "    (provider already exists — ok)"

echo "==> 3/4 Let ${REPO} impersonate ${SA}"
gcloud iam service-accounts add-iam-policy-binding "${SA}" \
  --project="${PROJECT}" \
  --role="roles/iam.workloadIdentityUser" \
  --member="principalSet://iam.googleapis.com/projects/${PROJECT_NUM}/locations/${LOCATION}/workloadIdentityPools/${POOL}/attribute.repository/${REPO}" \
  --condition=None >/dev/null

echo "==> 4/4 Ensure ${SA} can push to Artifact Registry"
gcloud projects add-iam-policy-binding "${PROJECT}" \
  --member="serviceAccount:${SA}" \
  --role="roles/artifactregistry.writer" \
  --condition=None >/dev/null

PROVIDER_RESOURCE="projects/${PROJECT_NUM}/locations/${LOCATION}/workloadIdentityPools/${POOL}/providers/${PROVIDER}"

cat <<EOF

==> Done. Set these two repo secrets (the assistant can do this for you):

  GCP_WORKLOAD_IDENTITY_PROVIDER = ${PROVIDER_RESOURCE}
  GCP_BUILD_SA                   = ${SA}

  gh secret set GCP_WORKLOAD_IDENTITY_PROVIDER --body "${PROVIDER_RESOURCE}"
  gh secret set GCP_BUILD_SA --body "${SA}"
EOF
