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
// The slug and passed-track pips use BOLD DEFAULT-FG (no color): the terminal
// supplies its own theme's strong foreground, so they read on light terminals
// too — a fixed bright-white (255) disappears on a white background.
const C_SLUG: &str = "1"; // bold default-fg — the run slug
const C_FACTORY: &str = "38;5;245"; // grey — the factory (methodology) name
const C_DONE: &str = "38;5;71"; // green — a completed station pip
const C_PENDING: &str = "38;5;243"; // dim grey — a pending pip
const C_TRACK_DONE: &str = "1"; // bold default-fg — a passed phase pip in the track
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
        // The pre-execution USER gate is the review stage's operator hold — the
        // engine's universal taxonomy (and the desktop) surface it as `review`,
        // not a phase of its own. The HOLD is shown by the `⊘` flow mark, not the
        // label, so the statusline agrees with every other surface.
        StationPhase::UserGate => ("review", C_REVIEW),
        StationPhase::Checkpoint => ("checkpoint", C_CHECKPOINT),
    }
}

// ── The second (live pool) line — the predecessor's chip system, ported ─────
//
// Line 1 is position; line 2 is the LIVE POOL, led by a dim `↳`:
//   - Manufacture: one CHIP per current-station unit — a pastel near-white box
//     (dark bold label) + one `▰`/`▱` pip per worker beat: green done, YELLOW
//     in-progress (uniform "working" hue), soft-red rejected, grey pending.
//   - Open feedback: one chip per item, the BOX tinted by severity (light red /
//     orange / gold / near-white, lavender unclassified) with a NO_COLOR-legible
//     mark (`!^~.?`).
//   - Review/Audit awaits: AGENT chips — solid pastel status boxes per
//     reviewer role: pastel-green ✓ stamped, near-white ▸ being awaited,
//     grey queued.
// Mutually exclusive, first match wins; none → single line.

/// Filled / empty progress pips.
const PIP_DONE: &str = "\u{25b0}";
const PIP_PENDING: &str = "\u{25b1}";
/// Leader for the second line.
const ITEM_LEADER: &str = "\u{21b3}";

/// In-progress beats are ALWAYS this yellow (truecolor, bold) — one color for
/// "working" on every box tint.
const PIP_ACTIVE: &str = "1;38;2;255;255;0";

/// One worker-beat's visual state on a unit chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seg {
    Done,
    Active,
    Rejected,
    Pending,
}

/// Per-worker segments over a unit's recorded beats — the predecessor's
/// `hatSegments`, on darkrun's `iterations`. Each worker's base state comes
/// from its MOST RECENT beat (advance → done, reject → rejected, none →
/// pending); the single ACTIVE worker is derived after: next-after-advance,
/// the rejecting worker keeps its red, first when nothing ran yet. An
/// unstarted unit shows empty progress.
fn worker_segments(
    iters: &[darkrun_core::domain::UnitIteration],
    workers: &[String],
    started: bool,
) -> Vec<Seg> {
    use darkrun_core::domain::IterationResult as R;
    let mut recent: std::collections::HashMap<&str, Option<R>> = Default::default();
    for it in iters {
        recent.insert(it.worker.as_str(), it.result);
    }
    let mut segs: Vec<Seg> = workers
        .iter()
        .map(|w| match recent.get(w.as_str()) {
            Some(Some(R::Advance)) => Seg::Done,
            Some(Some(R::Reject)) => Seg::Rejected,
            _ => Seg::Pending,
        })
        .collect();
    if workers.is_empty() || !started {
        return segs;
    }
    if iters.is_empty() {
        segs[0] = Seg::Active;
        return segs;
    }
    let last = &iters[iters.len() - 1];
    let last_idx = workers.iter().position(|w| *w == last.worker);
    match (last.result, last_idx) {
        (None, Some(i)) => segs[i] = Seg::Active,
        (Some(R::Advance), Some(i)) => {
            let next = i + 1;
            if next < workers.len() && segs[next] == Seg::Pending {
                segs[next] = Seg::Active;
            }
        }
        _ => {} // reject keeps its red; no separate active slot
    }
    segs
}

/// A unit chip: pastel near-white box, dark bold id, per-worker pips. The
/// whole chip is the OSC 8 click target when a browse URL resolves.
fn unit_chip(id: &str, segs: &[Seg], url: Option<&str>) -> String {
    let pips: String = segs
        .iter()
        .map(|s| match s {
            Seg::Done => format!("\x1b[38;5;71m{PIP_DONE}"),
            Seg::Active => format!("\x1b[{PIP_ACTIVE}m{PIP_DONE}"),
            Seg::Rejected => format!("\x1b[1;38;5;167m{PIP_DONE}"),
            Seg::Pending => format!("\x1b[38;5;250m{PIP_PENDING}"),
        })
        .collect();
    let chip = format!(
        "\x1b[48;5;254m \x1b[1;38;5;238m{id} {pips} {RESET}"
    );
    match url {
        Some(u) => osc8(u, &chip),
        None => chip,
    }
}

