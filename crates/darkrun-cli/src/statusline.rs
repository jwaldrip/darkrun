//! The darkrun Claude Code status line.
//!
//! `darkrun statusline` renders a one-line indicator of where the active Run
//! sits in its factory — the station pipeline, the current station, its phase,
//! and a unit aggregate — for Claude Code's `statusLine` setting, rendered in
//! the factory vocabulary.
//!
//! ```text
//! darkrun · add-healthcheck · software ●●◉○○○ build ❯ execute · 3/8 units
//! ```
//!
//! - the **darkrun** wordmark brand mark (dark bold · run regular), then the Run slug, then the factory
//! - the **station pipeline**: `●` complete · `◉` active · `○` pending
//! - the active **station**, a flow mark (`❯` running · `⊘` gated at a
//!   non-auto Checkpoint), and the **phase** (color-coded)
//! - a unit aggregate: completed / total
//!
//! With no active Run (no `.darkrun/`, or outside a project) it prints nothing
//! and exits 0, so Claude Code shows whatever line you had before.
//!
//! `install` / `uninstall` wire (and restore) Claude Code's `statusLine`.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use darkrun_core::domain::{CheckpointKind, StationPhase, Status};
use darkrun_core::StateStore;

type Dyn = Box<dyn std::error::Error>;

const RESET: &str = "\x1b[0m";

/// The darkrun wordmark brand mark, rendered for the terminal: **dark** bold +
/// `run` regular, both in the accent (256-color 81). Prefixes every status line.
const BRAND: &str = "\x1b[1;38;5;81mdark\x1b[0m\x1b[38;5;81mrun\x1b[0m";

// Palette (xterm-256). The phase hues double as the design system's semantic
// accents (see [[darkrun-brand]]).
const C_SLUG: &str = "1;38;5;255"; // bright white bold — the run slug
const C_FACTORY: &str = "38;5;245"; // grey — the factory (methodology) name
const C_DONE: &str = "38;5;71"; // green — a completed station pip
const C_PENDING: &str = "38;5;243"; // dim grey — a pending pip
const C_DIM: &str = "38;5;240"; // delimiters + the unit aggregate
const C_SPEC: &str = "38;5;245"; // grey
const C_REVIEW: &str = "38;5;75"; // blue
const C_MANUFACTURE: &str = "38;5;81"; // cyan (the accent)
const C_AUDIT: &str = "38;5;214"; // amber
const C_REFLECT: &str = "38;5;141"; // violet
const C_CHECKPOINT: &str = "38;5;170"; // magenta

fn paint(code: &str, s: &str) -> String {
    format!("\x1b[{code}m{s}{RESET}")
}

/// Map a [`StationPhase`] to its `(label, SGR color code)` for the status line.
/// The hues double as the design system's semantic accents — each phase gets a
/// distinct color so the active station's phase is legible at a glance.
fn phase_chrome(phase: StationPhase) -> (&'static str, &'static str) {
    match phase {
        StationPhase::Spec => ("spec", C_SPEC),
        StationPhase::Review => ("review", C_REVIEW),
        StationPhase::Manufacture => ("manufacture", C_MANUFACTURE),
        StationPhase::Audit => ("audit", C_AUDIT),
        StationPhase::Reflect => ("reflect", C_REFLECT),
        StationPhase::UserGate => ("gate", C_CHECKPOINT),
        StationPhase::Checkpoint => ("checkpoint", C_CHECKPOINT),
    }
}

/// Wrap `text` in an OSC 8 terminal hyperlink to `url`, so the chip is
/// clickable in terminals that support it (no-op visually elsewhere).
fn osc8(url: &str, text: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// The darkrun.ai site base for deep links, overridable via `DARKRUN_WEB_BASE`.
fn web_base() -> String {
    std::env::var("DARKRUN_WEB_BASE")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://darkrun.ai".to_string())
}

/// Parse the repo's `origin` remote into `(host, owner, repo)` for browse
/// links. `None` for local-only repos — the slug then renders unlinked.
fn origin_coords(root: &Path) -> Option<(String, String, String)> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_git_url(String::from_utf8(out.stdout).ok()?.trim())
}

