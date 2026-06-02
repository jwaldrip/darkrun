//! The desktop SHELL — a native toolbar + sidebar + main-pane layout.
//!
//! Per `mockups/desktop-shell.html`, the desktop is not a centered web page: it
//! is a unified TOOLBAR (the theme-aware wordmark + search/add/theme actions), a
//! full-height left SIDEBAR (projects as collapsible sections with their RUNS
//! nested under each, a Mine/All segmented filter, and a run/project search), and
//! a MAIN pane that fills the rest of the window. Selecting a run opens it in the
//! main pane; with nothing selected the pane shows the projects / add-a-project
//! surface; a no-engine project shows the per-harness one-step start command.
//!
//! This is a **restructure** of the old projects→runs→review drill-down into the
//! shell, not a rewrite of the review internals. All prior behavior is preserved:
//!
//!   - **Discovery / auto-connect** — live engines are read from `~/.darkrun` on
//!     launch and re-polled, so a freshly-booted engine appears without relaunch
//!     ([`wire::discover_live_engines`]).
//!   - **Current-focus poller** — when the agent calls `darkrun_show`, the engine
//!     raises a run under the `current` session; the shell navigates to it
//!     ([`wire::fetch_current_focus`]).
//!   - **Add a project** — clone a Git URL (into `~/darkrun/<name>`) or register a
//!     local git checkout; both write a durable `project.json` ([`add_project`]).
//!   - **No-engine one-step** — a project with no live engine shows the exact
//!     per-harness command to bring an engine up ([`NoEngine`]).
//!   - **Annotate / feedback / checkpoint** — all live in the unchanged
//!     [`crate::review::ReviewApp`], rendered into the main pane for a selected run.
//!
//! The shell is theme-aware (the foundation follows `prefers-color-scheme`) and
//! adds a System / Light / Dark override control in the toolbar, persisted to
//! `localStorage`. A media query in the webview collapses the sidebar to a
//! hamburger drawer and compacts the station strip on a narrow window, matching
//! the mockup's mobile frame.

use std::collections::BTreeMap;
use std::path::PathBuf;

use darkrun_api::RunSummary;
use darkrun_harness::Harness;
use darkrun_ui::prelude::*;

use crate::review::ReviewApp;
use crate::wire::{self, ConnConfig, DiscoveredEngine};

/// A project the shell lists in the sidebar: a working dir keyed by its
/// `~/.darkrun` slug, optionally backed by a live engine (its discovered port).
///
/// Sourced two ways and merged by [`load_projects`]: the durable project registry
/// (`~/.darkrun/<slug>/project.json`, every registered project — live or idle)
/// overlaid with [`wire::discover_live_engines`] (the running engines, which
/// contribute the `port`/`harness` and flip a project to "live").
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
    /// Project a discovered live engine into a [`Project`].
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

/// What the main pane is currently pointed at.
///
/// The sidebar drives this: picking a run opens it ([`Selection::Run`]); picking
/// a no-engine project shows its start command ([`Selection::NoEngine`]); nothing
/// selected shows the projects / add surface ([`Selection::None`]).
#[derive(Clone, PartialEq)]
enum Selection {
    /// Nothing selected — the projects / add-a-project welcome surface.
    None,
    /// A live run opened into its Review: the bound port + the run slug.
    Run { port: u16, slug: String, project: String },
    /// A project with no live engine — show the per-harness one-step command.
    NoEngine { name: String, path: String },
    /// The settings page (theme, …), opened from the toolbar gear.
    Settings,
}

