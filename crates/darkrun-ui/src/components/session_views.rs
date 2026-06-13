//! The interactive session views the agent poses mid-run: [`QuestionView`]
//! (pick among image-backed options), [`DirectionView`] (pick a design archetype
//! and annotate it with pins + comments), and [`PickerView`] (a plain option
//! list/grid select).
//!
//! Each view takes plain, `PartialEq` prop data — the caller maps the
//! `darkrun-api` wire payloads into these at the boundary, exactly as the rest of
//! the design system does. The selection/pin math lives in
//! [`crate::selection`]; these components are the thin dark-themed shell over it.
//!
//! A missing image never blanks a card: [`image_or_placeholder`] paints a
//! labelled placeholder surface instead.

use dioxus::prelude::*;

use crate::components::primitives::{Badge, Button, ButtonVariant, Card};
use crate::kinds::Tone;
use crate::selection::PinPoint;
use crate::tokens;

// ===========================================================================
// Shared option/archetype prop data
// ===========================================================================

/// One selectable option in a [`QuestionView`] — the design-system mirror of the
/// wire `QuestionOption`, carrying its own (optional) image and description.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionCard {
    /// Stable id echoed back in the answer.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional image url (a generated mockup / design option). Dark-theme
    /// variant when `image_url_light` is also set.
    pub image_url: Option<String>,
    /// Optional light-theme variant of `image_url`. When present, the card shows
    /// whichever image matches the active theme.
    pub image_url_light: Option<String>,
    /// Optional longer description.
    pub description: Option<String>,
}

impl OptionCard {
    /// Construct a label-only option.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            image_url: None,
            image_url_light: None,
            description: None,
        }
    }

    /// Attach an image url (the dark/default variant).
    pub fn with_image(mut self, url: impl Into<String>) -> Self {
        self.image_url = Some(url.into());
        self
    }

    /// Attach the light-theme variant of the image.
    pub fn with_image_light(mut self, url: impl Into<String>) -> Self {
        self.image_url_light = Some(url.into());
        self
    }

    /// Attach a description.
    pub fn with_description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
}

/// One design archetype in a [`DirectionView`] — the design-system mirror of the
/// wire `DirectionArchetype` (image + description always present).
#[derive(Debug, Clone, PartialEq)]
pub struct ArchetypeCard {
    /// Stable id echoed back as the chosen direction.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Generated preview-image url (dark/default variant when a light one is set).
    pub image_url: String,
    /// Optional light-theme variant of `image_url`. When present, the card shows
    /// whichever preview matches the active theme.
    pub image_url_light: Option<String>,
    /// Description of the design direction.
    pub description: String,
}

impl ArchetypeCard {
    /// Construct an archetype card.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        image_url: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            image_url: image_url.into(),
            image_url_light: None,
            description: description.into(),
        }
    }

    /// Attach the light-theme variant of the preview image.
    pub fn with_image_light(mut self, url: impl Into<String>) -> Self {
        self.image_url_light = Some(url.into());
        self
    }
}

/// One plain option in a [`PickerView`] — id + label + optional descriptions.
#[derive(Debug, Clone, PartialEq)]
pub struct PickerItem {
    /// Stable id echoed back on selection.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional description line.
    pub description: Option<String>,
    /// Optional secondary (right-aligned) text.
    pub secondary: Option<String>,
}

impl PickerItem {
    /// Construct a label-only picker item.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            secondary: None,
        }
    }

    /// Attach a description line.
    pub fn with_description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// Attach secondary (right-aligned) text.
    pub fn with_secondary(mut self, text: impl Into<String>) -> Self {
        self.secondary = Some(text.into());
        self
    }
}

// ===========================================================================
// Shared rendering helpers
// ===========================================================================

/// Render an `<img>` for `url`, or a labelled placeholder surface when the url is
/// absent or blank. `aspect` is a CSS `aspect-ratio` (e.g. `"4 / 3"`).
fn image_or_placeholder(url: Option<&str>, alt: &str, aspect: &str) -> Element {
    themed_image_or_placeholder(url, None, alt, aspect)
}