/// Pull `(host, owner, repo)` out of an scp-like or URL git remote. `repo` may
/// contain slashes (GitLab subgroups), so it absorbs the trailing segments.
fn parse_git_url(url: &str) -> Option<(String, String, String)> {
    let s = url.strip_suffix(".git").unwrap_or(url);
    let rest = if let Some(idx) = s.find("://") {
        let after = &s[idx + 3..];
        after.split_once('@').map_or(after, |(_, h)| h).to_string()
    } else if let Some(idx) = s.find('@') {
        s[idx + 1..].replacen(':', "/", 1)
    } else {
        s.to_string()
    };
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 {
        return None;
    }
    Some((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2..].join("/"),
    ))
}

/// Render the status line. Returns `None` when there is no active Run, in which
/// case the caller prints nothing.
pub fn render(repo_override: Option<PathBuf>) -> Option<String> {
    let root = repo_override
        .or_else(read_cwd_from_stdin)
        .or_else(|| std::env::current_dir().ok())?;

    let store = StateStore::new(&root);
    let slug = store.active_run().ok().flatten()?;
    let run = store.read_run(&slug).ok()?;
    let factory = darkrun_content::load_factory(&run.frontmatter.factory).ok()?;
    let state = store.read_state(&slug).ok().flatten();

    let active_station = state
        .as_ref()
        .map(|s| s.active_station.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| run.frontmatter.active_station.clone());

    // Phase + flow mark from the active station's derived state — computed
    // before the pipeline so the active pip can take the phase hue.
    let (phase_label, phase_code, gated) =
        match state.as_ref().and_then(|s| s.stations.get(&active_station)) {
            Some(st) => {
                let (label, code) = phase_chrome(st.phase);
                // The pre-execution USER gate is always an operator hold; the
                // post-execution Checkpoint is gated only for non-auto kinds.
                let gated = matches!(st.phase, StationPhase::UserGate)
                    || (matches!(st.phase, StationPhase::Checkpoint)
                        && st
                            .checkpoint
                            .as_ref()
                            .is_some_and(|c| c.kind != CheckpointKind::Auto));
                (label, code, gated)
            }
            None => ("spec", C_SPEC, false),
        };

    // Station pipeline, in factory order: complete (green `●`) · active
    // (phase-hued `◉`) · pending (dim `○`).
    let order: Vec<&str> = factory.stations.iter().map(|s| s.name()).collect();
    let active_idx = order.iter().position(|n| *n == active_station);
    let mut pipeline = String::new();
    for (i, name) in order.iter().enumerate() {
        let completed = state
            .as_ref()
            .and_then(|s| s.stations.get(*name))
            .is_some_and(|st| matches!(st.status, Status::Completed));
        if Some(i) == active_idx {
            pipeline.push_str(&paint(phase_code, "◉"));
        } else if completed || active_idx.is_some_and(|a| i < a) {
            pipeline.push_str(&paint(C_DONE, "●"));
        } else {
            pipeline.push_str(&paint(C_PENDING, "○"));
        }
    }

    // Unit aggregate.
    let (done, total) = match store.read_units(&slug) {
        Ok(units) => (
            units
                .iter()
                .filter(|u| matches!(u.status(), Status::Completed))
                .count(),
            units.len(),
        ),
        Err(_) => (0, 0),
    };

    // Clickable chips (OSC 8) → darkrun.ai routes. The wordmark links home; the
    // station links its definition page; the slug links the run's browse page
    // (only when the repo has a parseable `origin`).
    let base = web_base();
    let coords = origin_coords(&root);
    let brand = osc8(&base, BRAND);
    let slug_painted = paint(C_SLUG, &slug);
    let slug_disp = match &coords {
        Some((h, o, r)) => osc8(
            &format!("{base}/browse/{h}/{o}/{r}/run/{slug}/"),
            &slug_painted,
        ),
        None => slug_painted,
    };
    // The factory (methodology) driving the run, linked to its catalog page.
    let factory_disp = osc8(
        &format!("{base}/factories/{}/", run.frontmatter.factory),
        &paint(C_FACTORY, &run.frontmatter.factory),
    );
    let station_disp = osc8(
        &format!(
            "{base}/factories/{}/stations/{}/",
            run.frontmatter.factory, active_station
        ),
        &paint(phase_code, &active_station),
    );
    let flow = if gated {
        paint(C_REVIEW, "⊘")
    } else {
        paint(C_DIM, "❯")
    };
    let phase_disp = paint(phase_code, phase_label);
    let sep = paint(C_DIM, "·");

    let mut line = format!(
        "{brand} {sep} {slug_disp} {sep} {factory_disp} {pipeline} {station_disp} {flow} {phase_disp}"
    );
    if total > 0 {
        line.push_str(&format!(
            " {sep} {}",
            paint(C_DIM, &format!("{done}/{total} units"))
        ));
    }
    Some(line)
}

