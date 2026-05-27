// Tests use the unsafe std::env::set_var in Rust 2024 for isolation.
#![allow(unsafe_code)]
//! Persistent user preferences (font size today; trivially extensible).
//!
//! Stored alongside the recent-files list at `<config>/lgtm/settings.tsv`,
//! one `key<TAB>value<NL>` row per setting. Missing or unparseable rows
//! fall back to defaults silently — preferences are a nice-to-have,
//! never block startup.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Default monospace font size used by the GUI.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// Smallest allowed font size.
pub const MIN_FONT_SIZE: f32 = 8.0;
/// Largest allowed font size.
pub const MAX_FONT_SIZE: f32 = 32.0;
/// Increment / decrement step.
pub const FONT_SIZE_STEP: f32 = 1.0;

/// Which palette the GUI shell uses (egui's window chrome, menus, etc).
/// `Auto` follows the OS preference where supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    /// Follow the OS / egui default.
    Auto,
    /// Force light mode.
    Light,
    /// Force dark mode.
    Dark,
}

impl AppTheme {
    fn as_str(self) -> &'static str {
        match self {
            AppTheme::Auto => "auto",
            AppTheme::Light => "light",
            AppTheme::Dark => "dark",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(AppTheme::Auto),
            "light" => Some(AppTheme::Light),
            "dark" => Some(AppTheme::Dark),
            _ => None,
        }
    }
}

/// Which syntect theme paints the source code in the diff view. A few
/// bundled themes are surfaced explicitly; anything else falls through
/// to the syntect default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTheme {
    /// `base16-eighties.dark` — dark, vibrant. Default.
    EightiesDark,
    /// `base16-mocha.dark` — dark, warm.
    MochaDark,
    /// `base16-ocean.dark` — dark, blue.
    OceanDark,
    /// `base16-ocean.light` — light, blue.
    OceanLight,
    /// `InspiredGitHub` — light, GitHub-style.
    InspiredGitHub,
    /// `Solarized (dark)`.
    SolarizedDark,
    /// `Solarized (light)`.
    SolarizedLight,
}

impl EditorTheme {
    /// Name as accepted by [`crate::SyntectHighlighter::with_theme`].
    pub fn syntect_name(self) -> &'static str {
        match self {
            EditorTheme::EightiesDark => "base16-eighties.dark",
            EditorTheme::MochaDark => "base16-mocha.dark",
            EditorTheme::OceanDark => "base16-ocean.dark",
            EditorTheme::OceanLight => "base16-ocean.light",
            EditorTheme::InspiredGitHub => "InspiredGitHub",
            EditorTheme::SolarizedDark => "Solarized (dark)",
            EditorTheme::SolarizedLight => "Solarized (light)",
        }
    }

    /// All bundled choices, in menu order.
    pub fn all() -> &'static [EditorTheme] {
        &[
            EditorTheme::EightiesDark,
            EditorTheme::MochaDark,
            EditorTheme::OceanDark,
            EditorTheme::OceanLight,
            EditorTheme::InspiredGitHub,
            EditorTheme::SolarizedDark,
            EditorTheme::SolarizedLight,
        ]
    }

    /// Short, human-friendly label for the menu.
    pub fn label(self) -> &'static str {
        match self {
            EditorTheme::EightiesDark => "Base16 Eighties (dark)",
            EditorTheme::MochaDark => "Base16 Mocha (dark)",
            EditorTheme::OceanDark => "Base16 Ocean (dark)",
            EditorTheme::OceanLight => "Base16 Ocean (light)",
            EditorTheme::InspiredGitHub => "InspiredGitHub (light)",
            EditorTheme::SolarizedDark => "Solarized (dark)",
            EditorTheme::SolarizedLight => "Solarized (light)",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::all().iter().copied().find(|t| t.syntect_name() == s)
    }
}

impl Default for EditorTheme {
    fn default() -> Self {
        EditorTheme::EightiesDark
    }
}

/// User preferences. New fields are additive: add the field, give it a
/// sensible default in [`Settings::default`], add a row in
/// [`Settings::save_to`] / [`Settings::load_from`], and consumers will
/// transparently pick it up.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Monospace font size for both diff and edit views.
    pub font_size: f32,
    /// egui chrome theme (window background, menus, buttons).
    pub app_theme: AppTheme,
    /// Syntect theme for syntax-highlighted code in the diff view.
    pub editor_theme: EditorTheme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            app_theme: AppTheme::Auto,
            editor_theme: EditorTheme::default(),
        }
    }
}

