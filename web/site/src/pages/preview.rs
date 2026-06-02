//! `/preview` — a static fixture/preview of the interactive session views.
//!
//! Renders a representative VISUAL QUESTION and DESIGN DIRECTION built from the
//! real `darkrun-api` session payload types so the new `darkrun-ui`
//! [`QuestionView`] and [`DirectionView`] components can be viewed (and
//! screenshotted) in the browser without a running engine. The page is a
//! preview only — there is no live feed and the submit/annotation handlers are
//! intentionally unwired; a banner makes that explicit.

use darkrun_api::proof::{AuditResult, BenchProof, Proof, Surface, WebProof};
use darkrun_api::session::{
    DirectionAnnotations, DirectionArchetype, DirectionPin, DirectionSessionPayload, QuestionOption,
    QuestionSessionPayload, ViewArtifact, ViewArtifactKind, ViewMode, ViewScope,
    ViewSessionPayload, ViewStatus,
};
use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::pages::review::ScaffoldNote;
use crate::ui::SectionHead;

/// `/preview` — the session-view fixture gallery.
#[component]
pub fn Preview() -> Element {
    let question = sample_question();
    let direction = sample_direction();

    // Map the wire payloads into the darkrun-ui prop data (the same boundary the
    // desktop app crosses), then render them read-only.
    let q_options: Vec<OptionCard> = question
        .options
        .iter()
        .map(|o| OptionCard {
            id: o.id.clone(),
            label: o.label.clone(),
            image_url: o.image_url.clone(),
            description: o.description.clone(),
        })
        .collect();
    let q_selected = question
        .answer
        .as_ref()
        .map(|a| a.selected.clone())
        .unwrap_or_default();

    let d_archetypes: Vec<ArchetypeCard> = direction
        .archetypes
        .iter()
        .map(|a| ArchetypeCard {
            id: a.id.clone(),
            label: a.label.clone(),
            image_url: a.image_url.clone(),
            description: a.description.clone(),
        })
        .collect();
    let d_pins: Vec<PinPoint> = direction
        .annotations
        .as_ref()
        .map(|ann| {
            ann.pins
                .iter()
                .map(|p| PinPoint::new(p.x, p.y, p.note.clone()))
                .collect()
        })
        .unwrap_or_default();
    let d_comments = direction
        .annotations
        .as_ref()
        .map(|ann| ann.comments.clone())
        .unwrap_or_default();

    // Map the view fixture's artifacts into the browser prop data, and the two
    // proof fixtures into the panel's display projection.
    let view = sample_view();
    let view_artifacts: Vec<ArtifactEntry> = view.artifacts.iter().map(view_artifact_entry).collect();
    let view_focus = view.artifact.clone();

    let web_proof = proof_to_view(&sample_web_proof());
    let bench_proof = proof_to_view(&sample_bench_proof());

    rsx! {
        SectionHead {
            kicker: "fixture".to_string(),
            title: "Session preview".to_string(),
            lead: Some(
                "A static preview of the interactive session views the agent poses mid-run: a \
                 visual question, a design direction, the output artifact browser, and the \
                 objective-evidence proof panel, built from the real darkrun-api payload types. \
                 Preview only — no live feed is attached."
                    .to_string(),
            ),
        }

        ScaffoldNote {
            text: "Fixture: representative Question / Direction / View payloads + a web-vitals \
                   and a bench Proof, rendered read-only. Submit and annotation actions are \
                   unwired here."
                .to_string(),
        }

        div { style: "display:flex;flex-direction:column;gap:32px;margin-top:8px;",
            section {
                "data-fixture": "question",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, theme::TEXT,
                    ),
                    "Visual question"
                }
                QuestionView {
                    prompt: question.prompt.clone(),
                    context: question.context.clone(),
                    title: question.title.clone(),
                    options: q_options,
                    multi_select: question.multi_select,
                    image_urls: question.image_urls.clone(),
                    selected: q_selected,
                    answered: false,
                }
            }

            section {
                "data-fixture": "direction",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, theme::TEXT,
                    ),
                    "Design direction"
                }
                DirectionView {
                    prompt: direction.prompt.clone(),
                    context: direction.context.clone(),
                    title: direction.title.clone(),
                    archetypes: d_archetypes,
                    chosen: direction.chosen_archetype.clone(),
                    pins: d_pins,
                    comments: d_comments,
                    decided: false,
                }
            }

            section {
                "data-fixture": "view",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, theme::TEXT,
                    ),
                    "Output artifact browser"
                }
                ViewArtifacts {
                    run_slug: Some(view.run_slug.clone()),
                    station: view.station.clone(),
                    artifacts: view_artifacts,
                    focused: view_focus,
                }
            }

            section {
                "data-fixture": "proof-web",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, theme::TEXT,
                    ),
                    "Proof — web vitals (visual surface)"
                }
                ProofPanel { proof: web_proof }
            }

            section {
                "data-fixture": "proof-bench",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, theme::TEXT,
                    ),
                    "Proof — benchmarks (bench surface)"
                }
                ProofPanel { proof: bench_proof }
            }
        }
    }
}

