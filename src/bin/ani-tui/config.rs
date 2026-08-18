use ratatui::style::Color;
use ratatui::widgets::BorderType;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level config file shape (`$XDG_CONFIG_HOME/ani-tui/config.yml`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Colors and border style for the interactive TUI
    pub theme: Theme,
}

impl Config {
    /// Loads the config file, falling back to [`Config::default`] if it doesn't exist, and
    /// printing a warning (then still falling back) if it exists but fails to parse. Reads
    /// from `$XDG_CONFIG_HOME/ani-tui/config.yml` (or platform equivalent).
    pub fn load() -> Self {
        let mut config = Self::load_raw();
        config.theme = config.theme.merge_sources();
        config
    }

    /// Reads and parses the config file, without merging [`Theme::sources`] over the built-in
    /// defaults yet. Split out from [`Self::load`] so tests can exercise the merge separately.
    fn load_raw() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Self::default();
        };

        match parse_config(&contents) {
            Ok(config) => config,
            Err(err) => {
                eprintln!(
                    "Warning: could not parse {}: {err}. Using the default theme.",
                    path.display()
                );
                Self::default()
            }
        }
    }
}

/// Path to the config file, or `None` if the platform's config directory can't be determined.
fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ani-tui").map(|dirs| dirs.config_dir().join("config.yml"))
}

/// Parses a config file's contents. Split out from [`Config::load`] so it can be tested
/// against a fixture file without touching the filesystem.
fn parse_config(yaml: &str) -> Result<Config, serde_norway::Error> {
    serde_norway::from_str(yaml)
}

/// Which border-drawing characters to use. Maps to [`ratatui::widgets::BorderType`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BorderKind {
    #[default]
    Rounded,
    Plain,
    Double,
    Thick,
}

impl BorderKind {
    /// Converts to the ratatui type actually used for rendering.
    pub fn to_ratatui(self) -> BorderType {
        match self {
            BorderKind::Rounded => BorderType::Rounded,
            BorderKind::Plain => BorderType::Plain,
            BorderKind::Double => BorderType::Double,
            BorderKind::Thick => BorderType::Thick,
        }
    }
}

/// Colors and border style for the interactive TUI. Every field defaults independently to a
/// Catppuccin Mocha value, so a config file only needs to specify what it wants to change.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Border-drawing style for every panel
    pub border_type: BorderKind,
    /// Focused borders, in-progress status text, etc.
    #[serde(default = "default_accent")]
    pub accent: Color,
    /// Default text color
    #[serde(default = "default_text")]
    pub text: Color,
    /// Unfocused borders, hints, and the fallback color for sources with no configured one
    #[serde(default = "default_muted")]
    pub muted: Color,
    /// Error status text
    #[serde(default = "default_error")]
    pub error: Color,
    /// Warning status text
    #[serde(default = "default_warning")]
    pub warning: Color,
    /// Text color of the selected row's highlight bar
    #[serde(default = "default_selection_fg")]
    pub selection_fg: Color,
    /// Background color of the selected row's highlight bar
    #[serde(default = "default_selection_bg")]
    pub selection_bg: Color,
    /// Per-source badge overrides, keyed by [`crate::ani_tui::anime_repo::GlobalId::prefix`]
    /// (e.g. `"ADB-1"`). Merged over [`Theme::builtin_sources`] field-by-field, so overriding
    /// just a color keeps the built-in label, and vice versa. Unmapped prefixes fall back to
    /// the raw prefix as label and [`Theme::muted`] as color.
    pub sources: HashMap<String, SourceTheme>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            border_type: BorderKind::default(),
            accent: default_accent(),
            text: default_text(),
            muted: default_muted(),
            error: default_error(),
            warning: default_warning(),
            selection_fg: default_selection_fg(),
            selection_bg: default_selection_bg(),
            sources: HashMap::new(),
        }
    }
}