/// Claude Code pipes a JSON blob to the status-line command. Pull the workspace
/// directory out of it so we root the store at the user's project, not wherever
/// Claude launched us. Skips reading when stdin is a TTY (manual invocation).
fn read_cwd_from_stdin() -> Option<PathBuf> {
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).ok()?;
    let v: serde_json::Value = serde_json::from_str(buf.trim()).ok()?;
    let dir = v
        .get("workspace")
        .and_then(|w| w.get("current_dir").or_else(|| w.get("project_dir")))
        .or_else(|| v.get("cwd"))
        .and_then(|s| s.as_str())?;
    Some(PathBuf::from(dir))
}

// ─── install / uninstall ─────────────────────────────────────────────────

fn home() -> Result<PathBuf, Dyn> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn settings_path(global: bool, repo: &Path) -> Result<PathBuf, Dyn> {
    let base = if global { home()? } else { repo.to_path_buf() };
    Ok(base.join(".claude").join("settings.json"))
}

fn fallback_path(global: bool, repo: &Path) -> Result<PathBuf, Dyn> {
    let base = if global {
        home()?.join(".darkrun")
    } else {
        repo.join(".darkrun")
    };
    Ok(base.join("statusline-fallback.json"))
}

fn read_json(path: &Path) -> Result<serde_json::Value, Dyn> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let raw = std::fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    Ok(serde_json::from_str(&raw)?)
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), Dyn> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))?;
    Ok(())
}

/// Wire Claude Code's `statusLine` to `darkrun statusline`, saving any existing
/// status line as a restorable fallback.
pub fn install(global: bool, repo: &Path, command: &str) -> Result<(), Dyn> {
    let settings_file = settings_path(global, repo)?;
    let mut settings = read_json(&settings_file)?;

    if let Some(existing) = settings.get("statusLine").cloned() {
        if existing.get("command").and_then(|c| c.as_str()) != Some(command) {
            write_json(&fallback_path(global, repo)?, &existing)?;
        }
    }

    settings["statusLine"] = serde_json::json!({
        "type": "command",
        "command": command,
        "padding": 0,
        "refreshInterval": 1,
    });
    write_json(&settings_file, &settings)?;
    println!(
        "darkrun statusline installed → {} ({})",
        settings_file.display(),
        if global { "global" } else { "project" }
    );
    Ok(())
}

