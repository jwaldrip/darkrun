//! `darkrun verify web` / `darkrun bench` — the commands the agent runs so the
//! Prove station carries objective NUMBERS instead of an assertion.
//!
//! `verify web <url>` drives a real headless browser ([`darkrun_verify`]'s
//! chromiumoxide/CDP backend), prints the [`WebProof`](darkrun_verify::WebProof)
//! JSON, and saves the screenshot. `bench <url>` runs the HTTP load harness and
//! prints the [`BenchProof`](darkrun_verify::BenchProof) JSON. Both can wrap the
//! result in a surface-tagged [`Proof`](darkrun_verify::Proof) and write it to a
//! file for the Prove station to attach.

use std::path::PathBuf;
use std::time::Duration;

use darkrun_verify::{
    bench_proof_into, load_http, parse_surface, verify_web, web_proof_into, LoadOpts, Proof,
    Surface, WebOpts,
};

/// Errors surfaced to the CLI layer.
type CliResult = Result<(), Box<dyn std::error::Error>>;

/// Resolve a surface string (defaulting to a sensible per-command surface),
/// erroring on an unknown token so a typo doesn't silently mis-route.
fn resolve_surface(raw: Option<&str>, default: Surface) -> Result<Surface, Box<dyn std::error::Error>> {
    match raw {
        None => Ok(default),
        Some(s) => parse_surface(s)
            .ok_or_else(|| format!("unknown surface {s:?} — expected one of library|api|web-ui|tui|cli|desktop|mobile|data").into()),
    }
}

/// `darkrun verify web <url>` — capture a [`WebProof`] via the headless browser.
///
/// Prints the proof (the raw `WebProof`, or a surface-tagged `Proof` when
/// `--surface` is given) as JSON, and — when `--out` is set — writes the same
/// JSON to disk. The screenshot is saved alongside (defaults to `<out>.png`, or
/// `proof.png` in the cwd).
#[allow(clippy::too_many_arguments)]
#[cfg(not(tarpaulin_include))] // drives a real headless browser
pub fn verify_web_command(
    url: String,
    out: Option<PathBuf>,
    shot: Option<PathBuf>,
    surface: Option<String>,
    width: u32,
    height: u32,
    settle_ms: u64,
    timeout_s: u64,
) -> CliResult {
    let screenshot_path = shot.or_else(|| Some(default_shot_path(out.as_ref())));
    let opts = WebOpts {
        screenshot_path: screenshot_path.clone(),
        settle: Duration::from_millis(settle_ms),
        width,
        height,
        timeout: Duration::from_secs(timeout_s),
    };

    let runtime = tokio::runtime::Runtime::new()?;
    let web = runtime.block_on(verify_web(&url, &opts))?;

    // Tag with a surface only when asked; otherwise print the bare WebProof.
    let json = match &surface {
        Some(_) => {
            let surface = resolve_surface(surface.as_deref(), Surface::WebUi)?;
            serde_json::to_string_pretty(&web_proof_into(web, surface))?
        }
        None => serde_json::to_string_pretty(&web)?,
    };

    println!("{json}");
    if let Some(out) = &out {
        std::fs::write(out, format!("{json}\n"))?;
        eprintln!("wrote proof -> {}", out.display());
    }
    if let Some(p) = &screenshot_path {
        eprintln!("wrote screenshot -> {}", p.display());
    }
    Ok(())
}

/// `darkrun bench <target>` — run the HTTP load harness and print a
/// [`BenchProof`].
#[allow(clippy::too_many_arguments)]
#[cfg(not(tarpaulin_include))] // runs a live HTTP load harness
pub fn bench_command(
    target: String,
    out: Option<PathBuf>,
    surface: Option<String>,
    requests: u64,
    concurrency: usize,
    timeout_s: u64,
) -> CliResult {
    let opts = LoadOpts {
        requests,
        concurrency,
        timeout: Duration::from_secs(timeout_s),
    };

    let runtime = tokio::runtime::Runtime::new()?;
    let bench = runtime.block_on(load_http(&target, &opts))?;

    let json = match &surface {
        Some(_) => {
            let surface = resolve_surface(surface.as_deref(), Surface::Api)?;
            let proof: Proof = bench_proof_into(bench, surface);
            serde_json::to_string_pretty(&proof)?
        }
        None => serde_json::to_string_pretty(&bench)?,
    };

    println!("{json}");
    if let Some(out) = &out {
        std::fs::write(out, format!("{json}\n"))?;
        eprintln!("wrote proof -> {}", out.display());
    }
    Ok(())
}

/// Default screenshot path: sibling of `--out` with a `.png` extension, else
/// `proof.png` in the current directory.
fn default_shot_path(out: Option<&PathBuf>) -> PathBuf {
    match out {
        Some(o) => o.with_extension("png"),
        None => PathBuf::from("proof.png"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_surface_defaults_and_parses_aliases() {
        assert_eq!(resolve_surface(None, Surface::WebUi).unwrap(), Surface::WebUi);
        assert_eq!(resolve_surface(Some("web-ui"), Surface::Api).unwrap(), Surface::WebUi);
        assert_eq!(resolve_surface(Some("lib"), Surface::Api).unwrap(), Surface::Library);
        assert_eq!(resolve_surface(Some("DESKTOP"), Surface::Api).unwrap(), Surface::Desktop);
    }

    #[test]
    fn resolve_surface_rejects_garbage() {
        let err = resolve_surface(Some("telepathy"), Surface::WebUi).unwrap_err();
        assert!(err.to_string().contains("unknown surface"));
    }

    #[test]
    fn default_shot_path_follows_out_then_falls_back() {
        assert_eq!(
            default_shot_path(Some(&PathBuf::from("evidence/home.json"))),
            PathBuf::from("evidence/home.png")
        );
        assert_eq!(default_shot_path(None), PathBuf::from("proof.png"));
    }
}
