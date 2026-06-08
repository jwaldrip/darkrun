//! `/factories`, `/factories/:slug`, and `/factories/:factory/stations/:station`
//! — rendered from the embedded factory corpus in `darkrun-content`. This is real
//! content, not a stub: every factory the binary ships is listed and detailed
//! here, every station drills down to its real role instructions.

use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::content::render_markdown;
use crate::factory_view::{
    flow_stations, humanize, pipeline_slugs, right_size_tiers, role_view, station_index,
};
use crate::route::Route;
use crate::ui::SectionHead;

use darkrun_content::{Factory, Station};

/// `/factories` — the index of every embedded factory.
#[component]
pub fn Factories() -> Element {
    let slugs = darkrun_content::list_factories();
    rsx! {
        SectionHead {
            kicker: "the corpus".to_string(),
            title: "Factories".to_string(),
            lead: Some(
                "A factory is a methodology: an ordered set of stations that take work from intent \
                 to shipped. These ship inside the darkrun binary."
                    .to_string(),
            ),
        }
        if slugs.is_empty() {
            EmptyState {}
        } else {
            div { class: "dr-grid",
                for slug in slugs {
                    FactoryTile { slug }
                }
            }
        }
    }
}

/// A single factory tile, loaded and validated from the corpus.
#[component]
fn FactoryTile(slug: String) -> Element {
    // Load, falling back to a minimal card if validation fails so one bad
    // factory cannot blank the whole index.
    match darkrun_content::load_validated(&slug) {
        Ok(factory) => {
            let desc = factory.frontmatter.description.clone();
            let category = factory.frontmatter.category.clone();
            let station_count = factory.stations.len();
            rsx! {
                Link {
                    to: Route::FactoryDetail { slug: slug.clone() },
                    // `display:grid;height:100%` lets the `.dr-grid` stretch reach the
                    // Card so every tile in a row matches the tallest one's height.
                    style: "text-decoration:none;display:grid;height:100%;",
                    Card {
                        div {
                            style: format!(
                                "display:flex;justify-content:space-between;align-items:center;gap:10px;margin-bottom:8px;font-family:{};",
                                tokens::FONT_SANS,
                            ),
                            span {
                                style: format!(
                                    "font-size:17px;font-weight:700;color:{};text-transform:capitalize;",
                                    theme::TEXT,
                                ),
                                "{slug}"
                            }
                            Badge { tone: Tone::Neutral, "{station_count} stations" }
                        }
                        if !category.is_empty() {
                            div { style: "margin-bottom:8px;",
                                Badge { tone: Tone::Info, "{category}" }
                            }
                        }
                        p {
                            style: format!(
                                "font-family:{};font-size:14px;color:{};margin:0;",
                                tokens::FONT_SANS, theme::TEXT_MUTED,
                            ),
                            "{desc}"
                        }
                    }
                }
            }
        }
        Err(err) => {
            let msg = err.to_string();
            rsx! {
                Card {
                    span {
                        style: format!("font-family:{};color:{};", tokens::FONT_MONO, theme::STATUS_WARN),
                        "{slug}: {msg}"
                    }
                }
            }
        }
    }
}

/// `/factories/:slug` — the enriched factory view: overview, the interactive
/// station-flow diagram, a run walkthrough, the right-sizing strip, the
/// fix-workers track, then every station as a card linking to its detail page.
#[component]
pub fn FactoryDetail(slug: String) -> Element {
    match darkrun_content::load_validated(&slug) {
        Ok(factory) => rsx! { FactoryBody { slug, factory } },
        Err(err) => {
            let msg = err.to_string();
            rsx! {
                SectionHead {
                    kicker: "not found".to_string(),
                    title: slug.clone(),
                    lead: Some(format!("This factory could not be loaded: {msg}")),
                }
                Link { to: Route::Factories {},
                    Button { variant: ButtonVariant::Secondary, "Back to factories" }
                }
            }
        }
    }
}

