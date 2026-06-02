//! Theme controller — the manual override for the system appearance.
//!
//! darkrun follows `prefers-color-scheme` by default. On top of that, the user
//! can pin a theme via a three-way choice ([`ThemeChoice`]): **System** (follow
//! the OS), **Light**, or **Dark**. The choice maps to a `[data-theme]` attribute
//! on the document root — `light`/`dark` pin the theme (winning over the media
//! query in [`crate::tokens::THEME_CSS`]); System removes the attribute so the
//! media query / dark default takes over again.
//!
//! This module is the small, renderer-agnostic core: a `Copy` enum plus the
//! pure mapping helpers ([`ThemeChoice::data_theme`], [`ThemeChoice::from_label`]).
//! Both the desktop app and the website apply the result the same way — set or
//! remove `document.documentElement`'s `data-theme` attribute — and persist the
//! label (`"system"` / `"light"` / `"dark"`) wherever they keep settings.

/// The user's theme override. Defaults to [`ThemeChoice::System`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Follow the OS appearance via `prefers-color-scheme` (no attribute set).
    #[default]
    System,
    /// Pin the light theme (`data-theme="light"`).
    Light,
    /// Pin the dark theme (`data-theme="dark"`).
    Dark,
}

impl ThemeChoice {
    /// The value to write to the root `[data-theme]` attribute, or `None` when
    /// the attribute should be **removed** ([`ThemeChoice::System`]).
    ///
    /// ```
    /// use darkrun_ui::theme::ThemeChoice;
    /// assert_eq!(ThemeChoice::System.data_theme(), None);
    /// assert_eq!(ThemeChoice::Light.data_theme(), Some("light"));
    /// assert_eq!(ThemeChoice::Dark.data_theme(), Some("dark"));
    /// ```
    pub fn data_theme(self) -> Option<&'static str> {
        match self {
            ThemeChoice::System => None,
            ThemeChoice::Light => Some("light"),
            ThemeChoice::Dark => Some("dark"),
        }
    }

    /// The persisted/serialized label for this choice.
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    /// The human-facing label for a control (`System` / `Light` / `Dark`).
    pub fn display_label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    /// Parse a persisted label back into a choice (case-insensitive). Anything
    /// unrecognized — including an empty string or a missing setting — falls back
    /// to [`ThemeChoice::System`], the safe default.
    pub fn from_label(label: &str) -> ThemeChoice {
        match label.trim().to_ascii_lowercase().as_str() {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        }
    }

    /// The three choices in control order, for rendering a System / Light / Dark
    /// segmented control.
    pub const ALL: [ThemeChoice; 3] =
        [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];
}

/// A snippet of JS that applies a [`ThemeChoice`] to `document.documentElement`
/// by setting or removing the `data-theme` attribute. Renderer-agnostic: the
/// desktop app can `eval` it and the website can run it inline.
///
/// `System` removes the attribute (returning to the media query); `light`/`dark`
/// set it (pinning the theme via the override blocks in [`crate::tokens::THEME_CSS`]).
pub fn apply_script(choice: ThemeChoice) -> String {
    match choice.data_theme() {
        Some(value) => format!(
            "document.documentElement.setAttribute('data-theme','{value}');"
        ),
        None => {
            "document.documentElement.removeAttribute('data-theme');".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_theme_maps_each_choice() {
        assert_eq!(ThemeChoice::System.data_theme(), None);
        assert_eq!(ThemeChoice::Light.data_theme(), Some("light"));
        assert_eq!(ThemeChoice::Dark.data_theme(), Some("dark"));
    }

    #[test]
    fn from_label_roundtrips_and_defaults_to_system() {
        for choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_label(choice.label()), choice);
        }
        assert_eq!(ThemeChoice::from_label("LIGHT"), ThemeChoice::Light);
        assert_eq!(ThemeChoice::from_label("  Dark "), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::from_label(""), ThemeChoice::System);
        assert_eq!(ThemeChoice::from_label("nonsense"), ThemeChoice::System);
    }

    #[test]
    fn default_is_system() {
        assert_eq!(ThemeChoice::default(), ThemeChoice::System);
    }

    #[test]
    fn apply_script_sets_or_removes_attribute() {
        assert!(apply_script(ThemeChoice::System).contains("removeAttribute"));
        assert!(apply_script(ThemeChoice::Light).contains("'light'"));
        assert!(apply_script(ThemeChoice::Dark).contains("'dark'"));
        assert!(apply_script(ThemeChoice::Dark).contains("setAttribute"));
    }

    #[test]
    fn all_is_in_control_order() {
        assert_eq!(
            ThemeChoice::ALL,
            [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
        );
    }
}