/// A feedback chip: the BOX is the severity (predecessor palette), the mark
/// keeps it legible without color. darkrun feedback carries no per-beat
/// iterations, so the chip is the tinted id alone.
fn feedback_chip(id: &str, severity: Option<darkrun_core::domain::FeedbackSeverity>) -> String {
    use darkrun_core::domain::FeedbackSeverity as S;
    let (bg, mark) = match severity {
        Some(S::Blocker) => ("\x1b[48;5;210m", "!"),
        Some(S::High) => ("\x1b[48;5;216m", "^"),
        Some(S::Medium) => ("\x1b[48;5;223m", "~"),
        Some(S::Low) => ("\x1b[48;5;254m", "."),
        None => ("\x1b[48;5;189m", "?"), // unclassified → cool lavender
    };
    format!("{bg} \x1b[1;38;5;238m{mark}{id} {RESET}")
}

/// An agent (reviewer-role) chip for the await phases: the box color IS the
/// status — pastel green stamped, near-white being awaited, grey queued.
fn agent_chip(role: &str, done: bool, awaited: bool) -> String {
    let (bg, fg, mark) = if done {
        ("\x1b[48;5;151m", "\x1b[1;38;5;22m", " \u{2713}")
    } else if awaited {
        ("\x1b[48;5;254m", "\x1b[1;38;5;238m", " \u{25b8}")
    } else {
        ("\x1b[48;5;248m", "\x1b[38;5;240m", "")
    };
    format!("{bg}{fg} {role}{mark} {RESET}")
}

/// The most chips on the pool line before a `+N` overflow.
const MAX_POOL_CHIPS: usize = 6;

/// Build the second (live pool) line for the active station, or `None` when
/// there is nothing in flight to show.
fn pool_line(
    store: &darkrun_core::StateStore,
    slug: &str,
    factory: &darkrun_content::Factory,
    active_station: &str,
    phase: Option<StationPhase>,
    run_url: Option<&str>,
) -> Option<String> {
    let station_def = factory.station(active_station);
    let units: Vec<darkrun_core::domain::Unit> = store
        .read_units(slug)
        .unwrap_or_default()
        .into_iter()
        .filter(|u| u.station() == active_station)
        .collect();

    // Open feedback preempts (Track B preempts the run walk, so the pool
    // shows what the engine is actually working).
    let feedback: Vec<FbMin> = store
        .read_feedback_raw(slug)
        .unwrap_or_default()
        .iter()
        .filter_map(|(id, doc)| parse_feedback_min(id, doc))
        .filter(|f| f.open)
        .collect();
    if !feedback.is_empty() {
        let mut chips: Vec<String> = feedback
            .iter()
            .take(MAX_POOL_CHIPS)
            .map(|f| feedback_chip(&f.id, f.severity))
            .collect();
        if feedback.len() > MAX_POOL_CHIPS {
            chips.push(paint(C_DIM, &format!("+{}", feedback.len() - MAX_POOL_CHIPS)));
        }
        return Some(chips.join(" "));
    }

    match phase {
        Some(StationPhase::Manufacture) => {
            let workers: Vec<String> = station_def
                .map(|s| s.workers.iter().map(|w| w.name().to_string()).collect())
                .unwrap_or_default();
            if units.is_empty() {
                return None;
            }
            let mut chips: Vec<String> = units
                .iter()
                .take(MAX_POOL_CHIPS)
                .map(|u| {
                    let started = u.frontmatter.started_at.is_some()
                        || !u.frontmatter.iterations.is_empty();
                    let segs =
                        worker_segments(&u.frontmatter.iterations, &workers, started);
                    unit_chip(&u.slug, &segs, run_url)
                })
                .collect();
            if units.len() > MAX_POOL_CHIPS {
                chips.push(paint(C_DIM, &format!("+{}", units.len() - MAX_POOL_CHIPS)));
            }
            Some(chips.join(" "))
        }
        Some(StationPhase::Review) | Some(StationPhase::Audit) => {
            let reviewers: Vec<String> = station_def
                .map(|s| s.reviewers.iter().map(|r| r.name().to_string()).collect())
                .unwrap_or_default();
            if reviewers.is_empty() || units.is_empty() {
                return None;
            }
            let is_review = matches!(phase, Some(StationPhase::Review));
            // A role is DONE when every unit carries its stamp; the first
            // unstamped role is the one being awaited.
            let mut chips = Vec::new();
            let mut awaited_seen = false;
            for role in &reviewers {
                let done = units.iter().all(|u| {
                    let map = if is_review {
                        &u.frontmatter.reviews
                    } else {
                        &u.frontmatter.approvals
                    };
                    matches!(map.get(role), Some(Some(_)))
                });
                let awaited = !done && !awaited_seen;
                if awaited {
                    awaited_seen = true;
                }
                chips.push(agent_chip(role, done, awaited));
            }
            Some(chips.join(" "))
        }
        _ => None,
    }
}