/// Theme-aware variant of [`image_or_placeholder`]: when a `light` url is given,
/// render *both* the dark and light images and let the `.dr-themed-*` CSS (in
/// [`crate::tokens::THEME_CSS`]) show the one matching the active theme. With no
/// `light` url this collapses to a single theme-neutral image — same render path
/// as [`image_or_placeholder`].
fn themed_image_or_placeholder(
    url: Option<&str>,
    light: Option<&str>,
    alt: &str,
    aspect: &str,
) -> Element {
    let frame = format!(
        "width:100%;aspect-ratio:{aspect};border-radius:6px;overflow:hidden;\
         background:{base};border:1px solid {border};",
        aspect = aspect,
        base = tokens::var::SURFACE_BASE,
        border = tokens::var::BORDER,
    );
    let dark = url.map(str::trim).filter(|u| !u.is_empty());
    let light = light.map(str::trim).filter(|u| !u.is_empty());
    match (dark, light) {
        // Multi-theme: render both; CSS shows the variant matching the theme.
        (Some(d), Some(l)) => rsx! {
            img {
                class: "dr-themed-dark",
                style: "{frame}object-fit:cover;",
                src: "{d}",
                alt: "{alt}",
                loading: "lazy",
            }
            img {
                class: "dr-themed-light",
                style: "{frame}object-fit:cover;",
                src: "{l}",
                alt: "{alt}",
                loading: "lazy",
            }
        },
        // Single image (either variant present alone) — theme-neutral.
        (Some(u), None) | (None, Some(u)) => rsx! {
            img {
                style: "{frame}display:block;object-fit:cover;",
                src: "{u}",
                alt: "{alt}",
                loading: "lazy",
            }
        },
        (None, None) => {
            let ph = format!(
                "{frame}display:flex;align-items:center;justify-content:center;\
                 font-family:{mono};font-size:11px;color:{faint};\
                 background:repeating-linear-gradient(45deg,{base},{base} 10px,{raised} 10px,{raised} 20px);",
                mono = tokens::FONT_MONO,
                faint = tokens::var::TEXT_FAINT,
                base = tokens::var::SURFACE_BASE,
                raised = tokens::var::SURFACE_RAISED,
            );
            rsx! {
                div { class: "dr-img-placeholder", style: "{ph}", role: "img", "aria-label": "{alt}",
                    "no preview"
                }
            }
        }
    }
}