impl Settings {
    /// Read from the default config path. Missing or malformed files
    /// yield [`Settings::default`].
    pub fn load() -> Self {
        let Some(path) = default_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Read from an explicit path. Always returns a `Settings` —
    /// unparseable rows fall back to defaults.
    pub fn load_from(path: &Path) -> Self {
        let mut out = Self::default();
        let Ok(body) = std::fs::read_to_string(path) else {
            return out;
        };
        for line in body.lines() {
            let mut parts = line.splitn(2, '\t');
            let key = parts.next().unwrap_or("");
            let val = parts.next().unwrap_or("");
            match key {
                "font_size" => {
                    if let Ok(v) = val.parse::<f32>() {
                        out.font_size = clamp_font(v);
                    }
                }
                "app_theme" => {
                    if let Some(t) = AppTheme::parse(val) {
                        out.app_theme = t;
                    }
                }
                "editor_theme" => {
                    if let Some(t) = EditorTheme::parse(val) {
                        out.editor_theme = t;
                    }
                }
                _ => { /* unknown key — ignore for forward-compat */ }
            }
        }
        out
    }

    /// Persist to the default config path. Returns `Ok(())` when no
    /// config dir is available (extremely rare).
    pub fn save(&self) -> Result<()> {
        let Some(path) = default_path() else {
            return Ok(());
        };
        self.save_to(&path)
    }

    /// Persist to an explicit path. Creates parent dirs as needed.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| crate::error::Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let body = format!(
            "font_size\t{}\napp_theme\t{}\neditor_theme\t{}\n",
            self.font_size,
            self.app_theme.as_str(),
            self.editor_theme.syntect_name(),
        );
        std::fs::write(path, body).map_err(|source| crate::error::Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// Bump font size by [`FONT_SIZE_STEP`], clamped to [`MAX_FONT_SIZE`].
    pub fn increase_font(&mut self) {
        self.font_size = clamp_font(self.font_size + FONT_SIZE_STEP);
    }

    /// Reduce font size by [`FONT_SIZE_STEP`], clamped to [`MIN_FONT_SIZE`].
    pub fn decrease_font(&mut self) {
        self.font_size = clamp_font(self.font_size - FONT_SIZE_STEP);
    }

    /// Reset font size to [`DEFAULT_FONT_SIZE`].
    pub fn reset_font(&mut self) {
        self.font_size = DEFAULT_FONT_SIZE;
    }
}

fn clamp_font(v: f32) -> f32 {
    v.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

fn default_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("lgtm").join("settings.tsv"))
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("LGTM_CONFIG_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg));
            }
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lgtm-settings-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.tsv")
    }

    #[test]
    fn default_font_size_is_thirteen() {
        let s = Settings::default();
        assert_eq!(s.font_size, DEFAULT_FONT_SIZE);
    }

    #[test]
    fn load_from_missing_file_returns_defaults() {
        let s = Settings::load_from(Path::new("/no/such/path/lgtm.tsv"));
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = tmp_path("rt");
        let s = Settings {
            font_size: 17.0,
            app_theme: AppTheme::Light,
            editor_theme: EditorTheme::SolarizedLight,
        };
        s.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.font_size, 17.0);
        assert_eq!(loaded.app_theme, AppTheme::Light);
        assert_eq!(loaded.editor_theme, EditorTheme::SolarizedLight);
    }

    #[test]
    fn app_theme_parse_round_trips() {
        for t in [AppTheme::Auto, AppTheme::Light, AppTheme::Dark] {
            assert_eq!(AppTheme::parse(t.as_str()), Some(t));
        }
        assert_eq!(AppTheme::parse("bogus"), None);
    }

    #[test]
    fn editor_theme_parse_finds_each_bundled_one() {
        for t in EditorTheme::all() {
            assert_eq!(EditorTheme::parse(t.syntect_name()), Some(*t));
        }
        assert_eq!(EditorTheme::parse("not-a-theme"), None);
    }

    #[test]
    fn unknown_theme_value_falls_back_to_default() {
        let path = tmp_path("unknown-theme");
        std::fs::write(
            &path,
            "app_theme\tplaid\neditor_theme\trainbow\nfont_size\t14.0\n",
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.font_size, 14.0);
        assert_eq!(s.app_theme, AppTheme::Auto);
        assert_eq!(s.editor_theme, EditorTheme::default());
    }

    #[test]
    fn load_clamps_out_of_range_values() {
        let path = tmp_path("clamp");
        std::fs::write(&path, "font_size\t999.0\n").unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.font_size, MAX_FONT_SIZE);

        std::fs::write(&path, "font_size\t1.0\n").unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.font_size, MIN_FONT_SIZE);
    }

    #[test]
    fn load_skips_unknown_keys_and_malformed_rows() {
        let path = tmp_path("malformed");
        std::fs::write(
            &path,
            "font_size\t15.0\nfuture_key\twhatever\nnot a key line\n",
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.font_size, 15.0);
    }

    #[test]
    fn increase_decrease_reset_obey_clamps() {
        let mut s = Settings::default();
        for _ in 0..50 {
            s.increase_font();
        }
        assert_eq!(s.font_size, MAX_FONT_SIZE);
        for _ in 0..50 {
            s.decrease_font();
        }
        assert_eq!(s.font_size, MIN_FONT_SIZE);
        s.reset_font();
        assert_eq!(s.font_size, DEFAULT_FONT_SIZE);
    }

    #[test]
    fn save_creates_parent_dirs() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let deep = std::env::temp_dir()
            .join(format!("lgtm-settings-deep-{nanos}"))
            .join("a/b/c")
            .join("settings.tsv");
        let s = Settings::default();
        s.save_to(&deep).unwrap();
        assert!(deep.exists());
    }
}