/// The minimum of a feedback doc the pool line needs: id, open?, severity.
struct FbMin {
    id: String,
    open: bool,
    severity: Option<darkrun_core::domain::FeedbackSeverity>,
}

/// Parse just enough of a feedback document's frontmatter (status + severity)
/// for a chip. Tolerant: unparseable docs are skipped.
fn parse_feedback_min(id: &str, doc: &str) -> Option<FbMin> {
    use darkrun_core::domain::FeedbackSeverity as S;
    let rest = doc.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let fm = &rest[..end];
    let field = |key: &str| -> Option<String> {
        fm.lines().find_map(|l| {
            l.trim()
                .strip_prefix(key)
                .map(|v| v.trim().trim_matches('"').to_string())
        })
    };
    let status = field("status:").unwrap_or_default();
    let open = matches!(status.as_str(), "pending" | "fixing" | "escalated" | "");
    let severity = field("severity:").and_then(|s| match s.as_str() {
        "blocker" => Some(S::Blocker),
        "high" => Some(S::High),
        "medium" => Some(S::Medium),
        "low" => Some(S::Low),
        _ => None,
    });
    Some(FbMin { id: id.to_string(), open, severity })
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
    use darkrun_git::GitBackend;
    let url = darkrun_git::Git::open(root).ok()?.remote_url("origin").ok()??;
    parse_git_url(url.trim())
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

/// The in-station PHASE TRACK — a pip run right of the station name showing
/// where the six-phase machine stands: `▰` bright for phases already passed,
/// the active phase's pip in its hue (magenta when parked at a gate), `▱` dim
/// for phases not yet reached. `UserGate` (the pre-execution hold) sits on the
/// Manufacture slot it guards.
fn phase_track(active: Option<StationPhase>, phase_code: &str, gated: bool) -> String {
    let idx = match active {
        Some(StationPhase::Spec) => 0,
        Some(StationPhase::Review) => 1,
        Some(StationPhase::UserGate) | Some(StationPhase::Manufacture) => 2,
        Some(StationPhase::Audit) => 3,
        Some(StationPhase::Reflect) => 4,
        Some(StationPhase::Checkpoint) => 5,
        None => return String::new(),
    };
    let mut track = String::new();
    for i in 0..6 {
        if i < idx {
            track.push_str(&paint(C_TRACK_DONE, PIP_DONE));
        } else if i == idx {
            let hue = if gated { C_CHECKPOINT } else { phase_code };
            track.push_str(&paint(hue, PIP_DONE));
        } else {
            track.push_str(&paint(C_PENDING, PIP_PENDING));
        }
    }
    track
}

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
    let active_phase: Option<StationPhase> = state
        .as_ref()
        .and_then(|s| s.stations.get(&active_station))
        .map(|st| st.phase);
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
    // The flow mark: `❯` running, `Π` (a doorway) gated — magenta, the
    // "your turn" hue. The predecessor abandoned `⊘` because it reads as a
    // failure; a gate is a doorway you pass through, not an error.
    let flow = if gated {
        paint(C_CHECKPOINT, "Π")
    } else {
        paint(C_DIM, "❯")
    };
    let phase_disp = if gated {
        paint(C_CHECKPOINT, phase_label)
    } else {
        paint(phase_code, phase_label)
    };
    let sep = paint(C_DIM, "·");

    // The in-station phase track rides just right of the station name.
    let track = phase_track(active_phase, phase_code, gated);
    let track_disp = if track.is_empty() {
        String::new()
    } else {
        format!(" {track}")
    };
    let mut line = format!(
        "{brand} {sep} {slug_disp} {sep} {factory_disp} {pipeline} {station_disp}{track_disp} {flow} {phase_disp}"
    );
    if total > 0 {
        line.push_str(&format!(
            " {sep} {}",
            paint(C_DIM, &format!("{done}/{total} units"))
        ));
    }

    // The SECOND line — the live pool (the predecessor's chip system): unit
    // beat-progress bars during Manufacture, severity-tinted feedback chips
    // while the fix track preempts, reviewer-role status chips during the
    // Review/Audit awaits. Led by a dim `↳`; absent when nothing is in flight.
    let run_url = coords.as_ref().map(|(h, o, r)| {
        format!("{base}/browse/{h}/{o}/{r}/run/{slug}/")
    });
    if let Some(pool) = pool_line(
        &store,
        &slug,
        &factory,
        &active_station,
        active_phase,
        run_url.as_deref(),
    ) {
        line.push_str(&format!("\n{} {pool}", paint(C_DIM, ITEM_LEADER)));
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
    use super::{C_TRACK_DONE, 
        fallback_path, install, osc8, paint, parse_git_url, phase_chrome, read_json,
        render, settings_path, uninstall, web_base, write_json,
        ITEM_LEADER, PIP_DONE, PIP_PENDING,
    };
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

    #[test]
    fn phase_chrome_and_parse_git_url_cover_branches() {
        for p in [StationPhase::Spec, StationPhase::Review, StationPhase::Manufacture, StationPhase::Audit, StationPhase::Reflect, StationPhase::UserGate, StationPhase::Checkpoint] {
            let (label, _c) = phase_chrome(p);
            assert!(!label.is_empty());
        }
        assert!(parse_git_url("git@github.com:owner/repo.git").is_some());
        assert!(parse_git_url("https://gitlab.com/owner/repo.git").is_some());
        assert!(parse_git_url("garbage").is_none());
    }

    #[test]
    fn read_json_tolerates_missing_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.json");
        assert_eq!(read_json(&missing).unwrap(), serde_json::json!({}));
        let empty = dir.path().join("empty.json");
        std::fs::write(&empty, "   \n").unwrap();
        assert_eq!(read_json(&empty).unwrap(), serde_json::json!({}));
        let real = dir.path().join("real.json");
        write_json(&real, &serde_json::json!({"a": 1})).unwrap();
        assert_eq!(read_json(&real).unwrap()["a"], 1);
    }

    #[test]
    fn global_fallback_path_roots_under_home_darkrun() {
        // The global branch of fallback_path roots the saved status line under
        // `$HOME/.darkrun` rather than the repo.
        let repo = tempfile::tempdir().unwrap();
        let p = fallback_path(true, repo.path()).unwrap();
        assert!(p.to_string_lossy().contains(".darkrun"), "global fallback under .darkrun: {p:?}");
        assert!(!p.starts_with(repo.path()), "not under the repo");
    }

    #[test]
    fn render_tolerates_unreadable_units() {
        use darkrun_core::domain::{Run, RunFrontmatter};
        use darkrun_core::StateStore;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let store = StateStore::new(root);
        store.write_run(&Run {
            slug: "r".into(), title: "R".into(), body: String::new(),
            frontmatter: RunFrontmatter { factory: "software".into(), active_station: "frame".into(), ..Default::default() },
        }).unwrap();
        store.set_active_run("r").unwrap();
        // A corrupt unit makes read_units error → the unit tally degrades to 0/0
        // rather than failing the whole status line.
        let units = store.units_dir("r");
        std::fs::create_dir_all(&units).unwrap();
        std::fs::write(units.join("broken.md"), "---\nstatus: \"x\n---\n").unwrap();
        let line = render(Some(root.to_path_buf())).expect("still renders");
        // The unit tally degrades (read error → 0/0) without failing the line.
        assert!(line.contains('r'), "the run still renders: {line}");
    }

    #[test]
    fn install_then_uninstall_roundtrips_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let settings = settings_path(false, repo).unwrap();
        let fallback = fallback_path(false, repo).unwrap();

        // Pre-seed an existing, different status line so install saves a fallback.
        write_json(
            &settings,
            &serde_json::json!({"statusLine": {"type": "command", "command": "old-line"}}),
        )
        .unwrap();

        install(false, repo, "darkrun statusline").unwrap();
        let after = read_json(&settings).unwrap();
        assert_eq!(after["statusLine"]["command"], "darkrun statusline");
        assert!(fallback.exists(), "the prior status line is saved as a fallback");
        assert_eq!(read_json(&fallback).unwrap()["command"], "old-line");

        // Uninstall restores the saved fallback and clears the fallback file.
        uninstall(false, repo).unwrap();
        let restored = read_json(&settings).unwrap();
        assert_eq!(restored["statusLine"]["command"], "old-line");
        assert!(!fallback.exists(), "the fallback is consumed on restore");

        // Installing fresh (no prior statusLine) then uninstalling removes the key.
        let clean = tempfile::tempdir().unwrap();
        install(false, clean.path(), "darkrun statusline").unwrap();
        uninstall(false, clean.path()).unwrap();
        let gone = read_json(&settings_path(false, clean.path()).unwrap()).unwrap();
        assert!(gone.get("statusLine").is_none(), "no fallback → the key is removed");
    }

    #[test]
    fn render_builds_a_statusline_for_an_active_run() {
        use darkrun_core::domain::{Run, RunFrontmatter, Status, Unit, UnitFrontmatter};
        use darkrun_core::StateStore;
        use std::process::Command;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A git repo with a github origin → the slug chip becomes a browse link
        // (the origin_coords Some-arm).
        let git = |args: &[&str]| { Command::new("git").current_dir(root).args(args).output().unwrap(); };
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", "https://github.com/acme/store.git"]);

        let store = StateStore::new(root);
        let run = Run {
            slug: "store-r".into(),
            title: "Store".into(),
            body: String::new(),
            frontmatter: RunFrontmatter {
                factory: "software".into(),
                active_station: "frame".into(),
                ..Default::default()
            },
        };
        store.write_run(&run).unwrap();
        store.set_active_run("store-r").unwrap();
        // A completed + a pending unit → the `done/total units` tail renders.
        for (slug, status) in [("u1", Status::Completed), ("u2", Status::Pending)] {
            store.write_unit("store-r", &Unit {
                slug: slug.into(),
                frontmatter: UnitFrontmatter { status, station: Some("frame".into()), ..Default::default() },
                title: slug.into(),
                body: String::new(),
            }).unwrap();
        }

        let line = render(Some(root.to_path_buf())).expect("an active run renders a line");
        assert!(line.contains("store-r"), "the run slug is shown: {line}");
        assert!(line.contains("1/2 units"), "the unit tally renders: {line}");
        assert!(line.contains("github.com") || line.contains("/browse/"), "the slug links its browse page: {line}");

        // No active run → nothing to render.
        let empty = tempfile::tempdir().unwrap();
        assert!(render(Some(empty.path().to_path_buf())).is_none());
    }

    #[test]
    fn render_marks_a_gated_checkpoint_station() {
        use darkrun_core::domain::{
            Checkpoint, CheckpointKind, Run, RunFrontmatter, Station, StationPhase, Status,
        };
        use darkrun_core::state::RunState;
        use darkrun_core::StateStore;
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        store.write_run(&Run {
            slug: "r".into(), title: "T".into(), body: String::new(),
            frontmatter: RunFrontmatter { factory: "software".into(), active_station: "build".into(), ..Default::default() },
        }).unwrap();
        store.set_active_run("r").unwrap();
        // The active station sits at a non-auto Checkpoint → an operator hold.
        let mut state = RunState { factory: "software".into(), active_station: "build".into(), ..Default::default() };
        state.stations.insert("build".into(), Station {
            station: "build".into(), status: Status::Active, phase: StationPhase::Checkpoint,
            elaborated: true,
            checkpoint: Some(Checkpoint { kind: CheckpointKind::Ask, entered_at: None, outcome: None }),
            branch: None, pr_ref: None, pr_status: None,
            pr_ready_at: None, pr_merged_at: None, verifier_nonce: None,
            started_at: None, completed_at: None,
        });
        store.write_state("r", &state).unwrap();
        let line = render(Some(dir.path().to_path_buf())).expect("renders an active run");
        // The gated-hold flow mark (Π — a doorway, not a failure) appears for a
        // non-auto checkpoint, in the your-turn magenta.
        assert!(line.contains('Π'), "a gated checkpoint shows the doorway mark: {line}");
        assert!(!line.contains('⊘'), "the failure-reading glyph is gone: {line}");
    }

    /// Build a one-station run sitting at `phase` and render its status line.
    fn render_at_phase(station: &str, phase: StationPhase) -> String {
        use darkrun_core::domain::{Run, RunFrontmatter, Station, Status};
        use darkrun_core::state::RunState;
        use darkrun_core::StateStore;
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        store
            .write_run(&Run {
                slug: "r".into(),
                title: "T".into(),
                body: String::new(),
                frontmatter: RunFrontmatter {
                    factory: "software".into(),
                    active_station: station.into(),
                    ..Default::default()
                },
            })
            .unwrap();
        store.set_active_run("r").unwrap();
        let mut state = RunState {
            factory: "software".into(),
            active_station: station.into(),
            ..Default::default()
        };
        state.stations.insert(
            station.into(),
            Station {
                station: station.into(),
                status: Status::InProgress,
                phase,
                elaborated: true,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: None,
                completed_at: None,
            },
        );
        store.write_state("r", &state).unwrap();
        render(Some(dir.path().to_path_buf())).expect("renders an active run")
    }

    #[test]
    fn user_gate_renders_as_a_held_review_not_a_gate_label() {
        // The pre-execution USER gate reads as `review` (the universal taxonomy
        // the desktop uses), with the `⊘` hold mark carrying the "gated" meaning —
        // never a `gate` phase label, which no other surface shows.
        let line = render_at_phase("frame", StationPhase::UserGate);
        assert!(line.contains("review"), "user gate folds into review: {line}");
        assert!(!line.contains("gate"), "no `gate` phase label: {line}");
        assert!(line.contains('Π'), "the hold is shown by the doorway mark: {line}");
        // A held gate with nothing in flight shows no pool line.
        assert!(!line.contains(ITEM_LEADER), "no pool line at an idle gate: {line}");
    }

    #[test]
    fn manufacture_pool_renders_unit_beat_chips_on_a_second_line() {
        use darkrun_core::domain::{IterationResult, Status, Unit, UnitFrontmatter, UnitIteration};
        use darkrun_core::StateStore;
        // Build a Manufacture station with one in-flight unit whose first
        // worker advanced — the pool line carries its chip: filled pip(s) +
        // empties, on the second line behind the ↳ leader.
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        seed_run_at_phase(&store, "build", StationPhase::Manufacture);
        store
            .write_unit("r", &Unit {
                slug: "u1".into(),
                frontmatter: UnitFrontmatter {
                    status: Status::InProgress,
                    station: Some("build".into()),
                    started_at: Some("2026-06-01T00:00:00Z".into()),
                    iterations: vec![UnitIteration {
                        worker: "test_author".into(),
                        started_at: None,
                        completed_at: None,
                        result: Some(IterationResult::Advance),
                        note: None,
                    }],
                    ..Default::default()
                },
                title: "u1".into(),
                body: String::new(),
            })
            .unwrap();
        let line = render(Some(dir.path().to_path_buf())).expect("renders");
        let (first, second) = line.split_once('\n').expect("two lines");
        assert!(first.contains("manufacture"), "line 1 keeps position: {first}");
        assert!(second.contains(ITEM_LEADER), "pool leader: {second}");
        assert!(second.contains("u1"), "the unit chip labels its slug: {second}");
        assert!(second.contains(PIP_DONE), "a filled pip for the advanced beat: {second}");
        assert!(second.contains(PIP_PENDING), "empty pips for unreached beats: {second}");
    }

    #[test]
    fn open_feedback_preempts_the_pool_with_severity_chips() {
        use darkrun_core::StateStore;
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        seed_run_at_phase(&store, "build", StationPhase::Manufacture);
        store
            .write_feedback_raw(
                "r",
                "fb-01",
                "---\nstatus: pending\nseverity: blocker\n---\nbroken\n",
            )
            .unwrap();
        let line = render(Some(dir.path().to_path_buf())).expect("renders");
        let (_, second) = line.split_once('\n').expect("two lines");
        assert!(second.contains("fb-01"), "the feedback chip labels its id: {second}");
        assert!(second.contains('!'), "blocker mark survives NO_COLOR strips: {second}");
        assert!(second.contains("48;5;210"), "the blocker box tint: {second}");
    }

    #[test]
    fn review_await_renders_agent_status_chips() {
        use darkrun_core::domain::{Stamp, Status, Unit, UnitFrontmatter};
        use darkrun_core::StateStore;
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        seed_run_at_phase(&store, "build", StationPhase::Review);
        // One unit with the FIRST build reviewer stamped → its chip reads done
        // (✓); the next reviewer is the awaited one (▸).
        let mut reviews = std::collections::BTreeMap::new();
        reviews.insert(
            "correctness".to_string(),
            Some(Stamp { at: "2026-06-01T00:00:00Z".into() }),
        );
        store
            .write_unit("r", &Unit {
                slug: "u1".into(),
                frontmatter: UnitFrontmatter {
                    status: Status::InProgress,
                    station: Some("build".into()),
                    reviews,
                    ..Default::default()
                },
                title: "u1".into(),
                body: String::new(),
            })
            .unwrap();
        let line = render(Some(dir.path().to_path_buf())).expect("renders");
        let (_, second) = line.split_once('\n').expect("two lines");
        assert!(second.contains('\u{2713}'), "a stamped role shows ✓: {second}");
        assert!(second.contains('\u{25b8}'), "the awaited role shows ▸: {second}");
    }

    /// Seed a run whose active station sits at `phase` (no units/feedback).
    fn seed_run_at_phase(store: &darkrun_core::StateStore, station: &str, phase: StationPhase) {
        seed_slugged_run_at_phase(store, "r", station, phase);
    }

    fn seed_slugged_run_at_phase(
        store: &darkrun_core::StateStore,
        slug: &str,
        station: &str,
        phase: StationPhase,
    ) {
        use darkrun_core::domain::{Run, RunFrontmatter, Station, Status};
        use darkrun_core::state::RunState;
        store
            .write_run(&Run {
                slug: slug.into(),
                title: "T".into(),
                body: String::new(),
                frontmatter: RunFrontmatter {
                    factory: "software".into(),
                    active_station: station.into(),
                    ..Default::default()
                },
            })
            .unwrap();
        store.set_active_run(slug).unwrap();
        let mut state = RunState {
            factory: "software".into(),
            active_station: station.into(),
            ..Default::default()
        };
        state.stations.insert(station.into(), Station {
            station: station.into(),
            status: Status::InProgress,
            phase,
            elaborated: true,
            checkpoint: None,
            branch: None,
            pr_ref: None,
            pr_status: None,
            pr_ready_at: None,
            pr_merged_at: None,
            verifier_nonce: None,
            started_at: None,
            completed_at: None,
        });
        store.write_state(slug, &state).unwrap();
    }

    /// GENERATOR (run on demand, not in CI): renders the real statusline for
    /// three demo scenarios and writes them as HTML fragments the website
    /// embeds (`web/site/content/statusline-demo.html`). Regenerate with:
    ///
    /// ```sh
    /// cargo test -p darkrun-cli --bin darkrun gen_statusline_demo_html -- --ignored
    /// ```
    #[test]
    #[ignore = "writes website content; run explicitly to regenerate"]
    fn gen_statusline_demo_html() {
        use darkrun_core::domain::{IterationResult, Status, Unit, UnitFrontmatter, UnitIteration};
        use darkrun_core::StateStore;

        let mk_unit = |slug: &str, iters: Vec<(&str, Option<IterationResult>)>| Unit {
            slug: slug.into(),
            frontmatter: UnitFrontmatter {
                status: Status::InProgress,
                station: Some("build".into()),
                started_at: Some("2026-06-10T16:00:00Z".into()),
                iterations: iters
                    .into_iter()
                    .map(|(w, r)| UnitIteration {
                        worker: w.into(),
                        started_at: None,
                        completed_at: None,
                        result: r,
                        note: None,
                    })
                    .collect(),
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        };

        // The slug the website visitor sees — a believable run, not a fixture.
        const DEMO_SLUG: &str = "checkout-flow";

        // 1: Manufacture pool — two units mid-pass (one bounced).
        let d1 = tempfile::tempdir().unwrap();
        let s1 = StateStore::new(d1.path());
        seed_slugged_run_at_phase(&s1, DEMO_SLUG, "build", StationPhase::Manufacture);
        s1.write_unit(DEMO_SLUG, &mk_unit("u-03", vec![
            ("test_author", Some(IterationResult::Advance)),
            ("builder", Some(IterationResult::Advance)),
        ])).unwrap();
        s1.write_unit(DEMO_SLUG, &mk_unit("u-07", vec![
            ("test_author", Some(IterationResult::Advance)),
            ("builder", Some(IterationResult::Reject)),
        ])).unwrap();
        let line1 = render(Some(d1.path().to_path_buf())).unwrap();

        // 2: Open feedback preempts the pool — severity chips.
        let d2 = tempfile::tempdir().unwrap();
        let s2 = StateStore::new(d2.path());
        seed_slugged_run_at_phase(&s2, DEMO_SLUG, "build", StationPhase::Manufacture);
        s2.write_feedback_raw(DEMO_SLUG, "fb-01",
            "---\nstatus: pending\nseverity: blocker\n---\nbroken\n").unwrap();
        s2.write_feedback_raw(DEMO_SLUG, "fb-02",
            "---\nstatus: pending\nseverity: medium\n---\nnit\n").unwrap();
        let line2 = render(Some(d2.path().to_path_buf())).unwrap();

        // 3: Parked at the operator gate — the Π doorway.
        let d3 = tempfile::tempdir().unwrap();
        let s3 = StateStore::new(d3.path());
        seed_slugged_run_at_phase(&s3, DEMO_SLUG, "build", StationPhase::Checkpoint);
        {
            use darkrun_core::domain::{Checkpoint, CheckpointKind};
            let mut state = s3.read_state(DEMO_SLUG).unwrap().unwrap();
            let st = state.stations.get_mut("build").unwrap();
            st.checkpoint = Some(Checkpoint {
                kind: CheckpointKind::Ask,
                entered_at: Some("t".into()),
                outcome: None,
            });
            s3.write_state(DEMO_SLUG, &state).unwrap();
        }
        let line3 = render(Some(d3.path().to_path_buf())).unwrap();

        // The `-light` variants deepen the hues that wash out on a white
        // terminal — the cyan accent (81) and the review blue (75) — the same
        // remap a light-terminal palette applies. Everything else (chips,
        // severity tints, default-fg slug/pips) already reads on white.
        let light = |html: &str| -> String {
            html.replace("#5fd7ff", "#0087af").replace("#5fafff", "#0a6fc2")
        };
        let (h1, h2, h3) = (
            ansi_to_html(&line1),
            ansi_to_html(&line2),
            ansi_to_html(&line3),
        );
        let out = format!(
            "<!-- GENERATED by gen_statusline_demo_html — do not hand-edit. -->\n\
             <!--scenario:manufacture-->\n{h1}\n\
             <!--scenario:feedback-->\n{h2}\n\
             <!--scenario:gated-->\n{h3}\n\
             <!--scenario:manufacture-light-->\n{l1}\n\
             <!--scenario:feedback-light-->\n{l2}\n\
             <!--scenario:gated-light-->\n{l3}\n",
            l1 = light(&h1),
            l2 = light(&h2),
            l3 = light(&h3),
        );
        let dest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/site/content/statusline-demo.html");
        std::fs::write(&dest, out).unwrap();
        eprintln!("wrote {}", dest.display());
    }

    /// Minimal ANSI -> HTML for the generator: SGR 0/1/9, 256-color fg/bg,
    /// truecolor fg; OSC-8 hyperlinks stripped.
    fn ansi_to_html(text: &str) -> String {
        fn pal(i: u8) -> String {
            const BASE: [&str; 16] = [
                "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc",
                "#11a8cd", "#e5e5e5", "#666666", "#f14c4c", "#23d18b", "#f5f543",
                "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
            ];
            match i {
                0..=15 => BASE[i as usize].to_string(),
                16..=231 => {
                    let n = i - 16;
                    let f = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                    format!("#{:02x}{:02x}{:02x}", f(n / 36), f((n / 6) % 6), f(n % 6))
                }
                _ => {
                    let v = 8 + (i - 232) * 10;
                    format!("#{v:02x}{v:02x}{v:02x}")
                }
            }
        }
        // Strip OSC-8 link sequences.
        let mut s = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("\u{1b}]8;;") {
            s.push_str(&rest[..at]);
            let tail = &rest[at + 5..];
            let end = tail.find("\u{1b}\\").map(|e| e + 2).unwrap_or(0);
            rest = &tail[end..];
        }
        s.push_str(rest);

        let mut out = String::new();
        let (mut fg, mut bg): (Option<String>, Option<String>) = (None, None);
        let (mut bold, mut strike) = (false, false);
        let mut chars = s.chars().peekable();
        let mut buf = String::new();
        let flush = |out: &mut String, buf: &mut String, fg: &Option<String>,
                     bg: &Option<String>, bold: bool, strike: bool| {
            if buf.is_empty() {
                return;
            }
            let esc: String = buf
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let mut style = String::new();
            if let Some(c) = fg { style.push_str(&format!("color:{c};")); }
            if let Some(c) = bg { style.push_str(&format!("background:{c};")); }
            if bold { style.push_str("font-weight:700;"); }
            if strike { style.push_str("text-decoration:line-through;"); }
            if style.is_empty() {
                out.push_str(&esc);
            } else {
                out.push_str(&format!("<span style=\"{style}\">{esc}</span>"));
            }
            buf.clear();
        };
        while let Some(c) = chars.next() {
            if c == '\u{1b}' && chars.peek() == Some(&'[') {
                flush(&mut out, &mut buf, &fg, &bg, bold, strike);
                chars.next();
                let mut code = String::new();
                for d in chars.by_ref() {
                    if d == 'm' { break; }
                    code.push(d);
                }
                let nums: Vec<u8> = code.split(';').filter_map(|n| n.parse().ok()).collect();
                let mut i = 0;
                if nums.is_empty() { fg = None; bg = None; bold = false; strike = false; }
                while i < nums.len() {
                    match nums[i] {
                        0 => { fg = None; bg = None; bold = false; strike = false; }
                        1 => bold = true,
                        9 => strike = true,
                        38 if nums.get(i + 1) == Some(&5) => {
                            fg = Some(pal(nums[i + 2])); i += 2;
                        }
                        38 if nums.get(i + 1) == Some(&2) => {
                            fg = Some(format!("#{:02x}{:02x}{:02x}", nums[i + 2], nums[i + 3], nums[i + 4]));
                            i += 4;
                        }
                        48 if nums.get(i + 1) == Some(&5) => {
                            bg = Some(pal(nums[i + 2])); i += 2;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            } else {
                buf.push(c);
            }
        }
        flush(&mut out, &mut buf, &fg, &bg, bold, strike);
        out.replace('\n', "<br/>")
    }


    #[test]
    fn phase_track_pips_ride_the_station_name() {
        use darkrun_core::StateStore;
        // Manufacture (index 2): two passed + the manufacture-hued pip + three empty.
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        seed_run_at_phase(&store, "build", StationPhase::Manufacture);
        let line = render(Some(dir.path().to_path_buf())).expect("renders");
        let first = line.split('\n').next().unwrap();
        assert_eq!(first.matches(PIP_DONE).count(), 3, "3 filled track pips: {first}");
        assert_eq!(first.matches(PIP_PENDING).count(), 3, "3 empty track pips: {first}");
        // Passed pips paint in bold DEFAULT-FG (`ESC[1m▰`) — never a fixed
        // bright-white, which would vanish on a light terminal.
        assert!(
            first.contains(&format!("\x1b[{C_TRACK_DONE}m{PIP_DONE}")),
            "passed pips are bold default-fg: {first}"
        );
        assert!(
            !first.contains("38;5;255"),
            "no fixed bright-white on line one (invisible on light terminals): {first}"
        );

        // Gated checkpoint (index 5): five passed + the magenta gate pip, none empty.
        let d2 = tempfile::tempdir().unwrap();
        let s2 = StateStore::new(d2.path());
        seed_run_at_phase(&s2, "build", StationPhase::Checkpoint);
        {
            use darkrun_core::domain::{Checkpoint, CheckpointKind};
            let mut state = s2.read_state("r").unwrap().unwrap();
            s2.write_state("r", &{
                let st = state.stations.get_mut("build").unwrap();
                st.checkpoint = Some(Checkpoint { kind: CheckpointKind::Ask, entered_at: Some("t".into()), outcome: None });
                state.clone()
            }).unwrap();
        }
        let line2 = render(Some(d2.path().to_path_buf())).expect("renders");
        let first2 = line2.split('\n').next().unwrap();
        assert_eq!(first2.matches(PIP_DONE).count(), 6, "all six filled at the gate: {first2}");
        assert_eq!(first2.matches(PIP_PENDING).count(), 0, "{first2}");
    }

}