impl Theme {
    /// Built-in per-source label/color, used to fill in whatever a config file's `sources`
    /// entries don't override.
    fn builtin_sources() -> HashMap<String, SourceTheme> {
        HashMap::from([
            (
                "ADB-1".to_string(),
                SourceTheme {
                    label: Some("AniDB".to_string()),
                    color: Some(Color::Rgb(0xa6, 0xe3, 0xa1)),
                },
            ),
            (
                "AWT-1".to_string(),
                SourceTheme {
                    label: Some("AniWorld".to_string()),
                    color: Some(Color::Rgb(0xcb, 0xa6, 0xf7)),
                },
            ),
        ])
    }

    /// Merges [`Self::sources`] (as deserialized from a config file) over
    /// [`Self::builtin_sources`], field-by-field.
    fn merge_sources(mut self) -> Self {
        let mut merged = Self::builtin_sources();
        for (prefix, override_theme) in self.sources.drain() {
            let entry = merged.entry(prefix).or_default();
            if override_theme.label.is_some() {
                entry.label = override_theme.label;
            }
            if override_theme.color.is_some() {
                entry.color = override_theme.color;
            }
        }
        self.sources = merged;
        self
    }

    /// Resolves the display label and badge color for `prefix`, falling back to the raw
    /// prefix and [`Self::muted`] if nothing is configured for it.
    pub fn source_style(&self, prefix: &str) -> (String, Color) {
        match self.sources.get(prefix) {
            Some(source) => (
                source.label.clone().unwrap_or_else(|| prefix.to_string()),
                source.color.unwrap_or(self.muted),
            ),
            None => (prefix.to_string(), self.muted),
        }
    }
}

/// A single source's label/color override. Both fields are optional so a config file can
/// override just one without needing to know the other's built-in default.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SourceTheme {
    /// Display label shown in the results list, e.g. `"AniDB"`
    pub label: Option<String>,
    /// Badge color shown in the results list
    pub color: Option<Color>,
}

fn default_accent() -> Color {
    Color::Rgb(0x89, 0xb4, 0xfa)
}

fn default_text() -> Color {
    Color::Rgb(0xcd, 0xd6, 0xf4)
}

fn default_muted() -> Color {
    Color::Rgb(0x6c, 0x70, 0x86)
}

fn default_error() -> Color {
    Color::Rgb(0xf3, 0x8b, 0xa8)
}

fn default_warning() -> Color {
    Color::Rgb(0xf9, 0xe2, 0xaf)
}

fn default_selection_fg() -> Color {
    Color::Rgb(0x1e, 0x1e, 0x2e)
}

fn default_selection_bg() -> Color {
    Color::Rgb(0x89, 0xb4, 0xfa)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_uses_builtin_source_colors_after_merge() {
        let theme = Theme::default().merge_sources();
        assert_eq!(theme.source_style("ADB-1"), ("AniDB".to_string(), Color::Rgb(0xa6, 0xe3, 0xa1)));
        assert_eq!(theme.source_style("AWT-1"), ("AniWorld".to_string(), Color::Rgb(0xcb, 0xa6, 0xf7)));
    }

    #[test]
    fn unmapped_source_falls_back_to_prefix_and_muted() {
        let theme = Theme::default().merge_sources();
        assert_eq!(theme.source_style("XYZ-1"), ("XYZ-1".to_string(), theme.muted));
    }

    #[test]
    fn parses_partial_config_fixture() {
        let yaml = include_str!("../../../tests/fixtures/config-partial.yml");
        let config = parse_config(yaml).expect("fixture should parse");
        let theme = config.theme.merge_sources();

        // Overridden field takes the config's value...
        assert_eq!(theme.accent, Color::Rgb(0x11, 0x22, 0x33));
        // ...while every other scalar field keeps its built-in default.
        assert_eq!(theme.text, default_text());
        assert_eq!(theme.error, default_error());

        // Overriding just a source's color keeps its built-in label.
        assert_eq!(
            theme.source_style("ADB-1"),
            ("AniDB".to_string(), Color::Rgb(0x44, 0x55, 0x66))
        );
        // The other source is untouched by the config file.
        assert_eq!(theme.source_style("AWT-1"), ("AniWorld".to_string(), Color::Rgb(0xcb, 0xa6, 0xf7)));
    }

    #[test]
    fn parse_config_rejects_malformed_yaml() {
        assert!(parse_config("theme: [this is not a mapping").is_err());
    }
}
