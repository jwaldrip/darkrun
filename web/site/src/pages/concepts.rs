//! The concept pages: `/methodology`, `/methodology/:phase`, `/glossary`,
//! `/lifecycles`.
//!
//! `/methodology` is a real explainer — the cost-of-late-discovery ordering, the
//! anti-rework philosophy, the universal slot, and the six phases — built on the
//! `StationFlow` and `PhaseMachine` diagrams over the embedded software factory.
//! `/methodology/:phase` drills into a single phase. The glossary and lifecycles
//! pages render their embedded markdown.

use darkrun_ui::prelude::*;

use crate::content::{self, CONCEPTS};
use crate::factory_view::flow_stations;
use crate::route::Route;
use crate::ui::{PhaseLegend, Prose, SectionHead};

/// The six phase slugs, in canonical order — the source for `/methodology/:phase`
/// routes and the sitemap.
pub const PHASE_SLUGS: [&str; 6] =
    ["spec", "review", "manufacture", "audit", "tests", "checkpoint"];

/// Render a concept page by slug, or a small fallback if it is missing.
fn concept(slug: &str) -> Element {
    match content::find(CONCEPTS, slug) {
        Some(doc) => rsx! { Prose { doc: *doc } },
        None => rsx! {
            article { class: "dr-prose", h1 { "Not found" } p { "This concept page is unavailable." } }
        },
    }
}

/// Load the software factory's stations as flow nodes for the diagrams, falling
/// back to an empty pipeline if the corpus cannot be loaded.
fn factory_flows() -> Vec<FlowStation> {
    darkrun_content::load_validated("software")
        .map(|f| flow_stations(&f))
        .unwrap_or_default()
}

/// `/methodology` — the full explainer: cost-of-late-discovery ordering, the
/// anti-rework philosophy, the universal slot, and the six phases, with the
/// `StationFlow` pipeline and the `PhaseMachine` ring.
#[component]
pub fn Methodology() -> Element {
    let flows = factory_flows();
    rsx! {
        {concept("methodology")}

        // The assembly line, ordered by cost of late discovery.
        div { style: "margin-top:28px;",
            ConceptPanel { label: "the assembly line".to_string(),
                p {
                    style: muted_style(),
                    "The software factory's six stations, left to right, in cost-of-late-discovery \
                     order. Each kills a class of risk; the earlier it sits, the cheaper the defect \
                     it catches."
                }
                div { style: "overflow-x:auto;",
                    StationFlow { stations: flows.clone() }
                }
            }
        }

        // The within-station phase machine.
        div { style: "margin-top:20px;",
            ConceptPanel { label: "the universal slot".to_string(),
                p {
                    style: muted_style(),
                    "Every station runs the same six-phase machine: explore → decompose → \
                     pass-loop (make → challenge → resolve) → review → checkpoint → lock."
                }
                div { style: "display:flex;justify-content:center;",
                    PhaseMachine { active: Some(Phase::Manufacture), active_beat: Some(PassBeat::Challenge), size: 340.0 }
                }
            }
        }

        // The six phases, each a card linking to its detail page.
        div { style: "margin-top:28px;",
            h2 {
                style: format!(
                    "font-family:{};font-size:22px;color:{};margin:0 0 12px;",
                    tokens::FONT_SANS, tokens::TEXT,
                ),
                "The six phases"
            }
            div { class: "dr-grid",
                for phase in Phase::ALL {
                    PhaseCard { phase }
                }
            }
        }

        div { style: "margin-top:24px;", PhaseLegend {} }
    }
}

/// A single phase tile linking to its detail page.
#[component]
fn PhaseCard(phase: Phase) -> Element {
    let hue = phase.hue();
    let slug = phase.name().to_string();
    rsx! {
        Link {
            to: Route::PhaseDetail { phase: slug.clone() },
            style: "text-decoration:none;display:block;",
            Card { accent: Some(hue.base.to_string()),
                div { style: "display:flex;align-items:center;gap:8px;margin-bottom:6px;",
                    span { style: format!("color:{};font-size:14px;", hue.base), "{tokens::GLYPH_ACTIVE}" }
                    span {
                        style: format!(
                            "font-family:{};font-size:16px;font-weight:700;color:{};text-transform:capitalize;",
                            tokens::FONT_SANS, tokens::TEXT,
                        ),
                        "{phase_label(phase)}"
                    }
                }
                p {
                    style: format!("font-family:{};font-size:13px;color:{};margin:0;", tokens::FONT_SANS, tokens::TEXT_MUTED),
                    "{phase_beat(phase)}"
                }
            }
        }
    }
}