/// The loaded body of a factory detail page. Split out so the click handler can
/// own a router navigation closure with the factory slug in scope.
#[component]
fn FactoryBody(slug: String, factory: ReadSignal<Factory>) -> Element {
    let factory = factory();
    let overview = render_markdown(&factory.body);
    let default_model = factory.frontmatter.default_model.clone();
    let flows = flow_stations(&factory);
    let walk_flows = flows.clone();
    let tiers = right_size_tiers(&factory);
    let full = pipeline_slugs(&factory);
    let fix_workers = factory.frontmatter.fix_workers.clone();
    let surfaces = factory.frontmatter.surfaces.clone();

    // Clicking a node in the StationFlow navigates to that station's detail page.
    let nav = use_navigator();
    let factory_slug = slug.clone();
    let on_select = move |station_slug: String| {
        nav.push(Route::StationDetail {
            factory: factory_slug.clone(),
            station: station_slug,
        });
    };

    rsx! {
        div { style: "margin-bottom:8px;",
            Link { to: Route::Factories {},
                span {
                    style: format!("font-family:{};font-size:13px;color:{};", tokens::FONT_MONO, theme::ACCENT),
                    "\u{2190} all factories"
                }
            }
        }
        SectionHead {
            kicker: "factory".to_string(),
            title: slug.clone(),
            lead: Some(factory.frontmatter.description.clone()),
        }
        if !default_model.is_empty() {
            div { style: "margin-bottom:16px;",
                Badge { tone: Tone::Info, "default model: {default_model}" }
            }
        }

        // The interactive pipeline: click a station to open its detail page.
        Panel { label: "the assembly line".to_string(),
            div { style: "overflow-x:auto;",
                StationFlow { stations: flows.clone(), on_select, show_checkpoints: false }
            }
            p {
                style: format!(
                    "font-family:{};font-size:12px;color:{};margin:8px 0 0;",
                    tokens::FONT_MONO, theme::TEXT_FAINT,
                ),
                "Click a station to open its phase machine and role instructions."
            }
        }

        // The standout: a run stepper synchronizing the pipeline and phase ring.
        Panel { label: "walk a run".to_string(),
            RunWalkthrough { stations: walk_flows }
        }

        // Right-sizing: how small runs collapse stations.
        Panel { label: "right-sizing".to_string(),
            p {
                style: format!(
                    "font-family:{};font-size:13px;color:{};margin:0 0 10px;",
                    tokens::FONT_SANS, theme::TEXT_MUTED,
                ),
                "At run start the factory assesses size and may collapse stations. A one-line fix \
                 drops straight to build → prove; bigger work keeps the full line."
            }
            RightSizeStrip { full, tiers }
        }

        // Surfaces: the delivery shapes this factory can classify a run into,
        // which route how Prove/Audit verify. Declared per-factory data.
        if !surfaces.is_empty() {
            Panel { label: "delivery surfaces".to_string(),
                p {
                    style: format!(
                        "font-family:{};font-size:13px;color:{};margin:0 0 10px;",
                        tokens::FONT_SANS, theme::TEXT_MUTED,
                    ),
                    "At Shape the factory classifies the run into one of these surfaces. \
                     The surface routes how Prove and Audit verify it — a visual surface \
                     through a headless browser, a library through benchmarks."
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;",
                    for s in surfaces.iter() {
                        Badge { tone: Tone::Info, "{humanize(s)}" }
                    }
                }
            }
        }

        // fix-workers: the drift/feedback repair track.
        if !fix_workers.is_empty() {
            Panel { label: "fix-workers".to_string(),
                p {
                    style: format!(
                        "font-family:{};font-size:13px;color:{};margin:0 0 10px;",
                        tokens::FONT_SANS, theme::TEXT_MUTED,
                    ),
                    "When a checkpoint routes rework back as drift or feedback, fix-workers take the \
                     repair without re-running the whole station."
                }
                div { style: "display:flex;gap:8px;flex-wrap:wrap;",
                    for fw in fix_workers.iter() {
                        Badge { tone: Tone::Warn, "{humanize(fw)}" }
                    }
                }
            }
        }

        article { class: "dr-prose", style: "margin-top:24px;", dangerous_inner_html: "{overview}" }

        // Every station as a card linking to its deep page.
        div { style: "margin-top:32px;display:flex;flex-direction:column;gap:14px;",
            for (i, station) in factory.stations.iter().enumerate() {
                StationCard {
                    index: i,
                    factory: slug.clone(),
                    station: station.name().to_string(),
                    label: humanize(station.label()),
                    description: station.frontmatter.description.clone(),
                    explorers: station.explorers.iter().map(|r| humanize(r.name())).collect(),
                    workers: station.workers.iter().map(|r| humanize(r.name())).collect(),
                    reviewers: station.reviewers.iter().map(|r| humanize(r.name())).collect(),
                }
            }
        }
    }
}