/// A small section heading shared by the views.
fn heading(text: &str) -> Element {
    let style = format!(
        "margin:0;font-family:{sans};font-size:13px;font-weight:700;color:{text};\
         text-transform:uppercase;letter-spacing:0.04em;",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    rsx! { h2 { style: "{style}", "{text}" } }
}

/// Scoped CSS for the rendered-markdown blocks (prompt context + option
/// descriptions): readable list/paragraph spacing and an inline-code chip. Kept
/// local to the views so the markdown subset styles consistently wherever it
/// renders.
const MD_CSS: &str = "\
.dr-md .dr-md-p{margin:0 0 8px;line-height:1.5;}\
.dr-md .dr-md-p:last-child{margin-bottom:0;}\
.dr-md .dr-md-ul{margin:6px 0;padding-left:18px;display:flex;flex-direction:column;gap:5px;}\
.dr-md .dr-md-ul li{line-height:1.45;}\
.dr-md .dr-md-code{font-family:var(--dr-font-mono);font-size:0.92em;\
background:var(--dr-surface-base);border:1px solid var(--dr-border);\
border-radius:4px;padding:0.5px 5px;}\
.dr-md strong{font-weight:700;color:var(--dr-text);}";

/// The prompt / context block shared by question and direction. The prompt is a
/// single bold line; the context renders its markdown (bullets, bold, inline
/// code) so an agent-authored preamble reads as formatted prose rather than raw
/// `**` / `-` / backticks.
fn prompt_block(prompt: &str, context: Option<&str>) -> Element {
    let prompt_style = format!(
        "margin:0;font-family:{sans};font-size:16px;font-weight:600;color:{text};line-height:1.35;",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let ctx_style = format!(
        "margin:8px 0 0;font-family:{sans};font-size:13px;color:{muted};",
        sans = tokens::FONT_SANS,
        muted = tokens::var::TEXT_MUTED,
    );
    let ctx_html = context
        .filter(|c| !c.is_empty())
        .map(crate::markdown::to_html);
    rsx! {
        div {
            style { "{MD_CSS}" }
            if !prompt.is_empty() {
                p { style: "{prompt_style}", "{prompt}" }
            }
            if let Some(html) = ctx_html {
                div { class: "dr-md", style: "{ctx_style}", dangerous_inner_html: "{html}" }
            }
        }
    }
}

// ===========================================================================
// QuestionView
// ===========================================================================

/// A VISUAL QUESTION: a prompt plus a grid of option cards (image + label +
/// description) the operator picks among, single- or multi-select, with a submit
/// bar.
///
/// Selection state is owned by the caller (`selected` is the current set of
/// chosen ids); each card press calls `on_toggle` with its id. `on_submit` fires
/// the submit bar. The component is stateless beyond its props so it composes
/// cleanly with a parent that owns the [`crate::selection::SelectionModel`].
#[component]
pub fn QuestionView(
    /// The question prompt.
    prompt: String,
    /// Optional context preamble.
    #[props(default)]
    context: Option<String>,
    /// Optional title chip.
    #[props(default)]
    title: Option<String>,
    /// The selectable option cards.
    options: Vec<OptionCard>,
    /// Whether more than one may be chosen (drives the helper text + chip).
    #[props(default = false)]
    multi_select: bool,
    /// The currently-selected option ids.
    #[props(default)]
    selected: Vec<String>,
    /// Optional reference image urls (distinct from per-option images).
    #[props(default)]
    image_urls: Vec<String>,
    /// Whether the question is already answered / read-only.
    #[props(default = false)]
    answered: bool,
    /// Toggle handler — called with the pressed option id.
    #[props(default)]
    on_toggle: Option<EventHandler<String>>,
    /// Submit handler.
    #[props(default)]
    on_submit: Option<EventHandler<MouseEvent>>,
) -> Element {
    let mode_label = if multi_select { "select any" } else { "select one" };
    // Only show the image slots when at least one option actually carries a
    // mockup; a purely textual question (no mockups) renders clean text cards
    // instead of a wall of "no preview" placeholders. Text cards are also
    // narrower, so the grid packs more per row.
    let show_mockups = options
        .iter()
        .any(|o| o.image_url.is_some() || o.image_url_light.is_some());
    let min = if show_mockups { "200px" } else { "240px" };
    let grid = format!(
        "display:grid;grid-template-columns:repeat(auto-fill,minmax({min},1fr));\
         gap:12px;margin-top:14px;",
    );
    let has_selection = !selected.is_empty();

    rsx! {
        Card {
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:4px;",
                Badge { tone: Tone::Info, "question" }
                if multi_select {
                    Badge { tone: Tone::Neutral, "multi" }
                }
                if let Some(t) = title.clone() {
                    span {
                        style: format!(
                            "font-family:{};font-size:13px;color:{};",
                            tokens::FONT_MONO, tokens::var::TEXT_MUTED,
                        ),
                        "{t}"
                    }
                }
            }
            {prompt_block(&prompt, context.as_deref())}

            // Reference images, if any, above the options.
            if !image_urls.is_empty() {
                div { style: "display:flex;gap:8px;flex-wrap:wrap;margin-top:12px;",
                    for (i, url) in image_urls.iter().enumerate() {
                        div { style: "width:140px;",
                            {image_or_placeholder(Some(url), &format!("reference {}", i + 1), "16 / 10")}
                        }
                    }
                }
            }

            div { style: "{grid}",
                for opt in options.iter() {
                    {option_card(opt, selected.iter().any(|s| s == &opt.id), answered, show_mockups, on_toggle)}
                }
            }

            {submit_bar(
                &format!("{} selected", selected.len()),
                mode_label,
                has_selection && !answered,
                answered,
                if answered { "answered" } else { "Submit answer" },
                on_submit,
            )}
        }
    }
}

/// One option card. Selected cards take an accent border + check chip. The
/// image slot renders only when `show_mockups` is set (the question carries at
/// least one mockup); otherwise the card is text-only. The description renders
/// its markdown subset.
fn option_card(
    opt: &OptionCard,
    selected: bool,
    answered: bool,
    show_mockups: bool,
    on_toggle: Option<EventHandler<String>>,
) -> Element {
    let border = if selected { tokens::var::ACCENT } else { tokens::var::BORDER };
    let ring = if selected {
        format!("box-shadow:0 0 0 1px {};", tokens::var::ACCENT)
    } else {
        String::new()
    };
    let cursor = if answered { "default" } else { "pointer" };
    let card = format!(
        "display:flex;flex-direction:column;gap:8px;padding:10px;border-radius:8px;\
         background:{surface};border:1px solid {border};{ring}cursor:{cursor};\
         text-align:left;width:100%;color:{text};transition:border-color .12s ease;",
        surface = tokens::var::SURFACE_RAISED,
        border = border,
        text = tokens::var::TEXT,
    );
    let label_style = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:8px;\
         font-family:{sans};font-size:13px;font-weight:600;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let desc_style = format!(
        "margin:0;font-family:{sans};font-size:12px;color:{muted};line-height:1.4;",
        sans = tokens::FONT_SANS,
        muted = tokens::var::TEXT_MUTED,
    );
    let id = opt.id.clone();
    rsx! {
        button {
            class: "dr-option-card",
            style: "{card}",
            "data-option-id": "{opt.id}",
            "data-selected": "{selected}",
            "aria-pressed": "{selected}",
            disabled: answered,
            onclick: move |_| {
                if !answered {
                    if let Some(h) = &on_toggle {
                        h.call(id.clone());
                    }
                }
            },
            if show_mockups {
                {themed_image_or_placeholder(opt.image_url.as_deref(), opt.image_url_light.as_deref(), &opt.label, "4 / 3")}
            }
            div { style: "{label_style}",
                span { "{opt.label}" }
                if selected {
                    Badge { tone: Tone::Accent, filled: true, "picked" }
                }
            }
            if let Some(d) = opt.description.clone() {
                if !d.is_empty() {
                    div {
                        class: "dr-md",
                        style: "{desc_style}",
                        dangerous_inner_html: crate::markdown::to_html(&d),
                    }
                }
            }
        }
    }
}

// ===========================================================================
// DirectionView
// ===========================================================================

/// A DESIGN DIRECTION: a prompt plus design-archetype cards. Selecting an
/// archetype reveals an annotation layer over its preview — clickable pins and a
/// comment box — then a submit bar records the direction.
///
/// State is owned by the caller: `chosen` is the selected archetype id, `pins`
/// the current pin set over its preview, `comments` the current comment lines.
/// `on_choose` selects an archetype; `on_place_pin` fires with the pixel offset
/// of a click over the preview (the caller normalizes via
/// [`crate::selection::place_pin`]); `on_submit` records the direction.
#[component]
pub fn DirectionView(
    /// The direction prompt.
    prompt: String,
    /// Optional context preamble.
    #[props(default)]
    context: Option<String>,
    /// Optional title chip.
    #[props(default)]
    title: Option<String>,
    /// The design archetypes to choose between.
    archetypes: Vec<ArchetypeCard>,
    /// The currently-chosen archetype id, if any.
    #[props(default)]
    chosen: Option<String>,
    /// Pins placed over the chosen archetype's preview.
    #[props(default)]
    pins: Vec<PinPoint>,
    /// Comment lines on the chosen direction.
    #[props(default)]
    comments: Vec<String>,
    /// Whether the direction is already decided / read-only.
    #[props(default = false)]
    decided: bool,
    /// Choose-archetype handler — called with the archetype id.
    #[props(default)]
    on_choose: Option<EventHandler<String>>,
    /// Pin-placement handler — called with `(offset_x, offset_y, width, height)`
    /// in pixels relative to the preview box.
    #[props(default)]
    on_place_pin: Option<EventHandler<(f64, f64, f64, f64)>>,
    /// Comment-submit handler — called with the new comment text.
    #[props(default)]
    on_comment: Option<EventHandler<String>>,
    /// Submit handler.
    #[props(default)]
    on_submit: Option<EventHandler<MouseEvent>>,
) -> Element {
    let grid = "display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,1fr));\
                gap:12px;margin-top:14px;";
    let chosen_card = chosen
        .as_ref()
        .and_then(|id| archetypes.iter().find(|a| &a.id == id))
        .cloned();

    rsx! {
        Card {
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:4px;",
                Badge { tone: Tone::Info, "direction" }
                if let Some(t) = title.clone() {
                    span {
                        style: format!(
                            "font-family:{};font-size:13px;color:{};",
                            tokens::FONT_MONO, tokens::var::TEXT_MUTED,
                        ),
                        "{t}"
                    }
                }
            }
            {prompt_block(&prompt, context.as_deref())}

            div { style: "{grid}",
                for arch in archetypes.iter() {
                    {archetype_card(arch, chosen.as_deref() == Some(&arch.id), decided, on_choose)}
                }
            }

            // Annotation layer over the chosen archetype.
            if let Some(card) = chosen_card {
                {annotation_layer(&card, &pins, &comments, decided, on_place_pin, on_comment)}
            }

            {submit_bar(
                &match &chosen {
                    Some(id) => format!("chosen: {id}"),
                    None => "no archetype chosen".to_string(),
                },
                "pick a direction",
                chosen.is_some() && !decided,
                decided,
                if decided { "decided" } else { "Record direction" },
                on_submit,
            )}
        }
    }
}

