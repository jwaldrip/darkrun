//! The HOME surface — projects, then runs, then the live Review.
//!
//! Per mockup B/C, the desktop opens on a **projects** layer, not the bare run
//! browser. A project groups runs and maps to a working dir / `.darkrun`. The
//! home:
//!
//!   1. **Projects** — lists one card per project by merging the durable
//!      project registry (`~/.darkrun/<slug>/project.json`, every registered
//!      project) with the live engines discovered via
//!      [`wire::discover_live_engines`] (which flip a card to live and supply its
//!      port). A header switcher jumps between projects; an **add-a-project**
//!      card offers two entry points — clone a Git URL (cloned into
//!      `~/darkrun/<name>` then registered) or point at a local git repo
//!      (validated then registered).
//!   2. **Runs** — drilling into a project binds that project's live engine (its
//!      discovered port) and renders the existing run browser ([`RunBrowser`]).
//!      With **no** live engine, the project shows the no-engine ONE-STEP start
//!      command per harness ([`NoEngine`]) instead of a dead list.
//!   3. **Review** — opening a run swaps in the live [`crate::review::ReviewApp`]
//!      pointed at that session, with a back affordance.
//!
//! The screen degrades gracefully: while a list is loading it shows a spinner
//! line; an empty (but reachable) engine shows a friendly "no runs yet" note;
//! a project with no engine shows the per-harness start command and links the
//! instant an engine appears (discovery is already polled, see below).

use std::path::PathBuf;

use darkrun_harness::Harness;
use darkrun_ui::prelude::*;

use crate::map;
use crate::review::ReviewApp;
use crate::wire::{self, ConnConfig, DiscoveredEngine};

/// A project the home can drill into: a working dir keyed by its `~/.darkrun`
/// slug, optionally backed by a live engine (its discovered port).
///
/// Sourced two ways and merged by [`load_projects`]: the durable project
/// registry (`~/.darkrun/<slug>/project.json`, every registered project — live
/// or idle) overlaid with [`wire::discover_live_engines`] (the running engines,
/// which contribute the `port`/`harness` and flip a card to "live"). A live
/// engine with no registry record still surfaces as a card.
#[derive(Clone, PartialEq)]
struct Project {
    /// Display name — the registry slug (the working-tree dir name carries the
    /// hash suffix; the slug is the closest stable human label we have).
    name: String,
    /// Absolute repo root the project lives in.
    path: PathBuf,
    /// The live engine's loopback port, when one is serving this project.
    port: Option<u16>,
    /// The harness the live engine adapted to, for display.
    harness: Option<String>,
}

impl Project {
    /// Project a discovered live engine into a [`Project`] card.
    fn from_engine(e: &DiscoveredEngine) -> Self {
        Project {
            name: e.slug.clone(),
            path: e.project_path.clone(),
            port: Some(e.port),
            harness: Some(e.harness.clone()),
        }
    }

    /// Whether a live engine is serving this project right now.
    fn is_live(&self) -> bool {
        self.port.is_some()
    }
}

/// Where the home is currently pointed.
#[derive(Clone, PartialEq)]
enum Nav {
    /// The projects grid (the opening surface).
    Projects,
    /// Drilled into a project — its run browser (or the no-engine state).
    Runs(Project),
}