/// One station summary card on a factory detail page: its phase accent and role
/// roster, linking to the deep station page. The gate is global (the run's mode),
/// so no per-station checkpoint is shown here.
#[component]
fn StationCard(
    index: usize,
    factory: String,
    /// The fixed position slug — keys the route/URL.
    station: String,
    /// The domain-facing display label (already humanized).
    label: String,
    description: String,
    explorers: Vec<String>,
    workers: Vec<String>,
    reviewers: Vec<String>,
) -> Element {
    let phase = phase_for_index(index);
    let accent = phase.map(|p| p.hue_var().base.to_string());
    rsx! {
        Link {
            to: Route::StationDetail { factory: factory.clone(), station: station.clone() },
            style: "text-decoration:none;display:block;",
            Card { accent: accent.clone(),
                div {
                    style: format!(
                        "display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:6px;font-family:{};",
                        tokens::FONT_SANS,
                    ),
                    span {
                        style: format!(
                            "font-size:17px;font-weight:700;color:{};text-transform:capitalize;",
                            theme::TEXT,
                        ),
                        "{index + 1}. {label}"
                    }
                }
                if !description.is_empty() {
                    p {
                        style: format!("font-family:{};font-size:14px;color:{};margin:0 0 10px;", tokens::FONT_SANS, theme::TEXT_MUTED),
                        "{description}"
                    }
                }
                RoleRow { label: "explorers".to_string(), roles: explorers }
                RoleRow { label: "workers".to_string(), roles: workers }
                RoleRow { label: "reviewers".to_string(), roles: reviewers }
                div { style: format!("margin-top:8px;font-family:{};font-size:12px;color:{};", tokens::FONT_MONO, theme::ACCENT),
                    "open station \u{2192}"
                }
            }
        }
    }
}

/// `/factories/:factory/stations/:station` — the deep station page: header,
/// the phase machine, per-phase role sections (each role an ExpandableRole with
/// its real markdown), and prev/next nav along the pipeline.
#[component]
pub fn StationDetail(factory: String, station: String) -> Element {
    match darkrun_content::load_validated(&factory) {
        Ok(loaded) => match loaded.station(&station).cloned() {
            Some(st) => rsx! {
                StationBody { factory: factory.clone(), factory_data: loaded, station: st }
            },
            None => rsx! {
                StationMissing {
                    factory: factory.clone(),
                    message: format!("Station `{station}` is not part of the {factory} factory."),
                }
            },
        },
        Err(err) => rsx! {
            StationMissing {
                factory: factory.clone(),
                message: format!("This factory could not be loaded: {err}"),
            }
        },
    }
}

