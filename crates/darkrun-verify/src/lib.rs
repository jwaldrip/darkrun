//! darkrun-verify — the **objective verification engine**.
//!
//! darkrun's Prove station carries NUMBERS, not an agent's assertion that the
//! code looks good. A run's [`Surface`](darkrun_api::Surface) (classified at the
//! Shape station) routes which measurement applies, and this crate produces the
//! measurement:
//!
//! - **visual** surfaces (`web-ui` / `desktop` / `mobile`) → [`verify_web`]: a
//!   real headless Chrome (CDP via [`chromiumoxide`]) captures web vitals
//!   (LCP/FCP/CLS/TTFB + transfer size + JS heap), runs a11y/contrast/
//!   touch-target/reduced-motion/landmark/keyboard audits over the live DOM, and
//!   grabs a screenshot — shaped into a [`WebProof`](darkrun_api::WebProof).
//! - **bench** surfaces (`library` / `api` / `data`) → [`load_http`] + a
//!   criterion microbench harness: a small HTTP load harness reduces latency
//!   samples into p50/p95/p99 + throughput — a
//!   [`BenchProof`](darkrun_api::BenchProof).
//!
//! Both map into the [`darkrun_api::Proof`] payload the Prove station attaches.
//!
//! ## Browser backend
//!
//! **chromiumoxide (Chrome DevTools Protocol).** It drives a Chrome/Chromium
//! binary directly over CDP — no Node runtime — which gives `performance.*`
//! navigation/paint metrics and screenshots natively. Resolve the binary with
//! `$DARKRUN_CHROME` or `$CHROME`; otherwise chromiumoxide auto-detects.
//!
//! ## Testability
//!
//! The browser only *collects* a serializable [`DomSnapshot`](audit::DomSnapshot)
//! and [`PageVitals`](audit::PageVitals). Every analyzer and the proof-shaping
//! run as pure Rust over those structs, so the audit logic and the load-harness
//! percentile math are fully unit-tested with no browser and no network in CI
//! (the load harness is integration-tested against a local axum stub).

pub mod audit;
pub mod bench;
pub mod error;
pub mod web;

pub use audit::{
    audit_snapshot, ContrastSample, DomSnapshot, ImageInfo, PageVitals, TouchTarget,
};
pub use bench::{bench_proof_into, load_http, summarize, LoadOpts};
pub use error::{Result, VerifyError};
pub use web::{shape_web_proof, validate_target, verify_web, web_proof_into, WebOpts};

// Re-export the surface-routed proof types so callers can stay on one import.
pub use darkrun_api::{AuditResult, BenchProof, Proof, Surface, WebProof};

/// Parse a free-text surface token into a [`Surface`], tolerating the common
/// aliases (`web-ui`/`webui`/`web`, `lib`) and any casing/whitespace.
///
/// `darkrun_api::Surface` mirrors `darkrun_core::Surface` on the wire but omits
/// the lenient `parse` helper, so this crate (which owns the verification CLI)
/// provides it for routing a `--surface` flag.
pub fn parse_surface(raw: &str) -> Option<Surface> {
    match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "library" | "lib" => Some(Surface::Library),
        "api" => Some(Surface::Api),
        "web_ui" | "webui" | "web" => Some(Surface::WebUi),
        "tui" => Some(Surface::Tui),
        "cli" => Some(Surface::Cli),
        "desktop" => Some(Surface::Desktop),
        "mobile" => Some(Surface::Mobile),
        "data" => Some(Surface::Data),
        _ => None,
    }
}

/// Capture a surface-tagged [`Proof`] for a visual surface by driving the
/// headless browser. A thin convenience over [`verify_web`] +
/// [`web_proof_into`] for the CLI.
#[cfg(not(tarpaulin_include))] // drives the headless browser — irreducible I/O (web_proof_into is tested)
pub async fn prove_web(url: &str, surface: Surface, opts: &WebOpts) -> Result<Proof> {
    let web = verify_web(url, opts).await?;
    Ok(web_proof_into(web, surface))
}

/// Capture a surface-tagged [`Proof`] for a bench surface by running the HTTP
/// load harness. A thin convenience over [`load_http`] + [`bench_proof_into`].
pub async fn prove_load(url: &str, surface: Surface, opts: &LoadOpts) -> Result<Proof> {
    let bench = load_http(url, opts).await?;
    Ok(bench_proof_into(bench, surface))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prove_web_surface_must_be_visual_for_a_matching_block() {
        // The shaping is surface-agnostic, but a web proof only *matches* a
        // visual surface — this guards the routing contract callers rely on.
        let proof = web_proof_into(WebProof::default(), Surface::WebUi);
        assert!(proof.block_matches_surface());
        let mismatched = web_proof_into(WebProof::default(), Surface::Api);
        assert!(!mismatched.block_matches_surface());
    }

    #[test]
    fn parse_surface_tolerates_aliases_and_casing() {
        assert_eq!(parse_surface("web-ui"), Some(Surface::WebUi));
        assert_eq!(parse_surface("WEBUI"), Some(Surface::WebUi));
        assert_eq!(parse_surface("  web  "), Some(Surface::WebUi));
        assert_eq!(parse_surface("lib"), Some(Surface::Library));
        assert_eq!(parse_surface("Data"), Some(Surface::Data));
        assert_eq!(parse_surface("telepathy"), None);
    }

    #[test]
    fn re_exports_are_reachable() {
        // Compile-time proof that the public surface is wired.
        let _: DomSnapshot = DomSnapshot::default();
        let _: PageVitals = PageVitals::default();
        let _ = summarize(&[1.0], std::time::Duration::from_secs(1));
    }
}
