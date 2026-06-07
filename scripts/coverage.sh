#!/usr/bin/env bash
# darkrun workspace line-coverage gate.
#
# Mirrors the exclusion manifest documented in tarpaulin.toml as explicit CLI
# flags so the gate is deterministic regardless of tarpaulin's config-merge
# behavior. Excludes binary entry points, irreducible runtime I/O, and the
# Dioxus view layer (GUI render functions; SSR-smoke-tested, not line-covered).
# The gate floor is 99%: every line a portable test can reach IS reached
# (measured set ~99.7%), and the 99% floor fails CI on any real regression
# without un-gating the ~29 irreducible fault/defensive arms (see tarpaulin.toml
# for the documented residual — they'd drag ~250-350 tested lines out of scope).
#
# Usage:
#   scripts/coverage.sh            # enforce the gate (fail-under 99)
#   scripts/coverage.sh --report   # measure only, no fail-under (for grinding)
set -euo pipefail

OUT_DIR="${COV_OUT_DIR:-/tmp/cov}"
mkdir -p "$OUT_DIR"

FAIL_FLAGS=(--fail-under 99)
if [[ "${1:-}" == "--report" ]]; then
  FAIL_FLAGS=()
fi

exec cargo tarpaulin --workspace --engine llvm --timeout 1200 \
  --exclude-files "target/*" \
  --exclude-files "crates/darkrun-mcp/src/server.rs" \
  --exclude-files "crates/darkrun-mcp/src/desktop.rs" \
  --exclude-files "crates/darkrun-http/src/listen.rs" \
  --exclude-files "**/src/main.rs" \
  --exclude-files "**/src/bin/*.rs" \
  --exclude-files "desktop/src/main.rs" \
  --exclude-files "desktop/src/review.rs" \
  --exclude-files "desktop/src/home.rs" \
  --exclude-files "crates/darkrun-ui/src/components/annotate.rs" \
  --exclude-files "crates/darkrun-ui/src/components/feedback.rs" \
  --exclude-files "crates/darkrun-ui/src/components/session_views.rs" \
  --exclude-files "crates/darkrun-ui/src/components/view_artifacts.rs" \
  --exclude-files "crates/darkrun-ui/src/components/output_review.rs" \
  --exclude-files "crates/darkrun-ui/src/components/walkthrough.rs" \
  --exclude-files "crates/darkrun-ui/src/components/station_flow.rs" \
  --exclude-files "crates/darkrun-ui/src/components/factory.rs" \
  --exclude-files "crates/darkrun-ui/src/components/phase_machine.rs" \
  --exclude-files "crates/darkrun-ui/src/components/primitives.rs" \
  --exclude-files "crates/darkrun-ui/src/components/role.rs" \
  --exclude-files "crates/darkrun-ui/src/components/run_list.rs" \
  --exclude-files "crates/darkrun-ui/src/components/station_strip.rs" \
  --exclude-files "crates/darkrun-ui/src/components/tab_bar.rs" \
  --exclude-files "crates/darkrun-ui/src/components/wordmark.rs" \
  --exclude-files "crates/darkrun-ui/src/graph/view.rs" \
  --exclude-files "web/site/src/layout.rs" \
  --exclude-files "web/site/src/ui.rs" \
  --exclude-files "web/site/src/lib.rs" \
  --exclude-files "web/site/src/theme_toggle.rs" \
  --exclude-files "web/site/src/pages/*.rs" \
  --out Lcov --output-dir "$OUT_DIR" \
  "${FAIL_FLAGS[@]}"