/// One archetype card. The chosen card takes an accent border + chip.
fn archetype_card(
    arch: &ArchetypeCard,
    chosen: bool,
    decided: bool,
    on_choose: Option<EventHandler<String>>,
) -> Element {
    let border = if chosen { tokens::var::ACCENT } else { tokens::var::BORDER };
    let ring = if chosen {
        format!("box-shadow:0 0 0 1px {};", tokens::var::ACCENT)
    } else {
        String::new()
    };
    let cursor = if decided { "default" } else { "pointer" };
    let card = format!(
        "display:flex;flex-direction:column;gap:8px;padding:10px;border-radius:8px;\
         background:{surface};border:1px solid {border};{ring}cursor:{cursor};\
         text-align:left;width:100%;color:{text};transition:border-color .12s ease;",
        surface = tokens::var::SURFACE_RAISED,
        border = border,
        text = tokens::var::TEXT,
    );
    let label_style = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:8px;\
         font-family:{sans};font-size:13px;font-weight:600;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let desc_style = format!(
        "margin:0;font-family:{sans};font-size:12px;color:{muted};line-height:1.4;",
        sans = tokens::FONT_SANS,
        muted = tokens::var::TEXT_MUTED,
    );
    let id = arch.id.clone();
    rsx! {
        button {
            class: "dr-archetype-card",
            style: "{card}",
            "data-archetype-id": "{arch.id}",
            "data-chosen": "{chosen}",
            "aria-pressed": "{chosen}",
            disabled: decided,
            onclick: move |_| {
                if !decided {
                    if let Some(h) = &on_choose {
                        h.call(id.clone());
                    }
                }
            },
            {themed_image_or_placeholder(Some(&arch.image_url), arch.image_url_light.as_deref(), &arch.label, "4 / 3")}
            div { style: "{label_style}",
                span { "{arch.label}" }
                if chosen {
                    Badge { tone: Tone::Accent, filled: true, "direction" }
                }
            }
            p { style: "{desc_style}", "{arch.description}" }
        }
    }
}