/// The loaded body of a station detail page.
#[component]
fn StationBody(
    factory: String,
    factory_data: ReadSignal<Factory>,
    station: ReadSignal<Station>,
) -> Element {
    let factory_data = factory_data();
    let station = station();
    let idx = station_index(&factory_data, station.name()).unwrap_or(0);

    let fm = &station.frontmatter;
    let locked_artifact = fm.locked_artifact.clone();
    let inputs = fm.inputs.clone();
    let body_html = render_markdown(&station.body);

    // Prev/next stations along the pipeline. Each carries the routing slug plus
    // the domain-facing display label (humanized) shown on the nav button.
    let prev = idx
        .checked_sub(1)
        .and_then(|i| factory_data.stations.get(i))
        .map(|s| (s.name().to_string(), humanize(s.label())));
    let next = factory_data
        .stations
        .get(idx + 1)
        .map(|s| (s.name().to_string(), humanize(s.label())));

    // Role view-models, rendered with the site markdown renderer.
    let explorers: Vec<_> = station.explorers.iter().map(|r| role_view(r, render_markdown)).collect();
    let workers: Vec<_> = station.workers.iter().map(|r| role_view(r, render_markdown)).collect();
    let reviewers: Vec<_> = station.reviewers.iter().map(|r| role_view(r, render_markdown)).collect();

    rsx! {
        div { style: "margin-bottom:8px;",
            Link { to: Route::FactoryDetail { slug: factory.clone() },
                span {
                    style: format!("font-family:{};font-size:13px;color:{};", tokens::FONT_MONO, theme::ACCENT),
                    "\u{2190} {factory} factory"
                }
            }
        }
        SectionHead {
            kicker: format!("station {} / {}", idx + 1, factory_data.stations.len()),
            title: humanize(station.label()),
            lead: Some(fm.description.clone()),
        }

        // Header: risk killed, locked artifact, inputs.
        div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:16px;",
            if let Some(risk) = crate::factory_view::risk_from_body(&station.body) {
                RiskChip { risk }
            }
        }
        if !locked_artifact.is_empty() {
            div { style: "margin-bottom:12px;",
                ArtifactCard {
                    name: locked_artifact.clone(),
                    description: Some(format!("locked by {}", humanize(station.label()))),
                }
            }
        }
        if !inputs.is_empty() {
            div { style: "display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;margin-bottom:20px;",
                span {
                    style: format!(
                        "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;color:{};",
                        tokens::FONT_MONO, theme::TEXT_FAINT,
                    ),
                    "inputs"
                }
                for input in inputs.iter() {
                    Badge { tone: Tone::Neutral, "{input}" }
                }
            }
        }

        // The station purpose / risk prose.
        article { class: "dr-prose", style: "margin:24px 0;", dangerous_inner_html: "{body_html}" }

        // Per-phase role sections.
        PhaseSection {
            heading: "Explore".to_string(),
            phase: Some(Phase::Spec),
            note: "Explorers gather only the context this station needs.".to_string(),
            roles: explorers,
        }
        PhaseSection {
            heading: "Manufacture · Make → Challenge → Resolve".to_string(),
            phase: Some(Phase::Manufacture),
            note: "Workers run the pass loop: make a candidate, challenge it, resolve the weakness.".to_string(),
            roles: workers,
            beats: true,
        }
        PhaseSection {
            heading: "Review / Audit".to_string(),
            phase: Some(Phase::Review),
            note: "Reviewers verify output against criteria, independent of the workers that produced it.".to_string(),
            roles: reviewers,
        }

        // Gate section — the gate is global (the run's mode), not per-station.
        Panel { label: "gate".to_string(),
            span {
                style: format!("font-family:{};font-size:13px;color:{};", tokens::FONT_SANS, theme::TEXT_MUTED),
                "Every station ends in a checkpoint, but the gate is set by the run's "
                b { "mode" }
                ", not the station: "
                b { "team" }
                " opens a PR the team reviews and merges, "
                b { "solo" }
                " asks for local review, and "
                b { "dark" }
                " advances automatically. Pick the mode when you start the run."
            }
        }

        // Prev/next pipeline nav.
        div { style: format!("margin-top:32px;display:flex;justify-content:space-between;gap:12px;border-top:1px solid {};padding-top:16px;", theme::BORDER),
            if let Some((prev_slug, prev_label)) = prev {
                Link { to: Route::StationDetail { factory: factory.clone(), station: prev_slug.clone() },
                    Button { variant: ButtonVariant::Secondary, "\u{2190} {prev_label}" }
                }
            } else {
                span {}
            }
            if let Some((next_slug, next_label)) = next {
                Link { to: Route::StationDetail { factory: factory.clone(), station: next_slug.clone() },
                    Button { variant: ButtonVariant::Secondary, "{next_label} \u{2192}" }
                }
            } else {
                span {}
            }
        }
    }
}