/// The desktop shell: toolbar + sidebar + main pane.
///
/// `cfg` is the env-derived fallback connection (port from `DARKRUN_PORT`).
/// `project_path`, when given, pre-selects a project on launch: discovery binds
/// that project's live engine if present and opens its run list. With no
/// `project_path` the shell opens with nothing selected (the welcome surface).
#[component]
pub fn HomeApp(
    cfg: ConnConfig,
    project_path: Option<PathBuf>,
    /// When the engine launched us pinned to a run, its slug — the shell opens
    /// with that run pre-selected (its Review in the main pane) instead of the
    /// welcome surface. The focus poller refines the project label on its first
    /// tick.
    #[props(default)]
    initial_session: Option<String>,
) -> Element {
    // A pinned project_path is reserved for a future deep-link; discovery already
    // surfaces every live project, so the shell opens on the full tree.
    let _ = &project_path;
    // The engine's launch port — used to seed a pinned run's selection before the
    // poller has discovered which project owns it.
    let launch_port = cfg.port;

    // Every live engine discovered under `~/.darkrun`, refreshed on launch and
    // re-polled. The sidebar projects render from this merged with the registry.
    let mut engines = use_signal(Vec::<DiscoveredEngine>::new);

    // The per-project run lists, keyed by project name, refreshed by the run
    // fetcher. The sidebar nests these under each live project.
    let runs_by_project = use_signal(BTreeMap::<String, Vec<RunSummary>>::new);

    // What the main pane shows. A pinned launch opens straight to that run (the
    // project label is filled in by the focus poller's first tick); otherwise the
    // shell opens with nothing selected.
    let selection = use_signal(move || match initial_session {
        Some(ref slug) => Selection::Run {
            port: launch_port,
            slug: slug.clone(),
            project: String::new(),
        },
        None => Selection::None,
    });

    // The Mine/All sidebar filter — defaults to Mine per the mockup, scoping the
    // run list to runs the current git identity authored (`authored_by_me`).
    let mine_only = use_signal(|| true);

    // The sidebar search filter (matches run/project name, author, status, …).
    let search = use_signal(String::new);

    // Whether the mobile drawer is open (only meaningful on a narrow window; the
    // CSS hides the inline sidebar and shows the drawer toggle below a breakpoint).
    let drawer_open = use_signal(|| false);

    // Bumped after a clone/register so the sidebar re-reads the on-disk registry
    // immediately, rather than waiting for the next discovery tick.
    let refresh = use_signal(|| 0u32);

    // The shell's single poller, driving three things every tick so the sidebar
    // stays live without a relaunch:
    //   1. **Discovery / auto-connect** — re-read `~/.darkrun` so a freshly-booted
    //      engine appears (and a stopped one drops) without a relaunch.
    //   2. **Run lists** — fetch `/api/runs` per LIVE engine so the sidebar can
    //      nest each project's runs (status dots + `me` tags).
    //   3. **Current-focus** — probe the launch engine's `current` session so
    //      `darkrun show` jumps an already-open app straight to a run. We navigate
    //      only on a focus *change* so a manual back-out isn't undone next tick.
    // The first iteration runs immediately (no leading sleep) so the tree fills on
    // launch; subsequent iterations poll on an interval.
    let mut last_focus = use_signal(|| None::<String>);
    {
        let base = cfg.clone();
        let mut selection = selection;
        let mut runs_by_project = runs_by_project;
        use_future(move || {
            let base = base.clone();
            async move {
                loop {
                    // 1. Discovery.
                    let found = wire::discover_live_engines().await.unwrap_or_default();
                    if found != *engines.peek() {
                        engines.set(found.clone());
                    }

                    // 2. Per-project run lists.
                    let mut map = BTreeMap::<String, Vec<RunSummary>>::new();
                    for e in &found {
                        let mut c = base.clone();
                        c.port = e.port;
                        if let Ok(payload) = wire::fetch_runs(&c).await {
                            map.insert(engine_display_name(e), payload.runs);
                        }
                    }
                    if map != *runs_by_project.peek() {
                        runs_by_project.set(map);
                    }

                    // 3. Current-focus on the launch engine's port.
                    let mut probe = base.clone();
                    let focus = wire::fetch_current_focus(&probe).await;
                    if *last_focus.peek() != focus {
                        last_focus.set(focus.clone());
                        if let Some(slug) = focus {
                            let project = found
                                .iter()
                                .find(|e| e.port == probe.port)
                                .map(engine_display_name)
                                .unwrap_or_default();
                            probe.session_id = slug.clone();
                            selection.set(Selection::Run {
                                port: probe.port,
                                slug,
                                project,
                            });
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                }
            }
        });
    }

    // Apply the persisted theme override on launch. The control that *changes* it
    // now lives in the Settings page, which isn't mounted at startup — so the
    // shell itself re-applies the saved `[data-theme]` so a pinned Light/Dark
    // survives a relaunch without the user opening Settings first.
    use_effect(move || {
        spawn(async move {
            if let Ok(label) = document::eval(&format!(
                "return (localStorage.getItem('{THEME_STORAGE_KEY}') || 'system');"
            ))
            .join::<String>()
            .await
            {
                let _ = document::eval(&apply_script(ThemeChoice::from_label(&label)));
            }
        });
    });

    // Reading `refresh` ties a re-render to a clone/register so a freshly-written
    // `project.json` shows without waiting for the poller.
    let _ = refresh.read();
    let projects = load_projects(&engines.read());
    let runs_map = runs_by_project.read().clone();
    let live_count = engines.read().len();

    rsx! {
        // Shell-local CSS: the responsive sidebar→drawer collapse + station-strip
        // compaction the mockup's mobile frame shows. Scoped to `.dr-shell-*`.
        style { "{SHELL_CSS}" }
        div { class: "dr-shell",
            Toolbar { drawer_open, selection }
            div { class: "dr-shell-body",
                Sidebar {
                    projects: projects.clone(),
                    runs_map: runs_map.clone(),
                    selection,
                    mine_only,
                    search,
                    drawer_open,
                    live_count,
                }
                main { class: "dr-shell-main",
                    MainPane {
                        cfg: cfg.clone(),
                        selection,
                        projects: projects.clone(),
                        refresh,
                    }
                }
            }
        }
    }
}

/// The display name the sidebar keys a project's runs under — the engine slug,
/// which matches [`Project::name`].
fn engine_display_name(e: &DiscoveredEngine) -> String {
    e.slug.clone()
}

/// Shell-local CSS: the responsive collapse to a hamburger drawer and the
/// station-strip compaction on a narrow window, matching the mockup's mobile
/// frame. The desktop window is resizable, so this rides a viewport media query
/// inside the webview rather than any window event.
const SHELL_CSS: &str = r#"
/* Reset the webview's default 8px body margin so the shell sits flush to the
   window edges (otherwise it's inset on all sides AND 100vh overflows into a
   scrollbar). The website has its own reset; the desktop needs this one. */
html,body{ margin:0; padding:0; height:100%; }
*{ box-sizing:border-box; }
.dr-shell{ display:flex; flex-direction:column; height:100vh; overflow:hidden;
  background:var(--dr-surface-base); color:var(--dr-text);
  font-family:var(--dr-font-sans); }
.dr-shell-body{ flex:1; display:flex; min-height:0; }
.dr-shell-main{ flex:1; min-width:0; overflow:auto; }
.dr-shell-side{ width:248px; flex:none; background:var(--dr-surface-sink);
  border-right:1px solid var(--dr-border); display:flex; flex-direction:column;
  min-height:0; }
.dr-shell-burger{ display:none; }
.dr-shell-drawer-scrim{ display:none; }
/* Narrow window: collapse the inline sidebar; the burger toggles a drawer that
   slides over the content. Mirrors the mockup's mobile frame. */
@media (max-width: 720px){
  .dr-shell-burger{ display:inline-flex; }
  .dr-shell-side{ position:fixed; top:0; left:0; bottom:0; z-index:50;
    transform:translateX(-100%); transition:transform .18s ease;
    box-shadow:0 0 40px rgba(0,0,0,.5); }
  .dr-shell-side[data-open="true"]{ transform:translateX(0); }
  .dr-shell-drawer-scrim[data-open="true"]{ display:block; position:fixed;
    inset:0; z-index:40; background:rgba(0,0,0,.45); }
  /* Compact the station strip: hide the labels (the mockup's mobile strip). */
  .dr-shell-main .dr-station-strip .dr-station-lbl{ display:none; }
}
"#;

/// The unified top toolbar: the theme-aware wordmark, a spacer, the theme
/// override control, and (on a narrow window) a hamburger that toggles the
/// sidebar drawer.
#[component]
fn Toolbar(drawer_open: Signal<bool>, selection: Signal<Selection>) -> Element {
    // On macOS the title bar is transparent + fullsize-content, so this toolbar
    // sits at the very top with the traffic lights floating over its left — pad
    // left to clear them. The bar is a window drag region (`-webkit-app-region`);
    // the interactive controls opt back out with `no-drag`.
    let left_pad = if cfg!(target_os = "macos") { 78 } else { 14 };
    let bar = format!(
        "height:44px;flex:none;display:flex;align-items:center;gap:12px;\
         padding:0 14px 0 {left_pad}px;background:{overlay};\
         border-bottom:1px solid {border};-webkit-app-region:drag;user-select:none;",
        overlay = tokens::var::SURFACE_OVERLAY,
        border = tokens::var::BORDER,
    );
    let burger = format!(
        "appearance:none;background:transparent;border:1px solid {border};\
         border-radius:7px;width:30px;height:26px;align-items:center;\
         justify-content:center;color:{muted};font-size:15px;cursor:pointer;\
         -webkit-app-region:no-drag;",
        border = tokens::var::BORDER_STRONG,
        muted = tokens::var::TEXT_MUTED,
    );
    // The settings gear — opens the settings page (theme, …) in the main pane.
    let on_settings = matches!(*selection.read(), Selection::Settings);
    let gear = format!(
        "appearance:none;background:{bg};border:1px solid {border};\
         border-radius:7px;width:30px;height:26px;display:inline-flex;align-items:center;\
         justify-content:center;color:{color};font-size:14px;cursor:pointer;\
         -webkit-app-region:no-drag;",
        bg = if on_settings { tokens::var::SURFACE_OVERLAY } else { "transparent" },
        border = tokens::var::BORDER_STRONG,
        color = if on_settings { tokens::var::ACCENT } else { tokens::var::TEXT_MUTED },
    );

    let mut drawer_open = drawer_open;
    let mut selection = selection;
    rsx! {
        header { style: "{bar}",
            button {
                class: "dr-shell-burger",
                style: "{burger}",
                "aria-label": "Toggle projects",
                onclick: move |_| {
                    let now = *drawer_open.peek();
                    drawer_open.set(!now);
                },
                "\u{2630}"
            }
            Wordmark { variant: WordmarkVariant::OutlinedSolidRun, size: 22.0 }
            span { style: "flex:1;" }
            // The gear opts out of the drag region so it stays clickable.
            button {
                class: "dr-shell-settings",
                style: "{gear}",
                "aria-label": "Settings",
                title: "Settings",
                onclick: move |_| selection.set(Selection::Settings),
                "\u{2699}"
            }
        }
    }
}

/// The localStorage key the theme control persists its choice under.
const THEME_STORAGE_KEY: &str = "darkrun-theme";

/// The System / Light / Dark theme override control.
///
/// The foundation follows the system appearance by default; this pins a manual
/// override by setting the root `[data-theme]` attribute (via
/// [`apply_script`]) and persists the label to `localStorage` so it survives a
/// relaunch. "System" removes the attribute, returning to `prefers-color-scheme`.
/// The choice signal seeds from the persisted value on mount.
#[component]
fn ThemeControl() -> Element {
    let mut choice = use_signal(|| ThemeChoice::System);

    // Seed the control from the persisted choice once, after mount, and re-apply
    // it so a relaunch lands on the same theme.
    use_effect(move || {
        spawn(async move {
            if let Ok(label) = document::eval(&format!(
                "return (localStorage.getItem('{THEME_STORAGE_KEY}') || 'system');"
            ))
            .join::<String>()
            .await
            {
                let parsed = ThemeChoice::from_label(&label);
                choice.set(parsed);
                let _ = document::eval(&apply_script(parsed));
            }
        });
    });

    let wrap = format!(
        "display:inline-flex;align-items:center;gap:2px;\
         border:1px solid {border};border-radius:999px;padding:2px;\
         background:{raised};",
        border = tokens::var::BORDER_STRONG,
        raised = tokens::var::SURFACE_RAISED,
    );

    rsx! {
        div { style: "{wrap}", role: "group", "aria-label": "Theme",
            for opt in ThemeChoice::ALL {
                {
                    let active = choice() == opt;
                    let seg = format!(
                        "appearance:none;border:0;cursor:pointer;\
                         font-family:{mono};font-size:11px;letter-spacing:0.02em;\
                         padding:4px 10px;border-radius:999px;line-height:1;\
                         color:{fg};background:{bg};",
                        mono = tokens::FONT_MONO,
                        fg = if active { tokens::var::ON_ACCENT } else { tokens::var::TEXT_MUTED },
                        bg = if active { tokens::var::ACCENT } else { "transparent" },
                    );
                    rsx! {
                        button {
                            style: "{seg}",
                            "aria-pressed": if active { "true" } else { "false" },
                            title: "{opt.display_label()} theme",
                            onclick: move |_| {
                                choice.set(opt);
                                let _ = document::eval(&apply_script(opt));
                                let _ = document::eval(&format!(
                                    "try{{localStorage.setItem('{THEME_STORAGE_KEY}','{}');}}catch(e){{}}",
                                    opt.label(),
                                ));
                            },
                            "{opt.display_label()}"
                        }
                    }
                }
            }
        }
    }
}

/// The full-height left sidebar: a Projects header, the Mine/All filter, a search
/// box, the project→runs tree, and a live-engine footer. On a narrow window this
/// is the drawer the toolbar's hamburger toggles.
#[component]
fn Sidebar(
    projects: Vec<Project>,
    runs_map: BTreeMap<String, Vec<RunSummary>>,
    selection: Signal<Selection>,
    mine_only: Signal<bool>,
    search: Signal<String>,
    drawer_open: Signal<bool>,
    live_count: usize,
) -> Element {
    let open = *drawer_open.read();

    let head = "padding:12px 12px 8px;display:flex;align-items:center;\
                justify-content:space-between;gap:8px;";
    let head_label = format!(
        "margin:0;font-family:{sans};font-size:11px;font-weight:700;\
         text-transform:uppercase;letter-spacing:0.05em;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let foot = format!(
        "padding:8px 12px;border-top:1px solid {border};font-family:{mono};\
         font-size:11px;color:{faint};display:flex;align-items:center;gap:7px;",
        border = tokens::var::BORDER,
        mono = tokens::FONT_MONO,
        faint = tokens::var::TEXT_FAINT,
    );
    let dot_live = format!(
        "width:7px;height:7px;border-radius:50%;flex:none;background:{ok};",
        ok = tokens::var::STATUS_OK,
    );

    let mut drawer_open_close = drawer_open;
    let visible = visible_projects(&projects, &runs_map, *mine_only.read(), &search.read());

    rsx! {
        // The drawer scrim (narrow window only) — tap to close.
        div {
            class: "dr-shell-drawer-scrim",
            "data-open": if open { "true" } else { "false" },
            onclick: move |_| drawer_open_close.set(false),
        }
        aside {
            class: "dr-shell-side",
            "data-open": if open { "true" } else { "false" },
            div { style: "{head}",
                span { style: "{head_label}", "Projects" }
                Badge { tone: Tone::Neutral, "{projects.len()}" }
            }
            MineAllFilter { mine_only }
            SidebarSearch { search }
            div { style: "flex:1;overflow:auto;padding:2px 6px 10px;",
                if visible.is_empty() {
                    SidebarEmpty { mine_only, has_projects: !projects.is_empty() }
                } else {
                    for proj in visible.iter() {
                        ProjectSection {
                            proj: proj.clone(),
                            runs: runs_map.get(&proj.name).cloned().unwrap_or_default(),
                            selection,
                            mine_only,
                            search,
                            drawer_open,
                        }
                    }
                }
            }
            div { style: "{foot}",
                span { style: "{dot_live}" }
                "engine \u{b7} {live_count} live"
            }
        }
    }
}

/// The projects that survive the current Mine/All filter + search query. A
/// project is kept when it has at least one matching run, OR (so idle/no-engine
/// projects aren't hidden) when its own name matches the search and the Mine
/// filter isn't excluding it for lack of authored runs.
fn visible_projects(
    projects: &[Project],
    runs_map: &BTreeMap<String, Vec<RunSummary>>,
    mine_only: bool,
    query: &str,
) -> Vec<Project> {
    let q = query.trim().to_ascii_lowercase();
    projects
        .iter()
        .filter(|p| {
            let runs = runs_map.get(&p.name).map(Vec::as_slice).unwrap_or(&[]);
            let any_run = runs.iter().any(|r| run_matches(r, mine_only, &q));
            // Keep a project with matching runs; also keep an engine-less /
            // run-less project whose name matches the search (or no search), so
            // the add/idle projects stay reachable. Under Mine with a non-empty
            // run list that has no authored matches, hide it.
            if any_run {
                return true;
            }
            if runs.is_empty() {
                return q.is_empty() || p.name.to_ascii_lowercase().contains(&q);
            }
            false
        })
        .cloned()
        .collect()
}

/// Whether a run survives the Mine/All filter and the search query. The query
/// matches the run's title/slug, status, active station, and author.
fn run_matches(run: &RunSummary, mine_only: bool, query_lc: &str) -> bool {
    if mine_only && !run.authored_by_me {
        return false;
    }
    if query_lc.is_empty() {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {}",
        run.title,
        run.slug,
        run.status,
        run.active_station,
        run.author.as_deref().unwrap_or(""),
    )
    .to_ascii_lowercase();
    hay.contains(query_lc)
}

/// The Mine / All segmented filter at the top of the sidebar. Defaults to Mine.
#[component]
fn MineAllFilter(mine_only: Signal<bool>) -> Element {
    let seg = format!(
        "display:flex;margin:2px 12px 8px;border:1px solid {border};\
         border-radius:8px;overflow:hidden;font-size:12px;",
        border = tokens::var::BORDER_STRONG,
    );
    let mine = *mine_only.read();

    let cell = |on: bool| {
        if on {
            format!(
                "flex:1;text-align:center;padding:5px 0;cursor:pointer;\
                 background:{accent};color:{on_accent};font-weight:700;",
                accent = tokens::var::ACCENT,
                on_accent = tokens::var::ON_ACCENT,
            )
        } else {
            format!(
                "flex:1;text-align:center;padding:5px 0;cursor:pointer;color:{muted};",
                muted = tokens::var::TEXT_MUTED,
            )
        }
    };

    let mut mine_only = mine_only;
    rsx! {
        div { style: "{seg}", role: "group", "aria-label": "Filter runs",
            div {
                style: cell(mine),
                onclick: move |_| mine_only.set(true),
                "Mine"
            }
            div {
                style: cell(!mine),
                onclick: move |_| mine_only.set(false),
                "All"
            }
        }
    }
}

/// The sidebar run/project search box.
#[component]
fn SidebarSearch(search: Signal<String>) -> Element {
    let wrap = format!(
        "margin:0 12px 10px;display:flex;align-items:center;gap:7px;\
         padding:6px 9px;border:1px solid {border};border-radius:8px;\
         background:{raised};",
        border = tokens::var::BORDER,
        raised = tokens::var::SURFACE_RAISED,
    );
    let input = format!(
        "flex:1;appearance:none;border:0;background:transparent;outline:none;\
         color:{text};font-family:{sans};font-size:12px;",
        text = tokens::var::TEXT,
        sans = tokens::FONT_SANS,
    );
    let mut search = search;
    rsx! {
        div { style: "{wrap}",
            span { style: format!("color:{};font-size:12px;", tokens::var::TEXT_FAINT), "\u{2315}" }
            input {
                style: "{input}",
                placeholder: "search \u{b7} name, author, status, station\u{2026}",
                value: "{search}",
                oninput: move |evt| search.set(evt.value()),
            }
        }
    }
}

/// The empty-state line shown when the filter/search hides everything.
#[component]
fn SidebarEmpty(mine_only: Signal<bool>, has_projects: bool) -> Element {
    let style = format!(
        "padding:14px 12px;font-size:12px;color:{faint};line-height:1.5;",
        faint = tokens::var::TEXT_FAINT,
    );
    let msg = if !has_projects {
        "No projects yet \u{2014} add one from the main pane."
    } else if *mine_only.read() {
        "No runs you authored. Switch to All to see the whole team's runs."
    } else {
        "Nothing matches that search."
    };
    rsx! {
        p { style: "{style}", "{msg}" }
    }
}

/// One project section in the sidebar: a collapsible header (name + run count)
/// with its RUNS nested under it. Live projects list their runs (each with a
/// status dot + a `me` tag when authored); a no-engine project's header selects
/// the no-engine one-step view.
#[component]
fn ProjectSection(
    proj: Project,
    runs: Vec<RunSummary>,
    selection: Signal<Selection>,
    mine_only: Signal<bool>,
    search: Signal<String>,
    drawer_open: Signal<bool>,
) -> Element {
    // Collapsed state — start expanded so runs are visible on open.
    let mut expanded = use_signal(|| true);
    let is_open = *expanded.read();

    let q = search.read().trim().to_ascii_lowercase();
    let mine = *mine_only.read();
    let shown: Vec<RunSummary> = runs
        .iter()
        .filter(|r| run_matches(r, mine, &q))
        .cloned()
        .collect();

    let ph = format!(
        "display:flex;align-items:center;gap:7px;padding:6px 8px;border-radius:7px;\
         font-size:13px;font-weight:600;color:{text};cursor:pointer;",
        text = tokens::var::TEXT,
    );
    let car = format!(
        "color:{faint};font-size:10px;width:10px;",
        faint = tokens::var::TEXT_FAINT,
    );
    let ct = format!(
        "margin-left:auto;font-family:{mono};font-size:10.5px;color:{faint};",
        mono = tokens::FONT_MONO,
        faint = tokens::var::TEXT_FAINT,
    );
    let runs_wrap = format!(
        "margin:1px 0 4px 14px;border-left:1px solid {border};padding-left:6px;",
        border = tokens::var::BORDER,
    );

    // Clicking the header of a NO-ENGINE project opens its one-step view; for a
    // live project the header just toggles collapse.
    let proj_for_header = proj.clone();
    let mut selection_hdr = selection;
    let live = proj.is_live();
    let path_label = proj.path.display().to_string();
    let on_header = move |_| {
        if live {
            let now = *expanded.peek();
            expanded.set(!now);
        } else {
            selection_hdr.set(Selection::NoEngine {
                name: proj_for_header.name.clone(),
                path: path_label.clone(),
            });
        }
    };

    rsx! {
        div { style: "margin:2px 0;",
            div { style: "{ph}", onclick: on_header,
                span { style: "{car}", if is_open { "\u{25be}" } else { "\u{25b8}" } }
                span {
                    style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
                    "{proj.name}"
                }
                if live {
                    span { style: "{ct}", "{shown.len()}" }
                } else {
                    span {
                        style: format!(
                            "margin-left:auto;font-family:{};font-size:9px;color:{};\
                             border:1px solid {};border-radius:4px;padding:0 4px;",
                            tokens::FONT_MONO, tokens::var::STATUS_WARN, tokens::var::STATUS_WARN,
                        ),
                        "idle"
                    }
                }
            }
            if live && is_open {
                div { style: "{runs_wrap}",
                    for run in shown.iter() {
                        RunRow {
                            run: run.clone(),
                            project: proj.name.clone(),
                            port: proj.port.unwrap_or(0),
                            selection,
                            drawer_open,
                        }
                    }
                    NewRunRow {
                        name: proj.name.clone(),
                        path: proj.path.display().to_string(),
                        selection,
                    }
                }
            }
        }
    }
}

/// One run row under a project: a status dot (live / needs-review / idle), the
/// run title, and a `me` tag when the current identity authored it. Clicking it
/// opens the run's Review in the main pane.
#[component]
fn RunRow(
    run: RunSummary,
    project: String,
    port: u16,
    selection: Signal<Selection>,
    drawer_open: Signal<bool>,
) -> Element {
    // Is this row the open one?
    let selected = match &*selection.read() {
        Selection::Run { slug, .. } => slug == &run.slug,
        _ => false,
    };

    let row = if selected {
        format!(
            "display:flex;align-items:center;gap:8px;padding:5px 8px;border-radius:6px;\
             font-size:12.5px;cursor:pointer;background:{sel};color:{text};",
            sel = run_sel_bg(),
            text = tokens::var::TEXT,
        )
    } else {
        format!(
            "display:flex;align-items:center;gap:8px;padding:5px 8px;border-radius:6px;\
             font-size:12.5px;cursor:pointer;color:{muted};",
            muted = tokens::var::TEXT_MUTED,
        )
    };
    let dot = format!(
        "width:7px;height:7px;border-radius:50%;flex:none;background:{};",
        run_dot_color(&run.status),
    );
    let me_tag = format!(
        "margin-left:auto;font-size:9px;font-family:{mono};color:{ink};\
         border:1px solid {ink};border-radius:4px;padding:0 4px;",
        mono = tokens::FONT_MONO,
        ink = tokens::var::ACCENT_STRONG,
    );

    let slug = run.slug.clone();
    let mut selection = selection;
    let mut drawer_open = drawer_open;
    rsx! {
        div {
            style: "{row}",
            role: "button",
            tabindex: "0",
            onclick: move |_| {
                selection.set(Selection::Run {
                    port,
                    slug: slug.clone(),
                    project: project.clone(),
                });
                // Close the drawer after a pick on a narrow window.
                drawer_open.set(false);
            },
            span { style: "{dot}" }
            span {
                style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
                "{run.title}"
            }
            if run.authored_by_me {
                span { style: "{me_tag}", "me" }
            }
        }
    }
}

/// The translucent selected-row background. The mockup uses an accent tint; we
/// approximate it with the accent at low alpha (works in both themes).
fn run_sel_bg() -> String {
    format!("{}1f", tokens::var::ACCENT)
}

/// The status-dot color for a run row: ok/green for an active (live) run, amber
/// for one awaiting review, faint/grey for idle. Mirrors the mockup's
/// live / needs-review / idle dots.
fn run_dot_color(status: &str) -> &'static str {
    match status {
        "active" | "in_progress" | "completed" => tokens::var::STATUS_OK,
        "blocked" | "changes_requested" | "pending" | "review" => tokens::var::STATUS_WARN,
        _ => tokens::var::TEXT_FAINT,
    }
}

/// The "+ new run…" affordance under a live project — opens the no-engine
/// one-step view scoped to this project, which surfaces the per-harness start
/// command to paste so a new `darkrun/*/*` worktree can be kicked off.
#[component]
fn NewRunRow(name: String, path: String, selection: Signal<Selection>) -> Element {
    let row = format!(
        "display:flex;align-items:center;gap:8px;padding:5px 8px;border-radius:6px;\
         font-size:12.5px;cursor:pointer;color:{ink};",
        ink = tokens::var::ACCENT_STRONG,
    );
    let mut selection = selection;
    rsx! {
        div {
            style: "{row}",
            role: "button",
            tabindex: "0",
            onclick: move |_| selection.set(Selection::NoEngine {
                name: name.clone(),
                path: path.clone(),
            }),
            span { style: "width:7px;text-align:center;", "\u{ff0b}" }
            span { "new run\u{2026}" }
        }
    }
}

/// The main pane: renders the selected run's Review, a no-engine one-step view, or
/// (with nothing selected) the projects / add-a-project welcome surface.
#[component]
fn MainPane(
    cfg: ConnConfig,
    selection: Signal<Selection>,
    projects: Vec<Project>,
    refresh: Signal<u32>,
) -> Element {
    match selection.read().clone() {
        Selection::Run { port, slug, project } => {
            let mut run_cfg = cfg.with_session(slug.clone());
            run_cfg.port = port;
            rsx! {
                MainHeader { name: slug.clone(), crumb: project.clone() }
                ReviewApp { cfg: run_cfg }
            }
        }
        Selection::NoEngine { name, path } => rsx! {
            MainHeader { name: name.clone(), crumb: path.clone() }
            div { style: "padding:16px;",
                NoEngine { name, path }
            }
        },
        Selection::Settings => rsx! {
            MainHeader { name: "Settings".to_string(), crumb: "appearance & preferences".to_string() }
            SettingsPage {}
        },
        Selection::None => rsx! {
            Welcome { projects: projects.clone(), refresh }
        },
    }
}

/// The settings page — opened from the toolbar gear. Houses the theme override
/// (moved out of the toolbar) and is the home for future preferences.
#[component]
fn SettingsPage() -> Element {
    let wrap = "padding:20px;display:flex;flex-direction:column;gap:8px;max-width:560px;";
    let card = format!(
        "background:{surface};border:1px solid {border};border-radius:10px;padding:16px;\
         display:flex;flex-direction:column;gap:14px;",
        surface = tokens::var::SURFACE_OVERLAY,
        border = tokens::var::BORDER,
    );
    let label = format!(
        "font-size:11px;text-transform:uppercase;letter-spacing:0.06em;color:{faint};\
         font-family:{mono};",
        faint = tokens::var::TEXT_FAINT,
        mono = tokens::FONT_MONO,
    );
    let row = "display:flex;align-items:center;justify-content:space-between;gap:16px;";
    let name = format!("font-size:14px;font-weight:600;color:{text};", text = tokens::var::TEXT);
    let hint = format!(
        "font-size:12px;color:{muted};margin:0;",
        muted = tokens::var::TEXT_MUTED,
    );
    rsx! {
        div { style: "{wrap}",
            div { style: "{label}", "Appearance" }
            div { style: "{card}",
                div { style: "{row}",
                    div { style: "display:flex;flex-direction:column;gap:3px;",
                        span { style: "{name}", "Theme" }
                        p { style: "{hint}", "System follows your OS appearance; Light and Dark pin it." }
                    }
                    ThemeControl {}
                }
            }
        }
    }
}

/// The main pane header: the selected run/project name + a breadcrumb (the
/// owning project, or the project path for a no-engine view).
#[component]
fn MainHeader(name: String, crumb: String) -> Element {
    let bar = format!(
        "padding:11px 16px;border-bottom:1px solid {border};display:flex;\
         align-items:center;gap:10px;",
        border = tokens::var::BORDER,
    );
    let nm = format!(
        "font-weight:700;font-size:14px;color:{text};",
        text = tokens::var::TEXT,
    );
    let cr = format!(
        "color:{faint};font-size:12px;font-family:{mono};\
         overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
        faint = tokens::var::TEXT_FAINT,
        mono = tokens::FONT_MONO,
    );
    rsx! {
        div { style: "{bar}",
            span { style: "{nm}", "{name}" }
            span { style: "{cr}", "{crumb}" }
        }
    }
}

/// The welcome / projects surface shown when nothing is selected: a short lead,
/// the project cards (drilling a live project selects nothing but the sidebar
/// shows its runs; an idle project opens its no-engine view), and the
/// add-a-project form.
#[component]
fn Welcome(projects: Vec<Project>, refresh: Signal<u32>) -> Element {
    let shell = "padding:24px;display:flex;flex-direction:column;gap:16px;\
                 max-width:920px;margin:0 auto;";
    let lead = format!(
        "font-size:13.5px;line-height:1.55;color:{muted};margin:0;max-width:640px;",
        muted = tokens::var::TEXT_MUTED,
    );
    let heading = format!(
        "margin:0;font-family:{sans};font-size:13px;font-weight:700;\
         text-transform:uppercase;letter-spacing:0.04em;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let grid = "display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));\
                gap:12px;";

    rsx! {
        div { style: "{shell}",
            p { style: "{lead}",
                "Pick a run from the sidebar to open its review, or add a project below. \
                 Projects are git repos; the runs nested under each are its darkrun \
                 worktrees."
            }
            h2 { style: "{heading}", "Projects" }
            div { style: "{grid}",
                for proj in projects.iter() {
                    ProjectCard { proj: proj.clone() }
                }
                AddProjectCard {}
            }
            AddProjectForm { refresh }
        }
    }
}