/// The pin + comment annotation layer over the chosen archetype's preview.
fn annotation_layer(
    arch: &ArchetypeCard,
    pins: &[PinPoint],
    comments: &[String],
    decided: bool,
    on_place_pin: Option<EventHandler<(f64, f64, f64, f64)>>,
    on_comment: Option<EventHandler<String>>,
) -> Element {
    let wrap = format!(
        "margin-top:16px;padding-top:14px;border-top:1px solid {border};\
         display:flex;flex-direction:column;gap:10px;",
        border = tokens::var::BORDER,
    );
    // The clickable preview: clicking emits the pixel offset + box size so the
    // caller can normalize into a 0..1 pin via `selection::place_pin`.
    let stage = format!(
        "position:relative;width:100%;max-width:520px;aspect-ratio:4 / 3;\
         border-radius:8px;overflow:hidden;border:1px solid {border};\
         background:{base};cursor:{cursor};",
        border = tokens::var::BORDER_STRONG,
        base = tokens::var::SURFACE_BASE,
        cursor = if decided { "default" } else { "crosshair" },
    );
    let img_style = "position:absolute;inset:0;width:100%;height:100%;\
                     object-fit:cover;pointer-events:none;";
    rsx! {
        div { style: "{wrap}",
            {heading("Annotate the direction")}
            div {
                class: "dr-annotation-stage",
                style: "{stage}",
                "data-pin-count": "{pins.len()}",
                onclick: move |evt: MouseEvent| {
                    if decided {
                        return;
                    }
                    // `element_coordinates` is the click offset relative to the
                    // target's top-left. We forward it plus the box size so the
                    // caller can normalize into a 0..1 pin via `place_pin`. The
                    // rendered box is a 4:3 stage; the caller knows its pixel
                    // dimensions, so we pass the offset and let the normalize
                    // happen there. We forward (0,0) dims as a sentinel meaning
                    // "already-resolved"; callers that know the size override it.
                    let coords = evt.element_coordinates();
                    if let Some(h) = &on_place_pin {
                        h.call((coords.x, coords.y, 0.0, 0.0));
                    }
                },
                match arch.image_url_light.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                    Some(light) => rsx! {
                        img { class: "dr-themed-dark", style: "{img_style}", src: "{arch.image_url}", alt: "{arch.label}" }
                        img { class: "dr-themed-light", style: "{img_style}", src: "{light}", alt: "{arch.label}" }
                    },
                    None => rsx! {
                        img { style: "{img_style}", src: "{arch.image_url}", alt: "{arch.label}" }
                    },
                }
                for (i, pin) in pins.iter().enumerate() {
                    {pin_marker(i, pin)}
                }
            }
            if !pins.is_empty() {
                ul {
                    style: format!(
                        "margin:0;padding-left:18px;font-family:{};font-size:12px;color:{};",
                        tokens::FONT_SANS, tokens::var::TEXT_MUTED,
                    ),
                    for (i, pin) in pins.iter().enumerate() {
                        li { style: "margin:2px 0;",
                            span {
                                style: format!("color:{};font-weight:600;", tokens::var::ACCENT),
                                "#{i+1} "
                            }
                            "{pin.note}"
                        }
                    }
                }
            }
            {comment_box(comments, decided, on_comment)}
        }
    }
}