/// The home surface: projects → runs → a run's live Review.
///
/// `cfg` is the env-derived fallback connection (port from `DARKRUN_PORT`).
/// `project_path`, when given, pre-selects a project on launch: discovery binds
/// that project's live engine if present. With no `project_path` the home opens
/// on the projects grid and the user drills in.
#[component]
pub fn HomeApp(cfg: ConnConfig, project_path: Option<PathBuf>) -> Element {
    // Every live engine discovered under `~/.darkrun`, refreshed on launch. The
    // projects grid renders from this and the runs view binds the selected
    // project's port out of it.
    let mut engines = use_signal(Vec::<DiscoveredEngine>::new);

    // The active surface. A pinned `project_path` deep-links straight into that
    // project's runs once discovery resolves; otherwise we open on Projects.
    let mut nav = use_signal(|| Nav::Projects);

    // The session id of the run currently opened into Review, if any.
    let mut opened = use_signal(|| None::<String>);

    // Bumped after a clone/register so the projects grid re-reads the on-disk
    // registry immediately, rather than waiting for the next discovery tick.
    let refresh = use_signal(|| 0u32);

    // Discover live engines once on launch, then (if a project was pinned) drill
    // straight into it. Discovery failure leaves an empty grid — the projects
    // surface still renders the add-a-project entry points.
    {
        let pinned = project_path.clone();
        use_future(move || {
            let pinned = pinned.clone();
            async move {
                let found = wire::discover_live_engines().await.unwrap_or_default();
                if let Some(path) = &pinned {
                    if let Some(e) = found.iter().find(|e| &e.project_path == path) {
                        nav.set(Nav::Runs(Project::from_engine(e)));
                    }
                }
                engines.set(found);
            }
        });
    }

    // Watch the `current` focus channel against the *selected project's* engine:
    // when the agent calls `darkrun_show`, the engine raises a run under
    // `current` and the home navigates to it. We poll only while a live project
    // is open (there's a port to poll), and navigate only on a focus *change* so
    // a back-out isn't immediately undone by the next tick. The poller is also
    // the auto-connect path: a no-engine project that boots an engine appears in
    // the next discovery refresh, and focus then flows here.
    let mut last_focus = use_signal(|| None::<String>);
    {
        let base = cfg.clone();
        use_future(move || {
            let base = base.clone();
            async move {
                loop {
                    // Re-discover so a freshly-booted engine is picked up without a
                    // relaunch — this is the desktop's auto-connect on engine appear.
                    if let Ok(found) = wire::discover_live_engines().await {
                        if found != *engines.peek() {
                            engines.set(found);
                        }
                    }
                    // Probe the active engine for `current` focus — the open
                    // project's engine when drilled in, else the launch engine
                    // (base cfg port). This is what lets `darkrun show` jump an
                    // already-open app straight to the run from the projects view,
                    // not only from inside a project's run list.
                    let probe_port = match &*nav.peek() {
                        Nav::Runs(proj) => proj.port,
                        Nav::Projects => Some(base.port),
                    };
                    if let Some(port) = probe_port {
                        let mut probe = base.clone();
                        probe.port = port;
                        let focus = wire::fetch_current_focus(&probe).await;
                        if *last_focus.peek() != focus {
                            last_focus.set(focus.clone());
                            if let Some(slug) = focus {
                                opened.set(Some(slug));
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                }
            }
        });
    }

    let shell = "padding:24px;display:flex;flex-direction:column;gap:16px;\
                 max-width:920px;margin:0 auto;";
    // Translucent surface so content blurs *through* the sticky header.
    let header_style = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:12px;\
         position:sticky;top:0;z-index:10;padding:12px 0;\
         backdrop-filter:blur(8px);background:{base}ee;\
         border-bottom:1px solid {border};",
        base = tokens::SURFACE_BASE,
        border = tokens::BORDER,
    );

    // When a run is opened, render its live Review pointed at that session on the
    // selected project's engine port. Back returns to the project's run list.
    if let Some(session) = opened.read().clone() {
        let port = match &*nav.read() {
            Nav::Runs(p) => p.port,
            Nav::Projects => None,
        };
        let mut run_cfg = cfg.with_session(session);
        if let Some(port) = port {
            run_cfg.port = port;
        }
        let mut opened = opened;
        return rsx! {
            div { style: "{shell}",
                div { style: "display:flex;",
                    Button {
                        variant: ButtonVariant::Ghost,
                        tone: Tone::Neutral,
                        on_click: move |_| opened.set(None),
                        "\u{2190} all runs"
                    }
                }
            }
            ReviewApp { cfg: run_cfg }
        };
    }

    // The header label/badge tracks the active surface.
    let (badge_text, badge_tone) = match &*nav.read() {
        Nav::Projects => ("projects".to_string(), Tone::Neutral),
        Nav::Runs(p) if p.is_live() => ("live".to_string(), Tone::Ok),
        Nav::Runs(_) => ("no engine".to_string(), Tone::Warn),
    };

    // Build the switcher options (every registered + live project) once per
    // render. Reading `refresh` here ties a re-render to a clone/register so the
    // freshly-written `project.json` shows without waiting for the poller.
    let _ = refresh.read();
    let projects = load_projects(&engines.read());

    rsx! {
        div { style: "{shell}",
            header {
                style: "{header_style}",
                Wordmark { variant: WordmarkVariant::OutlinedSolidRun, size: 28.0 }
                div { style: "display:flex;align-items:center;gap:10px;",
                    ProjectSwitcher { projects: projects.clone(), nav }
                    Badge { tone: badge_tone, filled: badge_tone != Tone::Neutral, "{badge_text}" }
                }
            }
            match nav.read().clone() {
                Nav::Projects => rsx! {
                    ProjectsGrid { projects: projects.clone(), nav, refresh }
                },
                Nav::Runs(proj) => rsx! {
                    RunsSurface { cfg: cfg.clone(), proj, nav, opened }
                },
            }
        }
    }
}

/// Merge the on-disk project registry with the live engines into the project
/// cards the home renders.
///
/// Two sources, keyed by registry slug:
///   1. **Registered projects** — every `~/.darkrun/<slug>/project.json` (read
///      via [`registry::list_projects`]), live or idle. These carry the durable
///      name/path even when no engine is running, so registered-but-idle
///      projects show as cards (with `port: None`, which the no-engine card
///      already handles).
///   2. **Live engines** — every running engine ([`DiscoveredEngine`]). When an
///      engine's slug matches a registered project, its port + harness are
///      OVERLAID onto that project (flipping it to "live"). A live engine with
///      NO registered record (an engine booted in a never-registered repo) is
///      still surfaced so it isn't lost — discovery has always shown these.
///
/// Idempotent on slug: a project that is both registered and live yields ONE
/// card, not two. Returns cards sorted by name for a stable grid order.
fn load_projects(engines: &[DiscoveredEngine]) -> Vec<Project> {
    use std::collections::BTreeMap;

    // Start from the durable registry: every registered project as an idle card.
    let mut by_slug: BTreeMap<String, Project> = darkrun_mcp::registry::list_projects()
        .unwrap_or_default()
        .into_iter()
        .map(|rec| {
            let project = Project {
                name: rec.name.clone().unwrap_or_else(|| rec.slug.clone()),
                path: rec.path,
                port: None,
                harness: None,
            };
            (rec.slug, project)
        })
        .collect();

    // Overlay live engines: flip a matching registered project to live, or add a
    // standalone card for an engine with no registry record.
    for e in engines {
        by_slug
            .entry(e.slug.clone())
            .and_modify(|p| {
                p.port = Some(e.port);
                p.harness = Some(e.harness.clone());
            })
            .or_insert_with(|| Project::from_engine(e));
    }

    by_slug.into_values().collect()
}

/// The header project switcher — a native `select` over every discovered
/// project. Choosing one drills into its runs; the "all projects" option returns
/// to the grid. Hidden until at least one project exists.
#[component]
fn ProjectSwitcher(projects: Vec<Project>, nav: Signal<Nav>) -> Element {
    if projects.is_empty() {
        return rsx! {};
    }

    // The currently-selected project name, if we're in a runs view.
    let current = match &*nav.read() {
        Nav::Runs(p) => Some(p.name.clone()),
        Nav::Projects => None,
    };

    let style = format!(
        "appearance:none;background:transparent;border:1px solid {border};\
         border-radius:7px;padding:5px 11px;color:{muted};\
         font-family:{mono};font-size:12px;cursor:pointer;",
        border = tokens::BORDER_STRONG,
        muted = tokens::TEXT_MUTED,
        mono = tokens::FONT_MONO,
    );

    let options = projects.clone();
    let mut nav = nav;
    let pick = move |evt: FormEvent| {
        let value = evt.value();
        if value == "__projects__" {
            nav.set(Nav::Projects);
            return;
        }
        if let Some(p) = options.iter().find(|p| p.name == value) {
            nav.set(Nav::Runs(p.clone()));
        }
    };

    rsx! {
        select {
            style: "{style}",
            onchange: pick,
            option {
                value: "__projects__",
                selected: current.is_none(),
                "all projects"
            }
            for p in projects.iter() {
                option {
                    value: "{p.name}",
                    selected: current.as_deref() == Some(p.name.as_str()),
                    "project: {p.name}"
                }
            }
        }
    }
}

/// The projects grid (mockup B): one card per project + an add-a-project card,
/// followed by the add-a-project form.
#[component]
fn ProjectsGrid(projects: Vec<Project>, nav: Signal<Nav>, refresh: Signal<u32>) -> Element {
    let heading = format!(
        "margin:0;font-family:{sans};font-size:13px;font-weight:700;\
         text-transform:uppercase;letter-spacing:0.04em;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let grid = "display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
                gap:12px;";

    rsx! {
        h2 { style: "{heading}", "Projects" }
        div { style: "{grid}",
            for proj in projects.iter() {
                ProjectCard { proj: proj.clone(), nav }
            }
            AddProjectCard {}
        }
        AddProjectForm { refresh }
    }
}

/// One project card: name + status badge, path, and a run-count line. The whole
/// card drills into the project's runs on click.
#[component]
fn ProjectCard(proj: Project, nav: Signal<Nav>) -> Element {
    let (status_text, status_tone) = if proj.is_live() {
        ("live", Tone::Ok)
    } else {
        ("idle", Tone::Neutral)
    };

    let header = "display:flex;align-items:center;justify-content:space-between;\
                  gap:10px;margin-bottom:8px;";
    let name_style = format!(
        "font-family:{sans};font-size:15px;font-weight:700;color:{text};\
         overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0;",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let path_style = format!(
        "font-family:{mono};font-size:11px;color:{faint};\
         overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
        mono = tokens::FONT_MONO,
        faint = tokens::TEXT_FAINT,
    );
    let meta_style = format!(
        "font-size:12px;color:{muted};margin-top:6px;",
        muted = tokens::TEXT_MUTED,
    );

    let path_label = proj.path.display().to_string();
    let harness_label = proj.harness.clone();
    let mut nav = nav;
    let proj_for_click = proj.clone();
    rsx! {
        div {
            class: "dr-project-card",
            style: "cursor:pointer;",
            role: "button",
            tabindex: "0",
            onclick: move |_| nav.set(Nav::Runs(proj_for_click.clone())),
            Card {
                div { style: "{header}",
                    span { style: "{name_style}", title: "{proj.name}", "{proj.name}" }
                    Badge { tone: status_tone, filled: status_tone != Tone::Neutral, "{status_text}" }
                }
                div { style: "{path_style}", title: "{path_label}", "{path_label}" }
                div { style: "{meta_style}",
                    match &harness_label {
                        Some(h) => rsx! { "engine: {h}" },
                        None => rsx! { "no live engine" },
                    }
                }
            }
        }
    }
}

/// The dashed "add a project" card — a visual affordance that points at the
/// add-a-project form below (the form is always rendered, so this is a label).
#[component]
fn AddProjectCard() -> Element {
    let style = format!(
        "display:flex;align-items:center;justify-content:center;\
         border:1px dashed {border};border-radius:10px;padding:14px 16px;\
         color:{faint};font-family:{sans};font-size:13px;min-height:96px;",
        border = tokens::BORDER_STRONG,
        faint = tokens::TEXT_FAINT,
        sans = tokens::FONT_SANS,
    );
    rsx! {
        div { style: "{style}", "\u{ff0b} add a project below" }
    }
}

/// The two add-a-project entry points (mockup B): a **Git URL** (clone +
/// register) and a **Local repo** (pick an existing git checkout). Both must be
/// git repos; the slug + `.darkrun/` live in the working tree.
///
/// Both paths execute for real (see [`add_project`]): a Git URL clones into
/// `~/darkrun/<name>` then writes a [`ProjectRecord`]; a local path validates
/// the repo and writes the record. On success `refresh` is bumped so the grid
/// re-reads the registry and the new card appears immediately.
#[component]
fn AddProjectForm(refresh: Signal<u32>) -> Element {
    // Which entry point is selected (`git` URL vs `local` path).
    let mut mode = use_signal(|| AddMode::Git);
    // The current input value (a URL or a path, depending on `mode`).
    let mut value = use_signal(String::new);
    // A status line after a submit attempt (working note, success, or error).
    let mut status = use_signal(|| None::<String>);
    // True while a clone/register is in flight — disables re-submit and shows a
    // working note (a clone can take a while over the network).
    let mut busy = use_signal(|| false);

    let heading = format!(
        "margin:0;font-family:{sans};font-size:13px;font-weight:700;\
         text-transform:uppercase;letter-spacing:0.04em;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let input_style = format!(
        "flex:1;box-sizing:border-box;padding:9px 12px;border-radius:6px;\
         border:1px solid {border};background:{base};color:{text};\
         font-family:{sans};font-size:13px;",
        border = tokens::BORDER,
        base = tokens::SURFACE_BASE,
        text = tokens::TEXT,
        sans = tokens::FONT_SANS,
    );
    let note_style = format!(
        "font-size:11.5px;color:{faint};margin:8px 0 0;line-height:1.5;",
        faint = tokens::TEXT_FAINT,
    );

    let current = *mode.read();
    let placeholder = match current {
        AddMode::Git => "https://github.com/acme/storefront.git",
        AddMode::Local => "/Users/you/dev/acme/storefront",
    };
    let action_label = match current {
        AddMode::Git => "Clone & add",
        AddMode::Local => "Add repo",
    };

    // Submit clones/registers off the UI thread (a clone is blocking + network-
    // bound) via `spawn_blocking`, then bumps `refresh` so the grid re-reads the
    // registry. `busy` guards against a double-submit while in flight.
    let mut refresh = refresh;
    let on_submit = move |_| {
        if *busy.peek() {
            return;
        }
        let raw = value.read().trim().to_string();
        // Validate synchronously for instant feedback before doing any work.
        if let Err(err) = validate_add_input(current, &raw) {
            status.set(Some(err));
            return;
        }
        busy.set(true);
        status.set(Some(match current {
            AddMode::Git => "Cloning\u{2026}".to_string(),
            AddMode::Local => "Registering\u{2026}".to_string(),
        }));
        spawn(async move {
            let result = tokio::task::spawn_blocking(move || add_project(current, &raw))
                .await
                .unwrap_or_else(|e| Err(format!("internal error: {e}")));
            match result {
                Ok(note) => {
                    status.set(Some(note));
                    // A new `project.json` is on disk — nudge the grid to re-read.
                    let n = *refresh.peek();
                    refresh.set(n.wrapping_add(1));
                    value.set(String::new());
                }
                Err(err) => status.set(Some(err)),
            }
            busy.set(false);
        });
    };
    let busy_now = *busy.read();

    rsx! {
        Card {
            h2 { style: "{heading}", "Add a project" }
            // Entry-point tabs.
            div { style: "display:flex;gap:8px;margin-top:10px;",
                ModeTab { label: "Git URL", active: current == AddMode::Git,
                    on_pick: move |_| { mode.set(AddMode::Git); status.set(None); } }
                ModeTab { label: "Local repo", active: current == AddMode::Local,
                    on_pick: move |_| { mode.set(AddMode::Local); status.set(None); } }
            }
            // Input + action.
            div { style: "display:flex;gap:8px;margin-top:10px;align-items:center;",
                input {
                    style: "{input_style}",
                    placeholder: "{placeholder}",
                    value: "{value}",
                    oninput: move |evt| value.set(evt.value()),
                }
                Button {
                    variant: ButtonVariant::Primary,
                    tone: Tone::Accent,
                    disabled: busy_now,
                    on_click: on_submit,
                    if busy_now { "Working\u{2026}" } else { "{action_label}" }
                }
            }
            p { style: "{note_style}",
                "Git URL \u{2192} darkrun clones it, then registers it. Local repo \u{2192} pick a path \
                 to an existing git checkout. Either way it must be a git repo; the project slug + \
                 .darkrun/ live in the working tree."
            }
            if let Some(msg) = status.read().clone() {
                p {
                    style: format!(
                        "font-family:{mono};font-size:11.5px;color:{accent};margin:8px 0 0;",
                        mono = tokens::FONT_MONO,
                        accent = tokens::ACCENT,
                    ),
                    "{msg}"
                }
            }
        }
    }
}

/// Which add-a-project entry point is active.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AddMode {
    /// Clone from a Git URL, then register.
    Git,
    /// Register an existing local git checkout by path.
    Local,
}

/// Cheap, synchronous validation of an add-a-project input — run on the UI
/// thread before any blocking work so the operator gets instant feedback on an
/// empty/obviously-wrong value. The heavy lifting (clone, repo check) happens in
/// [`add_project`].
fn validate_add_input(mode: AddMode, raw: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err(match mode {
            AddMode::Git => "Enter a git URL to clone.".to_string(),
            AddMode::Local => "Enter a path to a local git repo.".to_string(),
        });
    }
    match mode {
        AddMode::Git => {
            let looks_like_url = raw.contains("://") || raw.starts_with("git@");
            if !looks_like_url {
                return Err(
                    "That doesn't look like a git URL (expected https:// or git@).".to_string(),
                );
            }
        }
        AddMode::Local => {
            let path = PathBuf::from(raw);
            if !path.is_absolute() {
                return Err("Use an absolute path to the git checkout.".to_string());
            }
        }
    }
    Ok(())
}

/// Perform an add-a-project request for real. **Blocking** — runs off the UI
/// thread (a clone is network-bound). Returns a human-readable success note or
/// an error string suitable for the status line.
///
/// - **Git URL**: clone into `~/darkrun/<repo-name>` (the editable default
///   clone root) via [`darkrun_git::clone_repo`], then derive the slug and write
///   a [`ProjectRecord`] to `~/.darkrun/<slug>/project.json` so the project
///   enumerates whether or not an engine is later serving it.
/// - **Local repo**: validate it's a git checkout, then register it (write the
///   record) without cloning.
///
/// Registration is idempotent — re-adding an already-registered repo just
/// rewrites its record.
fn add_project(mode: AddMode, raw: &str) -> Result<String, String> {
    validate_add_input(mode, raw)?;
    match mode {
        AddMode::Git => {
            // Default clone root is ~/darkrun; fall back to the cwd if home can't
            // be resolved (degraded, but never silently writes somewhere odd).
            let base = dirs::home_dir()
                .map(|h| h.join("darkrun"))
                .unwrap_or_else(|| PathBuf::from("darkrun"));
            let dest = darkrun_git::default_clone_dest(&base, raw);
            if dest.exists() {
                return Err(format!(
                    "{} already exists — remove it or register it as a local repo.",
                    dest.display()
                ));
            }
            let cloned = darkrun_git::clone_repo(raw, &dest)
                .map_err(|e| format!("Clone failed: {e}"))?;
            let record = darkrun_mcp::registry::register_project(cloned.repo_root(), None)
                .map_err(|e| format!("Cloned, but registering failed: {e}"))?;
            Ok(format!(
                "Cloned into {} and registered as \u{201c}{}\u{201d}.",
                dest.display(),
                record.slug
            ))
        }
        AddMode::Local => {
            let path = PathBuf::from(raw);
            if !path.join(".git").exists() {
                return Err("No .git found at that path — point at a git repo.".to_string());
            }
            // Confirm it really opens as a repo (a stray `.git` file isn't one).
            darkrun_git::Git::open(&path)
                .map_err(|e| format!("Not a usable git repo: {e}"))?;
            let record = darkrun_mcp::registry::register_project(&path, None)
                .map_err(|e| format!("Register failed: {e}"))?;
            Ok(format!(
                "Registered {} as \u{201c}{}\u{201d}.",
                path.display(),
                record.slug
            ))
        }
    }
}

/// A small tab chip for the add-a-project entry-point switch.
#[component]
fn ModeTab(label: String, active: bool, on_pick: EventHandler<MouseEvent>) -> Element {
    let style = if active {
        format!(
            "padding:6px 12px;border-radius:7px;border:1px solid {accent};\
             background:{accent}14;color:{text};font-size:12.5px;cursor:pointer;",
            accent = tokens::ACCENT,
            text = tokens::TEXT,
        )
    } else {
        format!(
            "padding:6px 12px;border-radius:7px;border:1px solid {border};\
             background:transparent;color:{muted};font-size:12.5px;cursor:pointer;",
            border = tokens::BORDER_STRONG,
            muted = tokens::TEXT_MUTED,
        )
    };
    rsx! {
        span {
            style: "{style}",
            onclick: move |evt| on_pick.call(evt),
            "{label}"
        }
    }
}

/// The runs surface for a drilled-into project: a back affordance, then either
/// the run browser (live engine) or the no-engine ONE-STEP start command.
#[component]
fn RunsSurface(
    cfg: ConnConfig,
    proj: Project,
    nav: Signal<Nav>,
    opened: Signal<Option<String>>,
) -> Element {
    let mut nav = nav;
    let back = move |_| nav.set(Nav::Projects);

    let meta_style = format!(
        "font-family:{mono};font-size:12px;color:{muted};",
        mono = tokens::FONT_MONO,
        muted = tokens::TEXT_MUTED,
    );
    let path_label = proj.path.display().to_string();

    // Bind the connection to this project's live engine port when one exists.
    let mut proj_cfg = cfg.clone();
    if let Some(port) = proj.port {
        proj_cfg.port = port;
    }

    rsx! {
        div { style: "display:flex;align-items:center;justify-content:space-between;gap:12px;",
            Button {
                variant: ButtonVariant::Ghost,
                tone: Tone::Neutral,
                on_click: back,
                "\u{2190} all projects"
            }
            span { style: "{meta_style}", "{proj.name}" }
        }
        if proj.is_live() {
            RunBrowser { cfg: proj_cfg, opened }
        } else {
            NoEngine { name: proj.name.clone(), path: path_label }
        }
    }
}

/// The no-engine ONE-STEP state (mockup C): the run/project has no live engine,
/// so the desktop can't drive it. Instead of a dead screen, show the exact
/// terminal command per harness to bring an engine up — and note the desktop
/// auto-connects the instant the engine advertises itself to `~/.darkrun`.
///
/// An autonomous-mode toggle (default ON) controls whether the command splices
/// the harness's permission-bypass flag (from
/// [`Capabilities::autonomous_launch_args`]).
#[component]
fn NoEngine(name: String, path: String, #[props(default)] run: Option<String>) -> Element {
    // Per-project autonomous mode — default ON (the permission boundary moves to
    // the checkpoint, not per-tool prompts). Toggling off drops the bypass flag.
    let mut autonomous = use_signal(|| true);
    // Which harness tab is selected (index into [`Harness::ALL`], reordered to
    // lead with the CLI harnesses that carry a flag — see `harness_order`).
    let mut selected = use_signal(|| 0usize);

    let order = harness_order();
    let sel = *selected.read();
    let active_harness = order.get(sel).copied().unwrap_or(Harness::ClaudeCode);
    let auto = *autonomous.read();

    let card_style = format!(
        "background:{raised};border:1px solid {border};border-top:2px solid {warn};\
         border-radius:8px;padding:16px 18px;",
        raised = tokens::SURFACE_RAISED,
        border = tokens::BORDER,
        warn = tokens::STATUS_WARN,
    );
    let head_style = "display:flex;align-items:center;justify-content:space-between;gap:10px;";
    let title_style = format!(
        "font-family:{sans};font-size:16px;font-weight:700;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let body_style = format!(
        "font-size:13.5px;line-height:1.55;color:{muted};margin:10px 0 14px;",
        muted = tokens::TEXT_MUTED,
    );
    let path_style = format!(
        "font-family:{mono};color:{faint};",
        mono = tokens::FONT_MONO,
        faint = tokens::TEXT_FAINT,
    );

    // Autonomous-mode toggle bar.
    let toggle_bar = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:10px;\
         padding:10px 12px;border:1px solid {border};border-radius:8px;\
         background:{overlay};margin-bottom:12px;",
        border = tokens::BORDER_STRONG,
        overlay = tokens::SURFACE_OVERLAY,
    );
    let pill_style = if auto {
        format!(
            "font-family:{mono};font-size:11px;background:{ok};color:{on};\
             border-radius:999px;padding:3px 11px;font-weight:700;cursor:pointer;",
            mono = tokens::FONT_MONO,
            ok = tokens::STATUS_OK,
            on = tokens::SURFACE_BASE,
        )
    } else {
        format!(
            "font-family:{mono};font-size:11px;border:1px solid {border};color:{muted};\
             border-radius:999px;padding:3px 11px;font-weight:700;cursor:pointer;",
            mono = tokens::FONT_MONO,
            border = tokens::BORDER_STRONG,
            muted = tokens::TEXT_MUTED,
        )
    };

    let tabs_style = "display:flex;gap:6px;flex-wrap:wrap;margin-bottom:12px;";
    let cmd_style = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:12px;\
         padding:12px 14px;border-radius:8px;background:{base};\
         border:1px solid {border};font-family:{mono};font-size:13px;color:{accent};",
        base = tokens::SURFACE_BASE,
        border = tokens::BORDER,
        mono = tokens::FONT_MONO,
        accent = tokens::ACCENT,
    );
    let foot_style = format!(
        "font-size:11.5px;color:{faint};margin:12px 0 0;line-height:1.5;",
        faint = tokens::TEXT_FAINT,
    );

    let command = launch_command(active_harness, &path, auto, run.as_deref());

    rsx! {
        div { style: "{card_style}",
            div { style: "{head_style}",
                span { style: "{title_style}", "{name}" }
                Badge { tone: Tone::Warn, filled: true, "no engine connected" }
            }
            p { style: "{body_style}",
                "This project lives in "
                span { style: "{path_style}", "{path}" }
                " but no darkrun engine is serving it. You already picked the project \u{2014} just \
                 bring an engine up in your harness of choice and it connects here. "
                b { "One command" }
                ", no "
                code { style: "{path_style}", "show" }
                ":"
            }

            // Autonomous-mode toggle (per-project, default ON).
            div { style: "{toggle_bar}",
                span { style: "font-size:13px;color:var(--dr-text);",
                    b { "Autonomous mode" }
                    span { style: "color:var(--dr-text-muted);",
                        " \u{2014} agent runs without per-tool prompts; your control point is the checkpoint"
                    }
                }
                span {
                    style: "{pill_style}",
                    onclick: move |_| {
                        let now = *autonomous.peek();
                        autonomous.set(!now);
                    },
                    if auto { "ON" } else { "OFF" }
                }
            }

            // Per-harness tabs.
            div { style: "{tabs_style}",
                for (idx, h) in order.iter().enumerate() {
                    HarnessTab {
                        label: h.capabilities().display_name.to_string(),
                        active: idx == sel,
                        on_pick: move |_| selected.set(idx),
                    }
                }
            }

            // The ONE-STEP command for the selected harness.
            div { style: "{cmd_style}",
                span { "{command}" }
                span { style: "font-size:11px;color:var(--dr-text-faint);", "copy \u{29c9}" }
            }

            // Harnesses without a launch flag get a note: autonomy is a setting.
            if active_harness.capabilities().autonomous_launch_args.is_none() {
                p {
                    style: format!(
                        "font-size:11.5px;color:{faint};margin:10px 0 0;line-height:1.5;",
                        faint = tokens::TEXT_FAINT,
                    ),
                    {format!(
                        "{} has no autonomy CLI flag \u{2014} enable its auto-run / turbo setting \
                         in the editor instead.",
                        active_harness.capabilities().display_name,
                    )}
                }
            }

            p { style: "{foot_style}",
                "Launching the harness boots the darkrun MCP engine, which advertises itself to "
                span { style: "{path_style}", "~/.darkrun/" }
                ". The desktop is already on this project \u{2014} it links the instant the engine appears. \
                 No second step."
            }
        }
    }
}