/// One project card on the welcome surface: name + status badge, path, harness
/// line. (Selection is driven from the sidebar; the card is informational here.)
#[component]
fn ProjectCard(proj: Project) -> Element {
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
        text = tokens::var::TEXT,
    );
    let path_style = format!(
        "font-family:{mono};font-size:11px;color:{faint};\
         overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
        mono = tokens::FONT_MONO,
        faint = tokens::var::TEXT_FAINT,
    );
    let meta_style = format!(
        "font-size:12px;color:{muted};margin-top:6px;",
        muted = tokens::var::TEXT_MUTED,
    );

    let path_label = proj.path.display().to_string();
    let harness_label = proj.harness.clone();
    rsx! {
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

/// Merge the on-disk project registry with the live engines into the projects the
/// sidebar + welcome surface render.
///
/// Two sources, keyed by registry slug:
///   1. **Registered projects** — every `~/.darkrun/<slug>/project.json` (via
///      [`registry::list_projects`]), live or idle, carrying the durable
///      name/path even when no engine is running.
///   2. **Live engines** — every running engine ([`DiscoveredEngine`]). A
///      matching registered project gets its port + harness OVERLAID (flipping it
///      live); a live engine with NO registered record still surfaces.
///
/// Idempotent on slug; returns projects sorted by name for a stable order.
fn load_projects(engines: &[DiscoveredEngine]) -> Vec<Project> {
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

/// The two add-a-project entry points: a **Git URL** (clone + register) and a
/// **Local repo** (register an existing git checkout). Both must be git repos; the
/// slug + `.darkrun/` live in the working tree.
///
/// Both paths execute for real (see [`add_project`]). On success `refresh` is
/// bumped so the sidebar + welcome re-read the registry and the new project shows.
#[component]
fn AddProjectForm(refresh: Signal<u32>) -> Element {
    let mut mode = use_signal(|| AddMode::Git);
    let mut value = use_signal(String::new);
    let mut status = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);

    let heading = format!(
        "margin:0;font-family:{sans};font-size:13px;font-weight:700;\
         text-transform:uppercase;letter-spacing:0.04em;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let input_style = format!(
        "flex:1;box-sizing:border-box;padding:9px 12px;border-radius:6px;\
         border:1px solid {border};background:{base};color:{text};\
         font-family:{sans};font-size:13px;",
        border = tokens::var::BORDER,
        base = tokens::var::SURFACE_BASE,
        text = tokens::var::TEXT,
        sans = tokens::FONT_SANS,
    );
    let note_style = format!(
        "font-size:11.5px;color:{faint};margin:8px 0 0;line-height:1.5;",
        faint = tokens::var::TEXT_FAINT,
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

    let mut refresh = refresh;
    let on_submit = move |_| {
        if *busy.peek() {
            return;
        }
        let raw = value.read().trim().to_string();
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
            div { style: "display:flex;gap:8px;margin-top:10px;",
                ModeTab { label: "Git URL", active: current == AddMode::Git,
                    on_pick: move |_| { mode.set(AddMode::Git); status.set(None); } }
                ModeTab { label: "Local repo", active: current == AddMode::Local,
                    on_pick: move |_| { mode.set(AddMode::Local); status.set(None); } }
            }
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
                        accent = tokens::var::ACCENT,
                    ),
                    "{msg}"
                }
            }
        }
    }
}

