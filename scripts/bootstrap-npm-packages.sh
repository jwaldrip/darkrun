#!/usr/bin/env bash
# One-time: claim the npm package names so OIDC trusted publishing can be
# configured on them (npm requires a package to exist before you can attach a
# trusted publisher). Publishes a throwaway 0.0.1 placeholder for each name under
# the `bootstrap` dist-tag, so `latest` stays clean and the real 0.1.0 release
# (via OIDC) is unaffected.
#
# Prereqs: the `@darkrun` npm org exists and `npm whoami` works (npm login).
# Run from the repo root: ./scripts/bootstrap-npm-packages.sh
set -euo pipefail

PACKAGES=(
  "darkrun"
  "@darkrun/darwin-arm64"
  "@darkrun/darwin-x64"
  "@darkrun/linux-x64"
  "@darkrun/linux-arm64"
  "@darkrun/win32-x64"
)

who="$(npm whoami 2>/dev/null || true)"
if [ -z "$who" ]; then
  echo "!! Not logged in to npm. Run: npm login" >&2
  exit 1
fi
echo "==> Publishing as: $who"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for name in "${PACKAGES[@]}"; do
  if npm view "${name}@0.0.1" version >/dev/null 2>&1; then
    echo "==> ${name}@0.0.1 already published, skipping."
    continue
  fi
  dir="${tmp}/$(echo "$name" | tr '/@' '__')"
  mkdir -p "$dir"
  cat > "${dir}/package.json" <<EOF
{
  "name": "${name}",
  "version": "0.0.1",
  "description": "Placeholder to reserve the name for OIDC trusted publishing. Replaced by the real release.",
  "license": "SEE LICENSE IN LICENSE"
}
EOF
  echo "==> Publishing ${name}@0.0.1 (dist-tag: bootstrap)..."
  ( cd "$dir" && npm publish --access public --tag bootstrap )
done

echo
echo "==> Done. All 6 names are claimed. Next:"
echo "    1. On npmjs.com, add a Trusted Publisher to each package:"
echo "         repo:     darkrun-ai/darkrun"
echo "         workflow: release.yml"
echo "    2. Switch release.yml to OIDC (drop NPM_TOKEN; add id-token: write)."