/// Harness tab order for the no-engine command (mockup C): lead with the CLI
/// harnesses that take a launch flag, GUI ones after. Matches the mockup's
/// Claude Code / Codex / Gemini CLI / Cursor / Windsurf / OpenCode / Kiro order.
fn harness_order() -> Vec<Harness> {
    vec![
        Harness::ClaudeCode,
        Harness::Codex,
        Harness::GeminiCli,
        Harness::Cursor,
        Harness::Windsurf,
        Harness::Opencode,
        Harness::Kiro,
    ]
}

/// The launch binary a harness is invoked as on the command line. The registry
/// carries the autonomy *flag* but not the binary name, so it's mapped here.
/// GUI harnesses have no terminal launcher — `None` renders an open-the-app
/// instruction instead of a `cd && <bin>` line.
fn harness_binary(h: Harness) -> Option<&'static str> {
    match h {
        Harness::ClaudeCode => Some("claude"),
        Harness::Codex => Some("codex"),
        Harness::GeminiCli => Some("gemini"),
        Harness::Opencode => Some("opencode"),
        // Cursor / Windsurf / Kiro are editor-driven — open the app, no CLI.
        Harness::Cursor | Harness::Windsurf | Harness::Kiro => None,
    }
}

/// Single-quote a path for a POSIX shell so the copied `cd` survives spaces and
/// other shell-special characters in the project path.
fn sh_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Build the ONE-STEP start command for a harness in a project dir. The path is
/// shell-quoted so the copied command `cd`s into the correct dir even when the
/// path has spaces. When `autonomous` is on the harness's bypass flag (if any)
/// is spliced in. When the harness exposes a worktree flag and `run` is given,
/// `<flag> darkrun-<run>` is appended so the Run gets its own git worktree. GUI
/// harnesses (no CLI binary) get an open-the-editor instruction.
fn launch_command(h: Harness, path: &str, autonomous: bool, run: Option<&str>) -> String {
    let caps = h.capabilities();
    let dir = sh_quote(path);
    match harness_binary(h) {
        Some(bin) => {
            let mut cmd = format!("cd {dir} && {bin}");
            if autonomous {
                let flag = caps.autonomous_launch_args();
                if !flag.is_empty() {
                    cmd.push(' ');
                    cmd.push_str(flag);
                }
            }
            if let (Some(wt), Some(run)) = (caps.worktree_flag, run) {
                cmd.push_str(&format!(" {wt} darkrun-{run}"));
            }
            cmd
        }
        None => format!("open {} in {dir}, then enable its MCP plugin", caps.display_name),
    }
}