/// A per-phase section of the station detail page: a heading, a note, and each
/// role as an [`ExpandableRole`] card rendering its real markdown instructions.
#[component]
fn PhaseSection(
    heading: String,
    phase: Option<Phase>,
    note: String,
    roles: Vec<crate::factory_view::RoleView>,
    #[props(default = false)] beats: bool,
) -> Element {
    if roles.is_empty() {
        return rsx! {};
    }
    let accent = phase.map(|p| p.hue_var().base).unwrap_or(theme::ACCENT);
    rsx! {
        section { style: "margin:28px 0;",
            h2 {
                style: format!(
                    "font-family:{};font-size:18px;font-weight:700;color:{};margin:0 0 4px;\
                     border-left:3px solid {};padding-left:10px;",
                    tokens::FONT_SANS, theme::TEXT, accent,
                ),
                "{heading}"
            }
            p {
                style: format!(
                    "font-family:{};font-size:13px;color:{};margin:0 0 12px;padding-left:13px;",
                    tokens::FONT_SANS, theme::TEXT_MUTED,
                ),
                "{note}"
            }
            div { style: "display:flex;flex-direction:column;gap:10px;",
                for (i, role) in roles.iter().enumerate() {
                    ExpandableRole {
                        name: role.name.clone(),
                        kind: role.kind,
                        agent_type: Some(role.agent_type.clone()),
                        model: role.model.clone(),
                        beat: if beats { PassBeat::ALL.get(i).copied() } else { None },
                        summary: role.summary.clone(),
                        body_html: Some(role.body_html.clone()),
                    }
                }
            }
        }
    }
}

/// A bordered panel with a mono label, used to frame each diagram block.
#[component]
fn Panel(label: String, children: Element) -> Element {
    let wrap = format!(
        "border:1px solid {};border-radius:10px;padding:16px;margin:16px 0;background:{};",
        theme::BORDER,
        theme::SURFACE_RAISED,
    );
    let label_style = format!(
        "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.08em;\
         color:{};margin-bottom:12px;",
        tokens::FONT_MONO,
        theme::ACCENT,
    );
    rsx! {
        div { style: "{wrap}",
            div { style: "{label_style}", "{label}" }
            {children}
        }
    }
}

/// The "not found" state for a station that does not exist.
#[component]
fn StationMissing(factory: String, message: String) -> Element {
    rsx! {
        SectionHead {
            kicker: "not found".to_string(),
            title: "Station".to_string(),
            lead: Some(message),
        }
        Link { to: Route::FactoryDetail { slug: factory.clone() },
            Button { variant: ButtonVariant::Secondary, "Back to {factory}" }
        }
    }
}

/// A labelled row of role chips. Hidden when empty.
#[component]
fn RoleRow(label: String, roles: Vec<String>) -> Element {
    if roles.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { style: "display:flex;align-items:baseline;gap:8px;margin:4px 0;flex-wrap:wrap;",
            span {
                style: format!(
                    "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;color:{};min-width:78px;",
                    tokens::FONT_MONO, theme::TEXT_FAINT,
                ),
                "{label}"
            }
            for role in roles {
                Badge { tone: Tone::Neutral, "{role}" }
            }
        }
    }
}

/// Empty state when no factories are embedded (defensive — the corpus ships one).
#[component]
fn EmptyState() -> Element {
    rsx! {
        Card {
            p {
                style: format!("font-family:{};color:{};margin:0;", tokens::FONT_SANS, theme::TEXT_MUTED),
                "No factories are embedded in this build."
            }
        }
    }
}

/// Map a station's position to the phase hue used for its accent stripe.
pub fn phase_for_index(index: usize) -> Option<Phase> {
    Phase::ALL.get(index).copied()
}