/// A single numbered pin marker positioned over the preview.
fn pin_marker(index: usize, pin: &PinPoint) -> Element {
    let dot = format!(
        "position:absolute;left:{left};top:{top};transform:translate(-50%,-50%);\
         width:18px;height:18px;border-radius:999px;background:{accent};\
         color:{on};border:2px solid {base};display:flex;align-items:center;\
         justify-content:center;font-family:{mono};font-size:10px;font-weight:700;\
         box-shadow:0 1px 3px rgba(0,0,0,0.5);",
        left = pin.left_pct(),
        top = pin.top_pct(),
        accent = tokens::var::ACCENT,
        on = tokens::var::ON_ACCENT,
        base = tokens::var::SURFACE_BASE,
        mono = tokens::FONT_MONO,
    );
    rsx! {
        div {
            class: "dr-pin",
            style: "{dot}",
            "data-pin-index": "{index}",
            title: "{pin.note}",
            "{index + 1}"
        }
    }
}

/// The comment box under the annotation layer — a textarea + add button. Each
/// add emits the entered text; the parent owns the comment list.
fn comment_box(
    comments: &[String],
    decided: bool,
    on_comment: Option<EventHandler<String>>,
) -> Element {
    let mut draft = use_signal(String::new);
    let ta_style = format!(
        "width:100%;min-height:60px;resize:vertical;padding:8px 10px;border-radius:6px;\
         background:{surface};border:1px solid {border};color:{text};\
         font-family:{sans};font-size:13px;",
        surface = tokens::var::SURFACE_BASE,
        border = tokens::var::BORDER,
        text = tokens::var::TEXT,
        sans = tokens::FONT_SANS,
    );
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:8px;",
            if !comments.is_empty() {
                div { style: "display:flex;flex-direction:column;gap:6px;",
                    for c in comments.iter() {
                        div {
                            class: "dr-comment",
                            style: format!(
                                "padding:8px 10px;border-radius:6px;background:{};border:1px solid {};\
                                 font-family:{};font-size:13px;color:{};",
                                tokens::var::SURFACE_RAISED, tokens::var::BORDER, tokens::FONT_SANS, tokens::var::TEXT,
                            ),
                            "{c}"
                        }
                    }
                }
            }
            if !decided {
                textarea {
                    class: "dr-comment-input",
                    style: "{ta_style}",
                    placeholder: "Add a comment on this direction…",
                    value: "{draft}",
                    oninput: move |e| draft.set(e.value()),
                }
                div {
                    Button {
                        variant: ButtonVariant::Secondary,
                        tone: Tone::Accent,
                        disabled: draft.read().trim().is_empty(),
                        on_click: move |_| {
                            let text = draft.read().trim().to_string();
                            if !text.is_empty() {
                                if let Some(h) = &on_comment {
                                    h.call(text);
                                }
                                draft.set(String::new());
                            }
                        },
                        "Add comment"
                    }
                }
            }
        }
    }
}