/// A per-harness tab chip in the no-engine command picker.
#[component]
fn HarnessTab(label: String, active: bool, on_pick: EventHandler<MouseEvent>) -> Element {
    let style = if active {
        format!(
            "padding:6px 12px;border-radius:7px;border:1px solid {accent};\
             background:{accent}14;color:{text};font-size:12.5px;cursor:pointer;",
            accent = tokens::ACCENT,
            text = tokens::TEXT,
        )
    } else {
        format!(
            "padding:6px 12px;border-radius:7px;border:1px solid {border};\
             background:transparent;color:{muted};font-size:12.5px;cursor:pointer;",
            border = tokens::BORDER_STRONG,
            muted = tokens::TEXT_MUTED,
        )
    };
    rsx! {
        span {
            style: "{style}",
            onclick: move |evt| on_pick.call(evt),
            "{label}"
        }
    }
}

/// The load state of the run list.
#[derive(Clone, PartialEq)]
enum Load {
    /// The `/api/runs` GET is in flight.
    Loading,
    /// The list loaded (possibly empty).
    Loaded(Vec<RunCardData>),
    /// The GET failed — the engine is likely not running. Carries the reason.
    Failed(String),
}

/// Loads `/api/runs` for the bound project engine and renders the [`RunList`]. A
/// click on each card opens the run by writing its session id into `opened`.
#[component]
fn RunBrowser(cfg: ConnConfig, opened: Signal<Option<String>>) -> Element {
    let mut state = use_signal(|| Load::Loading);

    // Fetch the run list once on mount.
    let fetch_cfg = cfg.clone();
    use_future(move || {
        let cfg = fetch_cfg.clone();
        async move {
            state.set(Load::Loading);
            match wire::fetch_runs(&cfg).await {
                Ok(payload) => {
                    let cards: Vec<RunCardData> = payload.runs.iter().map(map::run_card).collect();
                    state.set(Load::Loaded(cards));
                }
                Err(e) => state.set(Load::Failed(e.to_string())),
            }
        }
    });

    let mut opened = opened;
    let open = move |slug: String| {
        // The run slug is the session id the engine serves the live feed under.
        opened.set(Some(slug));
    };

    let current = state.read().clone();
    match current {
        Load::Loading => rsx! {
            Card {
                p { style: "color:var(--dr-text-muted);margin:0;",
                    "Loading runs from the local engine\u{2026}"
                }
            }
        },
        Load::Failed(reason) => rsx! { EngineDown { reason } },
        Load::Loaded(cards) => rsx! {
            RunList {
                runs: cards,
                on_select: open,
                empty: rsx! { NoRuns {} },
            }
        },
    }
}