/// `/methodology/:phase` — a single phase explained, with the phase machine
/// fixed on it and (for Manufacture) the Make → Challenge → Resolve beats.
#[component]
pub fn PhaseDetail(phase: String) -> Element {
    let Some(p) = Phase::from_name(&phase) else {
        return rsx! {
            SectionHead {
                kicker: "not found".to_string(),
                title: "Phase".to_string(),
                lead: Some(format!("`{phase}` is not one of the six phases.")),
            }
            Link { to: Route::Methodology {},
                Button { variant: ButtonVariant::Secondary, "Back to methodology" }
            }
        };
    };

    let idx = Phase::ALL.iter().position(|x| *x == p).unwrap_or(0);
    let prev = idx.checked_sub(1).map(|i| Phase::ALL[i]);
    let next = Phase::ALL.get(idx + 1).copied();
    let hue = p.hue();
    let is_manufacture = p == Phase::Manufacture;

    rsx! {
        div { style: "margin-bottom:8px;",
            Link { to: Route::Methodology {},
                span {
                    style: format!("font-family:{};font-size:13px;color:{};", tokens::FONT_MONO, tokens::ACCENT),
                    "\u{2190} methodology"
                }
            }
        }
        SectionHead {
            kicker: format!("phase {} / 6", idx + 1),
            title: phase_label(p).to_string(),
            lead: Some(phase_beat(p).to_string()),
        }
        div { style: "margin-bottom:16px;",
            span {
                style: format!(
                    "display:inline-flex;align-items:center;gap:6px;font-family:{};font-size:12px;\
                     color:{};border:1px solid {};border-radius:999px;padding:4px 10px;",
                    tokens::FONT_MONO, hue.base, tokens::BORDER,
                ),
                span { "{tokens::GLYPH_ACTIVE}" }
                "{p.name()}"
            }
        }

        ConceptPanel { label: "where it sits".to_string(),
            div { style: "display:flex;justify-content:center;",
                PhaseMachine {
                    active: Some(p),
                    active_beat: if is_manufacture { Some(PassBeat::Make) } else { None },
                    size: 340.0,
                }
            }
        }

        article { class: "dr-prose", style: "margin-top:20px;",
            p { "{phase_explainer(p)}" }
        }

        if is_manufacture {
            ConceptPanel { label: "the pass loop".to_string(),
                div { style: "display:flex;flex-direction:column;gap:8px;",
                    for beat in PassBeat::ALL {
                        div { style: format!(
                            "display:flex;align-items:baseline;gap:10px;font-family:{};",
                            tokens::FONT_SANS,
                        ),
                            Badge { tone: Tone::Accent, filled: true, "{beat.label()}" }
                            span {
                                style: format!("font-size:13px;color:{};", tokens::TEXT_MUTED),
                                "{beat.beat()}"
                            }
                        }
                    }
                }
            }
        }

        // Prev/next phase nav.
        div { style: format!("margin-top:32px;display:flex;justify-content:space-between;gap:12px;border-top:1px solid {};padding-top:16px;", tokens::BORDER),
            if let Some(prev) = prev {
                Link { to: Route::PhaseDetail { phase: prev.name().to_string() },
                    Button { variant: ButtonVariant::Secondary, "\u{2190} {phase_label(prev)}" }
                }
            } else {
                span {}
            }
            if let Some(next) = next {
                Link { to: Route::PhaseDetail { phase: next.name().to_string() },
                    Button { variant: ButtonVariant::Secondary, "{phase_label(next)} \u{2192}" }
                }
            } else {
                span {}
            }
        }
    }
}

/// `/glossary` — the vocabulary reference.
#[component]
pub fn Glossary() -> Element {
    concept("glossary")
}

/// `/lifecycles` — the path work travels through a factory.
#[component]
pub fn Lifecycles() -> Element {
    concept("lifecycles")
}

/// A bordered concept panel with a mono label.
#[component]
fn ConceptPanel(label: String, children: Element) -> Element {
    let wrap = format!(
        "border:1px solid {};border-radius:10px;padding:16px;margin:8px 0;background:{};",
        tokens::BORDER,
        tokens::SURFACE_RAISED,
    );
    let label_style = format!(
        "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.08em;\
         color:{};margin-bottom:12px;",
        tokens::FONT_MONO,
        tokens::ACCENT,
    );
    rsx! {
        div { style: "{wrap}",
            div { style: "{label_style}", "{label}" }
            {children}
        }
    }
}

fn muted_style() -> String {
    format!(
        "font-family:{};font-size:13px;color:{};margin:0 0 12px;",
        tokens::FONT_SANS,
        tokens::TEXT_MUTED,
    )
}

/// A paragraph-length explainer for each phase. Keeps the methodology vocabulary
/// load-bearing: each phase maps onto the universal slot.
pub fn phase_explainer(phase: Phase) -> &'static str {
    match phase {
        Phase::Spec => "Specify is where a station explores the context it needs and decomposes the \
             work into Units with testable completion criteria. Nothing is produced yet — the goal \
             is to know exactly what \"done\" means before spending anything making it.",
        Phase::Review => "Review challenges the spec before any output exists. It is cheaper to \
             reject a bad scope here than to discover it after the work is built, so the spec is \
             attacked for gaps and contradictions first.",
        Phase::Manufacture => "Manufacture is the pass loop. Each Unit runs Passes, and one Pass is \
             the three-beat worker sequence Make → Challenge → Resolve: produce a candidate, attack \
             it for its weakest seam, then fix what the attack surfaced. This is where output is \
             actually made.",
        Phase::Audit => "Audit verifies the produced output against the spec — independent of the \
             workers that made it. A reviewer that did not write the code checks that it matches \
             what was specified, catching the defects the maker is blind to.",
        Phase::Tests => "Prove runs the quality gates: the tests, the checks, the evidence that the \
             work holds. A station cannot lock its artifact until its gates pass, so this is the \
             station's proof obligation.",
        Phase::Checkpoint => "Checkpoint fires the station's gate — auto, ask, external, or await — \
             and then locks the durable artifact. Passing the gate advances the line; failing it \
             routes the rework back as drift. Once locked, downstream stations may not reopen it.",
    }
}