// ===========================================================================
// PickerView
// ===========================================================================

/// A blocking PICKER: a titled prompt plus a list of plain options. Selecting an
/// option emits its id; the selected row takes an accent rail + chip. Used for
/// factory/mode/station/confirm picks.
#[component]
pub fn PickerView(
    /// The picker title.
    #[props(default)]
    title: Option<String>,
    /// The picker prompt.
    prompt: String,
    /// The selectable options.
    options: Vec<PickerItem>,
    /// The currently-selected option id, if any.
    #[props(default)]
    selected: Option<String>,
    /// Whether the picker is already decided / read-only.
    #[props(default = false)]
    decided: bool,
    /// Select handler — called with the chosen option id.
    #[props(default)]
    on_select: Option<EventHandler<String>>,
) -> Element {
    let title_style = format!(
        "margin:0;font-family:{sans};font-size:16px;font-weight:700;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let prompt_style = format!(
        "margin:6px 0 0;font-family:{sans};font-size:13px;color:{muted};line-height:1.4;",
        sans = tokens::FONT_SANS,
        muted = tokens::var::TEXT_MUTED,
    );
    rsx! {
        Card {
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:6px;",
                Badge { tone: Tone::Info, "picker" }
            }
            if let Some(t) = title.clone() {
                h2 { style: "{title_style}", "{t}" }
            }
            if !prompt.is_empty() {
                p { style: "{prompt_style}", "{prompt}" }
            }
            div { style: "display:flex;flex-direction:column;gap:8px;margin-top:12px;",
                for opt in options.iter() {
                    {picker_row(opt, selected.as_deref() == Some(&opt.id), decided, on_select)}
                }
            }
        }
    }
}