/// Shown when `/api/runs` could not be reached on a project we *thought* had a
/// live engine — the engine went down between discovery and the fetch. Tells the
/// operator how to bring it back.
#[component]
fn EngineDown(reason: String) -> Element {
    rsx! {
        Card { accent: Tone::Danger.color().to_string(),
            h2 {
                style: "margin:0 0 8px;font-family:var(--dr-font-sans);\
                        font-size:15px;font-weight:700;color:var(--dr-text);",
                "Engine unreachable"
            }
            p { style: "margin:0 0 10px;color:var(--dr-text-muted);font-size:13px;",
                "Couldn't reach this project's engine to list runs. It may have just \
                 stopped \u{2014} bring it back up and this screen refills."
            }
            pre {
                style: "margin:0;padding:10px 12px;border-radius:6px;\
                        background:var(--dr-surface-raised);border:1px solid var(--dr-border);\
                        font-family:var(--dr-font-mono);font-size:13px;color:var(--dr-accent);",
                "darkrun serve"
            }
            p {
                style: "margin:10px 0 0;font-family:var(--dr-font-mono);\
                        font-size:11px;color:var(--dr-text-faint);",
                "{reason}"
            }
        }
    }
}

/// Shown when the engine is reachable but the project has no runs yet.
#[component]
fn NoRuns() -> Element {
    rsx! {
        Card {
            h2 {
                style: "margin:0 0 8px;font-family:var(--dr-font-sans);\
                        font-size:15px;font-weight:700;color:var(--dr-text);",
                "No runs yet"
            }
            p { style: "margin:0;color:var(--dr-text-muted);font-size:13px;",
                "The engine is up but hasn't started any runs. Kick one off and it'll \
                 show up here."
            }
        }
    }
}
