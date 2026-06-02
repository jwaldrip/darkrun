//! The darkrun wordmark: **dark** in bold + "run" in regular weight.
//!
//! Three variants:
//! - [`WordmarkVariant::Filled`] — solid accent text, used in the desktop app.
//! - [`WordmarkVariant::Outlined`] — transparent fill with an accent stroke,
//!   used on the website hero.
//! - [`WordmarkVariant::OutlinedSolidRun`] — outlined accent "dark" with a solid
//!   "run", used in the desktop sticky header.

use dioxus::prelude::*;

use crate::tokens;

/// Which rendering of the wordmark to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordmarkVariant {
    /// Solid fill (desktop chrome).
    #[default]
    Filled,
    /// Outlined / stroked text (website hero).
    Outlined,
    /// Outlined accent "dark" + solid "run" at medium weight (sticky header).
    OutlinedSolidRun,
}

impl WordmarkVariant {
    /// The `data-variant` slug emitted on the wordmark root.
    fn slug(self) -> &'static str {
        match self {
            WordmarkVariant::Filled => "filled",
            WordmarkVariant::Outlined => "outlined",
            WordmarkVariant::OutlinedSolidRun => "outlined-solid-run",
        }
    }
}

/// Render the darkrun wordmark.
///
/// `size` is the font size in CSS pixels (defaults to 24). The "dark" segment is
/// bold and accent-colored; "run" is regular weight in the primary text color.
#[component]
pub fn Wordmark(
    #[props(default = WordmarkVariant::Filled)] variant: WordmarkVariant,
    #[props(default = 24.0)] size: f64,
    /// Lights-out interaction (website logo): rest in the dark-filled outline,
    /// glow blue on hover, flicker back out on blur. Ignores `variant` (always
    /// the outlined-dark + solid-run look). Color/glow are driven by the
    /// `data-anim` state in `THEME_CSS` so they override the inline stroke cleanly.
    #[props(default = false)]
    interactive: bool,
) -> Element {
    let root_style = format!(
        "font-family:{font};font-size:{size}px;letter-spacing:-0.02em;\
         line-height:1;display:inline-flex;align-items:baseline;",
        font = tokens::FONT_SANS,
    );

    if interactive {
        let mut state = use_signal(|| "rest");
        // Constant stroke + paint-order inline; the fill color + glow come from
        // THEME_CSS keyed on data-anim (rest -> lit -> flicker), so the keyframes
        // can override without fighting an inline `color`.
        let dark_const = format!(
            "font-weight:800;paint-order:stroke;\
             -webkit-text-stroke:1.5px {accent};text-stroke:1.5px {accent};",
            accent = tokens::ACCENT,
        );
        let run_style = format!("color:{};font-weight:500;", tokens::TEXT);
        return rsx! {
            span {
                class: "dr-wordmark dr-wordmark-anim",
                "data-anim": "{state}",
                style: "{root_style}",
                "aria-label": "darkrun",
                onmouseenter: move |_| state.set("lit"),
                onmouseleave: move |_| state.set("flicker"),
                span { class: "dr-wordmark-dark", style: "{dark_const}", "dark" }
                span { class: "dr-wordmark-run", style: "{run_style}", "run" }
            }
        };
    }

    let (dark_style, run_style) = match variant {
        WordmarkVariant::Filled => (
            format!("color:{};font-weight:800;", tokens::ACCENT),
            format!("color:{};font-weight:400;", tokens::TEXT),
        ),
        WordmarkVariant::Outlined => (
            // Transparent fill + accent stroke via text-stroke (webkit) with a
            // color fallback so the glyphs are never invisible if unsupported.
            format!(
                "color:transparent;font-weight:800;\
                 -webkit-text-stroke:1px {accent};text-stroke:1px {accent};",
                accent = tokens::ACCENT
            ),
            format!(
                "color:transparent;font-weight:400;\
                 -webkit-text-stroke:1px {muted};text-stroke:1px {muted};",
                muted = tokens::TEXT_MUTED
            ),
        ),
        WordmarkVariant::OutlinedSolidRun => (
            // Outlined accent "dark" paired with a solid, medium-weight "run".
            // The "dark" glyphs are FILLED with the base color (not transparent)
            // and painted stroke-under-fill (`paint-order:stroke`), so the fill
            // masks the inner stroke + the crossings where tight-kerned bold
            // glyphs overlap — leaving a single clean outer outline.
            format!(
                "color:{base};font-weight:800;paint-order:stroke;\
                 -webkit-text-stroke:1.5px {accent};text-stroke:1.5px {accent};",
                base = tokens::SURFACE_BASE,
                accent = tokens::ACCENT
            ),
            format!("color:{};font-weight:500;", tokens::TEXT),
        ),
    };

    rsx! {
        span {
            class: "dr-wordmark",
            "data-variant": variant.slug(),
            style: "{root_style}",
            "aria-label": "darkrun",
            span { class: "dr-wordmark-dark", style: "{dark_style}", "dark" }
            span { class: "dr-wordmark-run", style: "{run_style}", "run" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_stable_per_variant() {
        assert_eq!(WordmarkVariant::Filled.slug(), "filled");
        assert_eq!(WordmarkVariant::Outlined.slug(), "outlined");
        assert_eq!(WordmarkVariant::OutlinedSolidRun.slug(), "outlined-solid-run");
    }

    #[test]
    fn default_variant_is_filled() {
        assert_eq!(WordmarkVariant::default(), WordmarkVariant::Filled);
    }
}