/// The dashed "add a project" hint card pointing at the form below.
#[component]
fn AddProjectCard() -> Element {
    let style = format!(
        "display:flex;align-items:center;justify-content:center;\
         border:1px dashed {border};border-radius:10px;padding:14px 16px;\
         color:{faint};font-family:{sans};font-size:13px;min-height:96px;",
        border = tokens::var::BORDER_STRONG,
        faint = tokens::var::TEXT_FAINT,
        sans = tokens::FONT_SANS,
    );
    rsx! {
        div { style: "{style}", "\u{ff0b} add a project below" }
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

/// Cheap, synchronous validation of an add-a-project input — run on the UI thread
/// before any blocking work so the operator gets instant feedback. The heavy
/// lifting (clone, repo check) happens in [`add_project`].
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
/// thread (a clone is network-bound). Returns a human-readable success note or an
/// error string suitable for the status line.
///
/// - **Git URL**: clone into `~/darkrun/<repo-name>` via [`darkrun_git::clone_repo`],
///   then write a [`ProjectRecord`] to `~/.darkrun/<slug>/project.json`.
/// - **Local repo**: validate it's a git checkout, then register it.
///
/// Registration is idempotent.
fn add_project(mode: AddMode, raw: &str) -> Result<String, String> {
    validate_add_input(mode, raw)?;
    match mode {
        AddMode::Git => {
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
             background:{accent_bg}14;color:{text};font-size:12.5px;cursor:pointer;",
            accent = tokens::var::ACCENT,
            // Hex (not the var) so the `14` alpha suffix is a valid 8-digit color;
            // a faint static tint behind the active tab reads on both themes.
            accent_bg = tokens::ACCENT,
            text = tokens::var::TEXT,
        )
    } else {
        format!(
            "padding:6px 12px;border-radius:7px;border:1px solid {border};\
             background:transparent;color:{muted};font-size:12.5px;cursor:pointer;",
            border = tokens::var::BORDER_STRONG,
            muted = tokens::var::TEXT_MUTED,
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

/// The no-engine ONE-STEP state: the run/project has no live engine, so the
/// desktop can't drive it. Instead of a dead screen, show the exact terminal
/// command per harness to bring an engine up — and note the desktop auto-connects
/// the instant the engine advertises itself to `~/.darkrun`.
///
/// An autonomous-mode toggle (default ON) controls whether the command splices
/// the harness's permission-bypass flag.
#[component]
fn NoEngine(name: String, path: String, #[props(default)] run: Option<String>) -> Element {
    let mut autonomous = use_signal(|| true);
    let mut selected = use_signal(|| 0usize);

    let order = harness_order();
    let sel = *selected.read();
    let active_harness = order.get(sel).copied().unwrap_or(Harness::ClaudeCode);
    let auto = *autonomous.read();

    let card_style = format!(
        "background:{raised};border:1px solid {border};border-top:2px solid {warn};\
         border-radius:8px;padding:16px 18px;",
        raised = tokens::var::SURFACE_RAISED,
        border = tokens::var::BORDER,
        warn = tokens::var::STATUS_WARN,
    );
    let head_style = "display:flex;align-items:center;justify-content:space-between;gap:10px;";
    let title_style = format!(
        "font-family:{sans};font-size:16px;font-weight:700;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let body_style = format!(
        "font-size:13.5px;line-height:1.55;color:{muted};margin:10px 0 14px;",
        muted = tokens::var::TEXT_MUTED,
    );
    let path_style = format!(
        "font-family:{mono};color:{faint};",
        mono = tokens::FONT_MONO,
        faint = tokens::var::TEXT_FAINT,
    );

    let toggle_bar = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:10px;\
         padding:10px 12px;border:1px solid {border};border-radius:8px;\
         background:{overlay};margin-bottom:12px;",
        border = tokens::var::BORDER_STRONG,
        overlay = tokens::var::SURFACE_OVERLAY,
    );
    let pill_style = if auto {
        format!(
            "font-family:{mono};font-size:11px;background:{ok};color:{on};\
             border-radius:999px;padding:3px 11px;font-weight:700;cursor:pointer;",
            mono = tokens::FONT_MONO,
            ok = tokens::var::STATUS_OK,
            on = tokens::var::SURFACE_BASE,
        )
    } else {
        format!(
            "font-family:{mono};font-size:11px;border:1px solid {border};color:{muted};\
             border-radius:999px;padding:3px 11px;font-weight:700;cursor:pointer;",
            mono = tokens::FONT_MONO,
            border = tokens::var::BORDER_STRONG,
            muted = tokens::var::TEXT_MUTED,
        )
    };

    let tabs_style = "display:flex;gap:6px;flex-wrap:wrap;margin-bottom:12px;";
    let cmd_style = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:12px;\
         padding:12px 14px;border-radius:8px;background:{base};\
         border:1px solid {border};font-family:{mono};font-size:13px;color:{accent};",
        base = tokens::var::SURFACE_BASE,
        border = tokens::var::BORDER,
        mono = tokens::FONT_MONO,
        accent = tokens::var::ACCENT,
    );
    let foot_style = format!(
        "font-size:11.5px;color:{faint};margin:12px 0 0;line-height:1.5;",
        faint = tokens::var::TEXT_FAINT,
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

            div { style: "{tabs_style}",
                for (idx, h) in order.iter().enumerate() {
                    HarnessTab {
                        label: h.capabilities().display_name.to_string(),
                        active: idx == sel,
                        on_pick: move |_| selected.set(idx),
                    }
                }
            }

            div { style: "{cmd_style}",
                span { "{command}" }
                span { style: "font-size:11px;color:var(--dr-text-faint);", "copy \u{29c9}" }
            }

            if active_harness.capabilities().autonomous_launch_args.is_none() {
                p {
                    style: format!(
                        "font-size:11.5px;color:{faint};margin:10px 0 0;line-height:1.5;",
                        faint = tokens::var::TEXT_FAINT,
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

/// Harness tab order for the no-engine command: lead with the CLI harnesses that
/// take a launch flag, GUI ones after.
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

/// The launch binary a harness is invoked as on the command line. GUI harnesses
/// have no terminal launcher — `None` renders an open-the-app instruction.
fn harness_binary(h: Harness) -> Option<&'static str> {
    match h {
        Harness::ClaudeCode => Some("claude"),
        Harness::Codex => Some("codex"),
        Harness::GeminiCli => Some("gemini"),
        Harness::Opencode => Some("opencode"),
        Harness::Cursor | Harness::Windsurf | Harness::Kiro => None,
    }
}

/// Single-quote a path for a POSIX shell so the copied `cd` survives spaces.
fn sh_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Build the ONE-STEP start command for a harness in a project dir. The path is
/// shell-quoted; when `autonomous` is on the harness's bypass flag (if any) is
/// spliced in; when the harness exposes a worktree flag and `run` is given,
/// `<flag> darkrun-<run>` is appended. GUI harnesses get an open-the-editor note.
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
             background:{accent_bg}14;color:{text};font-size:12.5px;cursor:pointer;",
            accent = tokens::var::ACCENT,
            // Hex (not the var) so the `14` alpha suffix is a valid 8-digit color;
            // a faint static tint behind the active tab reads on both themes.
            accent_bg = tokens::ACCENT,
            text = tokens::var::TEXT,
        )
    } else {
        format!(
            "padding:6px 12px;border-radius:7px;border:1px solid {border};\
             background:transparent;color:{muted};font-size:12.5px;cursor:pointer;",
            border = tokens::var::BORDER_STRONG,
            muted = tokens::var::TEXT_MUTED,
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