/// The canonical web-vital display order in the preview panel.
const VITAL_ORDER: [&str; 5] = ["lcp", "fcp", "ttfb", "inp", "cls"];

/// Map a wire [`ViewArtifactKind`] onto the UI [`ArtifactKind`].
fn view_artifact_kind(k: ViewArtifactKind) -> ArtifactKind {
    match k {
        ViewArtifactKind::File => ArtifactKind::File,
        ViewArtifactKind::Image => ArtifactKind::Image,
        ViewArtifactKind::Screenshot => ArtifactKind::Screenshot,
        ViewArtifactKind::Markdown => ArtifactKind::Markdown,
        ViewArtifactKind::Json => ArtifactKind::Json,
    }
}

/// Map a wire [`ViewArtifact`] onto a UI [`ArtifactEntry`] for the browser.
fn view_artifact_entry(a: &ViewArtifact) -> ArtifactEntry {
    ArtifactEntry {
        id: a.id.clone(),
        path: a.path.clone(),
        kind: view_artifact_kind(a.kind),
        label: a.label.clone(),
        thumbnail_url: a.thumbnail_url.clone(),
        url: a.url.clone(),
        body: None,
    }
}

/// Map a wire [`Proof`] onto the [`ProofView`] the panel renders, pre-formatting
/// and classifying every number — the same boundary the desktop app crosses.
pub fn proof_to_view(proof: &Proof) -> ProofView {
    let kind = if proof.surface.is_visual() {
        ProofMetricKind::Web
    } else if proof.surface.is_bench() {
        ProofMetricKind::Bench
    } else {
        ProofMetricKind::Terminal
    };

    let mut vitals = Vec::new();
    let mut audits = Vec::new();
    let mut screenshot_url = None;
    if let Some(web) = &proof.web {
        for key in VITAL_ORDER {
            if let Some(value) = web.vitals.get(key) {
                vitals.push(VitalMetric {
                    key: key.to_string(),
                    value: *value,
                    display: format_vital(key, *value),
                    verdict: classify_vital(key, *value),
                });
            }
        }
        audits = web
            .audits
            .iter()
            .map(|a| AuditRow {
                name: a.name.clone(),
                value: a.value.clone(),
                pass: a.pass,
            })
            .collect();
        screenshot_url = web.screenshot_url.clone();
    }

    let mut bench = Vec::new();
    if let Some(b) = &proof.bench {
        if let Some(v) = b.p50 {
            bench.push(BenchStat { label: "p50".to_string(), display: format_latency_ms(v) });
        }
        if let Some(v) = b.p95 {
            bench.push(BenchStat { label: "p95".to_string(), display: format_latency_ms(v) });
        }
        if let Some(v) = b.p99 {
            bench.push(BenchStat { label: "p99".to_string(), display: format_latency_ms(v) });
        }
        if let Some(v) = b.throughput {
            bench.push(BenchStat {
                label: "throughput".to_string(),
                display: format_throughput(v),
            });
        }
        if let Some(n) = b.samples {
            bench.push(BenchStat { label: "samples".to_string(), display: format_samples(n) });
        }
    }

    ProofView {
        surface: proof.surface.as_str().to_string(),
        kind,
        vitals,
        audits,
        screenshot_url,
        bench,
        block_matches_surface: proof.block_matches_surface(),
    }
}

/// A representative visual-question payload built from the real API types.
pub fn sample_question() -> QuestionSessionPayload {
    QuestionSessionPayload {
        session_id: "preview-question".to_string(),
        status: darkrun_api::common::SessionStatus::Pending,
        title: Some("dashboard hero treatment".to_string()),
        prompt: "Which hero layout reads best for the operator dashboard?".to_string(),
        context: Some(
            "Generated three options from the brand tokens. Pick the one that best frames the \
             live run status."
                .to_string(),
        ),
        options: vec![
            QuestionOption {
                id: "split".to_string(),
                label: "Split status rail".to_string(),
                image_url: None,
                description: Some("Run status pinned left, output stream right.".to_string()),
            },
            QuestionOption {
                id: "stacked".to_string(),
                label: "Stacked timeline".to_string(),
                image_url: None,
                description: Some("Phases as a vertical timeline with inline previews.".to_string()),
            },
            QuestionOption {
                id: "grid".to_string(),
                label: "Station grid".to_string(),
                image_url: None,
                description: Some("Every station as a card in a responsive grid.".to_string()),
            },
        ],
        multi_select: false,
        answer: None,
        image_urls: Vec::new(),
    }
}