/// One picker row.
fn picker_row(
    opt: &PickerItem,
    selected: bool,
    decided: bool,
    on_select: Option<EventHandler<String>>,
) -> Element {
    let rail = if selected {
        format!("border-left:3px solid {};", tokens::var::ACCENT)
    } else {
        "border-left:3px solid transparent;".to_string()
    };
    let border = if selected { tokens::var::ACCENT } else { tokens::var::BORDER };
    let cursor = if decided { "default" } else { "pointer" };
    let row = format!(
        "display:flex;align-items:center;gap:12px;padding:10px 12px;border-radius:6px;\
         background:{surface};border:1px solid {border};{rail}cursor:{cursor};\
         text-align:left;width:100%;color:{text};transition:border-color .12s ease;",
        surface = tokens::var::SURFACE_RAISED,
        border = border,
        text = tokens::var::TEXT,
    );
    let label_style = format!(
        "font-family:{sans};font-size:14px;font-weight:600;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::var::TEXT,
    );
    let desc_style = format!(
        "margin:2px 0 0;font-family:{sans};font-size:12px;color:{muted};",
        sans = tokens::FONT_SANS,
        muted = tokens::var::TEXT_MUTED,
    );
    let id = opt.id.clone();
    rsx! {
        button {
            class: "dr-picker-row",
            style: "{row}",
            "data-option-id": "{opt.id}",
            "data-selected": "{selected}",
            "aria-pressed": "{selected}",
            disabled: decided,
            onclick: move |_| {
                if !decided {
                    if let Some(h) = &on_select {
                        h.call(id.clone());
                    }
                }
            },
            div { style: "flex:1;min-width:0;",
                span { style: "{label_style}", "{opt.label}" }
                if let Some(d) = opt.description.clone() {
                    p { style: "{desc_style}", "{d}" }
                }
            }
            if let Some(s) = opt.secondary.clone() {
                span {
                    style: format!(
                        "font-family:{};font-size:12px;color:{};",
                        tokens::FONT_MONO, tokens::var::TEXT_FAINT,
                    ),
                    "{s}"
                }
            }
            if selected {
                Badge { tone: Tone::Accent, filled: true, "selected" }
            }
        }
    }
}

// ===========================================================================
// Shared submit bar
// ===========================================================================

/// A sticky submit bar shared by question + direction. Shows a status line and a
/// primary submit button.
fn submit_bar(
    status: &str,
    hint: &str,
    can_submit: bool,
    done: bool,
    label: &str,
    on_submit: Option<EventHandler<MouseEvent>>,
) -> Element {
    let bar = format!(
        "display:flex;align-items:center;gap:12px;margin-top:16px;padding-top:14px;\
         border-top:1px solid {border};",
        border = tokens::var::BORDER,
    );
    let status_style = format!(
        "font-family:{mono};font-size:12px;color:{muted};",
        mono = tokens::FONT_MONO,
        muted = tokens::var::TEXT_MUTED,
    );
    let hint_style = format!(
        "font-family:{mono};font-size:11px;color:{faint};text-transform:lowercase;",
        mono = tokens::FONT_MONO,
        faint = tokens::var::TEXT_FAINT,
    );
    rsx! {
        div { class: "dr-submit-bar", style: "{bar}",
            span { style: "{hint_style}", "{hint}" }
            span { style: "flex:1;" }
            span { style: "{status_style}", "{status}" }
            if done {
                Badge { tone: Tone::Ok, filled: true, "{label}" }
            } else {
                Button {
                    variant: ButtonVariant::Primary,
                    tone: Tone::Accent,
                    disabled: !can_submit,
                    on_click: move |evt| {
                        if let Some(h) = &on_submit {
                            h.call(evt);
                        }
                    },
                    "{label}"
                }
            }
        }
    }
}