/// Restore the previous status line (or remove the key if there was none).
pub fn uninstall(global: bool, repo: &Path) -> Result<(), Dyn> {
    let settings_file = settings_path(global, repo)?;
    let mut settings = read_json(&settings_file)?;

    let fallback = fallback_path(global, repo)?;
    if fallback.exists() {
        let prev = read_json(&fallback)?;
        settings["statusLine"] = prev;
        std::fs::remove_file(&fallback).ok();
    } else if let Some(obj) = settings.as_object_mut() {
        obj.remove("statusLine");
    }
    write_json(&settings_file, &settings)?;
    println!("darkrun statusline removed → {}", settings_file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{osc8, paint, parse_git_url, phase_chrome, web_base};
    use darkrun_core::domain::StationPhase;

    // ── phase chrome (label + hue) ───────────────────────────────────────

    #[test]
    fn phase_chrome_labels_match_the_new_taxonomy() {
        assert_eq!(phase_chrome(StationPhase::Spec).0, "spec");
        assert_eq!(phase_chrome(StationPhase::Review).0, "review");
        assert_eq!(phase_chrome(StationPhase::Manufacture).0, "manufacture");
        assert_eq!(phase_chrome(StationPhase::Audit).0, "audit");
        assert_eq!(phase_chrome(StationPhase::Reflect).0, "reflect");
        assert_eq!(phase_chrome(StationPhase::Checkpoint).0, "checkpoint");
    }

    #[test]
    fn reflect_has_a_hue_distinct_from_every_other_phase() {
        let reflect = phase_chrome(StationPhase::Reflect).1;
        for other in [
            StationPhase::Spec,
            StationPhase::Review,
            StationPhase::Manufacture,
            StationPhase::Audit,
            StationPhase::Checkpoint,
        ] {
            assert_ne!(
                reflect,
                phase_chrome(other).1,
                "reflect hue must differ from {:?}",
                other
            );
        }
    }

    #[test]
    fn every_phase_hue_is_unique() {
        let phases = [
            StationPhase::Spec,
            StationPhase::Review,
            StationPhase::Manufacture,
            StationPhase::Audit,
            StationPhase::Reflect,
            StationPhase::Checkpoint,
        ];
        for (i, a) in phases.iter().enumerate() {
            for b in &phases[i + 1..] {
                assert_ne!(
                    phase_chrome(*a).1,
                    phase_chrome(*b).1,
                    "hue collision between {:?} and {:?}",
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn reflect_hue_is_the_expected_violet() {
        assert_eq!(phase_chrome(StationPhase::Reflect).1, "38;5;141");
    }

    #[test]
    fn parse_git_url_https_github() {
        assert_eq!(
            parse_git_url("https://github.com/owner/repo.git"),
            Some(("github.com".into(), "owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_git_url_https_without_dot_git() {
        assert_eq!(
            parse_git_url("https://github.com/owner/repo"),
            Some(("github.com".into(), "owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_git_url_scp_style() {
        assert_eq!(
            parse_git_url("git@github.com:owner/repo.git"),
            Some(("github.com".into(), "owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_git_url_https_with_userinfo() {
        // The leading `user@` (token in URL) is stripped from the host.
        assert_eq!(
            parse_git_url("https://x-token@gitlab.com/owner/repo.git"),
            Some(("gitlab.com".into(), "owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_git_url_gitlab_subgroups_absorb_into_repo() {
        // The trailing segments after host/owner all belong to the repo path.
        assert_eq!(
            parse_git_url("https://gitlab.com/group/subgroup/repo.git"),
            Some(("gitlab.com".into(), "group".into(), "subgroup/repo".into()))
        );
    }

    #[test]
    fn parse_git_url_ssh_url_form() {
        assert_eq!(
            parse_git_url("ssh://git@github.com/owner/repo.git"),
            Some(("github.com".into(), "owner".into(), "repo".into()))
        );
    }

    #[test]
    fn parse_git_url_rejects_too_few_segments() {
        assert_eq!(parse_git_url("https://github.com/owner"), None);
        assert_eq!(parse_git_url("github.com"), None);
        assert_eq!(parse_git_url(""), None);
    }

    #[test]
    fn parse_git_url_handles_trailing_slash_segments() {
        // Empty path segments are filtered out before counting.
        assert_eq!(
            parse_git_url("https://github.com/owner/repo/"),
            Some(("github.com".into(), "owner".into(), "repo".into()))
        );
    }

    // `DARKRUN_WEB_BASE` is process-global, so the three tests that mutate it
    // must not run concurrently — a parallel test setting/clearing it would race
    // this one's assertion. Serialize them through one lock.
    static WEB_BASE_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn web_base_defaults_when_unset() {
        let _g = WEB_BASE_ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("DARKRUN_WEB_BASE");
        assert_eq!(web_base(), "https://darkrun.ai");
    }

    #[test]
    fn web_base_trims_trailing_slash_and_whitespace() {
        let _g = WEB_BASE_ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DARKRUN_WEB_BASE", "  https://x.test/  ");
        assert_eq!(web_base(), "https://x.test");
        std::env::remove_var("DARKRUN_WEB_BASE");
    }

    #[test]
    fn web_base_blank_falls_back_to_default() {
        let _g = WEB_BASE_ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DARKRUN_WEB_BASE", "   ");
        assert_eq!(web_base(), "https://darkrun.ai");
        std::env::remove_var("DARKRUN_WEB_BASE");
    }

    #[test]
    fn paint_wraps_in_sgr_and_reset() {
        let s = paint("38;5;81", "hi");
        assert!(s.starts_with("\x1b[38;5;81m"));
        assert!(s.ends_with("\x1b[0m"));
        assert!(s.contains("hi"));
    }

    #[test]
    fn osc8_wraps_text_in_a_hyperlink() {
        let s = osc8("https://x.test", "label");
        assert!(s.starts_with("\x1b]8;;https://x.test\x1b\\"));
        assert!(s.contains("label"));
        assert!(s.ends_with("\x1b]8;;\x1b\\"));
    }
}