/// A representative design-direction payload built from the real API types,
/// including two pre-placed annotation pins on the chosen archetype.
pub fn sample_direction() -> DirectionSessionPayload {
    DirectionSessionPayload {
        session_id: "preview-direction".to_string(),
        status: darkrun_api::common::SessionStatus::Pending,
        title: Some("control surface aesthetic".to_string()),
        run_slug: Some("operator-console".to_string()),
        prompt: "Pick the design direction for the operator console.".to_string(),
        context: Some(
            "Three archetypes generated from the dark brand. Choose one and drop pins where you \
             want changes."
                .to_string(),
        ),
        archetypes: vec![
            DirectionArchetype {
                id: "instrument".to_string(),
                label: "Instrument panel".to_string(),
                image_url: String::new(),
                description: "Dense, telemetry-first: gauges, sparklines, monospace readouts."
                    .to_string(),
            },
            DirectionArchetype {
                id: "editorial".to_string(),
                label: "Editorial calm".to_string(),
                image_url: String::new(),
                description: "Generous whitespace, one focal metric, quiet supporting detail."
                    .to_string(),
            },
            DirectionArchetype {
                id: "terminal".to_string(),
                label: "Terminal".to_string(),
                image_url: String::new(),
                description: "Pure mono, log-stream forward, minimal chrome.".to_string(),
            },
        ],
        chosen_archetype: Some("instrument".to_string()),
        annotations: Some(DirectionAnnotations {
            pins: vec![
                DirectionPin {
                    x: 0.25,
                    y: 0.3,
                    note: "tighten the header density".to_string(),
                },
                DirectionPin {
                    x: 0.7,
                    y: 0.62,
                    note: "more contrast on the active gauge".to_string(),
                },
            ],
            screenshot: None,
            comments: vec!["Lean into the instrument metaphor across the whole console.".to_string()],
        }),
    }
}

/// A representative view-session payload built from the real API types — an
/// artifact browser over a run's outputs, with a screenshot focused.
pub fn sample_view() -> ViewSessionPayload {
    ViewSessionPayload {
        session_id: "preview-view".to_string(),
        status: ViewStatus::Open,
        run_slug: "operator-console".to_string(),
        scope: ViewScope::Run,
        artifacts: vec![
            ViewArtifact {
                id: "home-shot".to_string(),
                path: "build/prove/home.png".to_string(),
                kind: ViewArtifactKind::Screenshot,
                label: "Home screenshot".to_string(),
                thumbnail_url: None,
                url: None,
            },
            ViewArtifact {
                id: "spec".to_string(),
                path: "build/spec.md".to_string(),
                kind: ViewArtifactKind::Markdown,
                label: "Console spec".to_string(),
                thumbnail_url: None,
                url: None,
            },
            ViewArtifact {
                id: "vitals".to_string(),
                path: "build/prove/vitals.json".to_string(),
                kind: ViewArtifactKind::Json,
                label: "Web vitals".to_string(),
                thumbnail_url: None,
                url: None,
            },
            ViewArtifact {
                id: "bundle".to_string(),
                path: "build/console.tar.gz".to_string(),
                kind: ViewArtifactKind::File,
                label: "Console bundle".to_string(),
                thumbnail_url: None,
                url: None,
            },
        ],
        factory: Some("software-factory".to_string()),
        station: Some("prove".to_string()),
        artifact: Some("spec".to_string()),
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    }
}

/// A representative VISUAL-surface proof: web vitals + a11y audits + a captured
/// screenshot — the NUMBERS a headless browser measures.
pub fn sample_web_proof() -> Proof {
    let mut web = WebProof::default();
    web.vitals.insert("lcp".to_string(), 1850.0);
    web.vitals.insert("fcp".to_string(), 1200.0);
    web.vitals.insert("ttfb".to_string(), 620.0);
    web.vitals.insert("inp".to_string(), 180.0);
    web.vitals.insert("cls".to_string(), 0.04);
    web.audits = vec![
        AuditResult { name: "contrast".to_string(), value: "5.1:1".to_string(), pass: true },
        AuditResult { name: "touch-target".to_string(), value: "44px".to_string(), pass: true },
        AuditResult {
            name: "reduced-motion".to_string(),
            value: "honored".to_string(),
            pass: true,
        },
        AuditResult { name: "alt-text".to_string(), value: "2 missing".to_string(), pass: false },
    ];
    Proof::web(Surface::WebUi, web)
}

/// A representative BENCH-surface proof: latency percentiles + throughput — the
/// NUMBERS criterion + a load harness measure.
pub fn sample_bench_proof() -> Proof {
    Proof::bench(
        Surface::Library,
        BenchProof {
            p50: Some(0.42),
            p95: Some(1.15),
            p99: Some(2.30),
            throughput: Some(48_500.0),
            samples: Some(100_000),
        },
    )
}
