//! The HOME surface — projects, then runs, then the live Review.
//!
//! Per mockup B/C, the desktop opens on a **projects** layer, not the bare run
//! browser. A project groups runs and maps to a working dir / `.darkrun`. The
//! home:
//!
//!   1. **Projects** — scans `~/.darkrun` for live engines via
//!      [`wire::discover_live_engines`] and lists one card per project (name,
//!      path, run-count when known, live/idle status). A header switcher jumps
//!      between projects; an **add-a-project** card offers two entry points —
//!      clone a Git URL, or point at a local git repo.
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
/// Derived from [`wire::discover_live_engines`] today — every live engine is a
/// project. Projects that exist on disk but have no live engine (recents /
/// registered-only) are not yet enumerable from the desktop; see the TODO in
/// [`load_projects`].
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
                    if let Nav::Runs(proj) = &*nav.peek() {
                        if let Some(port) = proj.port {
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

    // Build the switcher options (every discovered project) once per render.
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
                    ProjectsGrid { projects: projects.clone(), nav }
                },
                Nav::Runs(proj) => rsx! {
                    RunsSurface { cfg: cfg.clone(), proj, nav, opened }
                },
            }
        }
    }
}

/// Project the discovered live engines into the project cards the home renders.
///
/// TODO(projects-on-disk): this only surfaces projects with a *live* engine.
/// Recents and registered-but-idle projects (a `~/.darkrun/<slug>/` that exists
/// without a running engine) aren't enumerable from the desktop yet — the
/// registry exposes only live descriptors. When a "list registered projects"
/// endpoint lands, merge it here so idle projects show with `port: None` (the
/// no-engine card already handles that path).
fn load_projects(engines: &[DiscoveredEngine]) -> Vec<Project> {
    engines.iter().map(Project::from_engine).collect()
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
fn ProjectsGrid(projects: Vec<Project>, nav: Signal<Nav>) -> Element {
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
        AddProjectForm {}
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
/// The actual clone/register is not yet wired to an engine endpoint — see the
/// TODO in [`add_project`]. The UI is fully present and validates locally.
#[component]
fn AddProjectForm() -> Element {
    // Which entry point is selected (`git` URL vs `local` path).
    let mut mode = use_signal(|| AddMode::Git);
    // The current input value (a URL or a path, depending on `mode`).
    let mut value = use_signal(String::new);
    // A status line after a submit attempt (success TODO note or validation).
    let mut status = use_signal(|| None::<String>);

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

    let on_submit = move |_| {
        let raw = value.read().trim().to_string();
        match add_project(current, &raw) {
            Ok(note) => status.set(Some(note)),
            Err(err) => status.set(Some(err)),
        }
    };

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
                    on_click: on_submit,
                    "{action_label}"
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

/// Validate an add-a-project request and (eventually) hand it to the engine.
///
/// TODO(add-project-endpoint): the engine has no "clone + register" / "register
/// local repo" HTTP endpoint yet, so this only validates the input and returns a
/// human-readable note describing what *would* happen. When the endpoint lands
/// (e.g. `POST /api/projects` with `{ "git_url" }` or `{ "path" }`), call it via
/// a new `wire::register_project(...)` and refresh discovery on success.
fn add_project(mode: AddMode, raw: &str) -> Result<String, String> {
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
                return Err("That doesn't look like a git URL (expected https:// or git@).".to_string());
            }
            Ok(format!(
                "Would clone {raw} and register it. (engine clone/register endpoint pending — TODO)"
            ))
        }
        AddMode::Local => {
            let path = PathBuf::from(raw);
            if !path.is_absolute() {
                return Err("Use an absolute path to the git checkout.".to_string());
            }
            if !path.join(".git").exists() {
                return Err("No .git found at that path — point at a git repo.".to_string());
            }
            Ok(format!(
                "Would register {raw} as a project. (engine register endpoint pending — TODO)"
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
fn NoEngine(name: String, path: String) -> Element {
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

    let command = launch_command(active_harness, &path, auto);

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

/// Build the ONE-STEP start command for a harness in a project dir. When
/// `autonomous` is on the harness's bypass flag (if any) is spliced in. GUI
/// harnesses (no CLI binary) get an open-the-editor instruction.
fn launch_command(h: Harness, path: &str, autonomous: bool) -> String {
    let caps = h.capabilities();
    match harness_binary(h) {
        Some(bin) => {
            let flag = if autonomous { caps.autonomous_launch_args() } else { "" };
            if flag.is_empty() {
                format!("cd {path} && {bin}")
            } else {
                format!("cd {path} && {bin} {flag}")
            }
        }
        None => format!("open {} in {path}, then enable its MCP plugin", caps.display_name),
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
