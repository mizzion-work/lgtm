//! Syntax highlighting via `syntect`.
//!
//! The [`Highlighter`] trait is stateful across lines (block comments and
//! multi-line strings need that), so the entry point hands the *whole
//! document* and gets back one `Vec<StyledSpan>` per line.
//!
//! - [`NoopHighlighter`] returns empty spans per line; callers fall back
//!   to the GUI's default foreground color.
//! - [`SyntectHighlighter`] does real highlighting using `syntect`'s
//!   bundled syntaxes and themes.

use std::ops::Range;
use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Bold bit for [`StyledSpan::style_bits`].
pub const STYLE_BOLD: u8 = 1 << 0;
/// Italic bit for [`StyledSpan::style_bits`].
pub const STYLE_ITALIC: u8 = 1 << 1;
/// Underline bit for [`StyledSpan::style_bits`].
pub const STYLE_UNDERLINE: u8 = 1 << 2;

/// One styled span within a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    /// Byte range within the line (including any trailing `\n`).
    pub range: Range<usize>,
    /// Foreground color as `0xRRGGBB`. The GUI converts to its color type.
    pub rgb: u32,
    /// Bitfield: [`STYLE_BOLD`] | [`STYLE_ITALIC`] | [`STYLE_UNDERLINE`].
    pub style_bits: u8,
}

/// Trait the GUI calls to syntax-highlight a document.
///
/// Implementations should be cheap to share across documents.
/// `language_hint` is typically the file extension (without the dot)
/// and may be empty — implementations should fall back to plain text.
pub trait Highlighter: Send + Sync {
    /// Highlight a whole document, returning one set of spans per line.
    /// The returned `Vec` has one entry per `\n`-delimited line in `text`
    /// (counting the final line even if it doesn't end with `\n`).
    fn highlight_document(&self, language_hint: &str, text: &str) -> Vec<Vec<StyledSpan>>;
}

/// No-op highlighter. Returns empty spans per line so the GUI uses its
/// default foreground color.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopHighlighter;

impl Highlighter for NoopHighlighter {
    fn highlight_document(&self, _language_hint: &str, text: &str) -> Vec<Vec<StyledSpan>> {
        line_count(text).map(|_| Vec::new()).collect::<Vec<_>>()
    }
}

fn line_count(text: &str) -> impl Iterator<Item = ()> + '_ {
    LinesWithEndings::from(text).map(|_| ())
}

/// Lazy global syntax + theme assets. Loading `default-fancy` themes is
/// cheap enough to do once per process.
fn assets() -> &'static (SyntaxSet, ThemeSet) {
    static ASSETS: OnceLock<(SyntaxSet, ThemeSet)> = OnceLock::new();
    ASSETS.get_or_init(|| {
        (
            SyntaxSet::load_defaults_newlines(),
            ThemeSet::load_defaults(),
        )
    })
}

/// `syntect`-backed highlighter.
pub struct SyntectHighlighter {
    theme: Theme,
}

impl SyntectHighlighter {
    /// A highlighter using a dark theme that pairs with the lgtm GUI's
    /// dark mode.
    pub fn dark() -> Self {
        Self::with_theme("base16-eighties.dark")
    }

    /// A highlighter using a light theme.
    pub fn light() -> Self {
        Self::with_theme("base16-ocean.light")
    }

    /// Pick by theme name; falls back to a dark default if unknown.
    pub fn with_theme(name: &str) -> Self {
        let (_, themes) = assets();
        let theme = themes
            .themes
            .get(name)
            .cloned()
            .unwrap_or_else(|| themes.themes["base16-eighties.dark"].clone());
        Self { theme }
    }
}

impl Default for SyntectHighlighter {
    fn default() -> Self {
        Self::dark()
    }
}

impl Highlighter for SyntectHighlighter {
    fn highlight_document(&self, language_hint: &str, text: &str) -> Vec<Vec<StyledSpan>> {
        let (syntaxes, _) = assets();
        let syntax = if language_hint.is_empty() {
            syntaxes.find_syntax_plain_text()
        } else {
            syntaxes
                .find_syntax_by_extension(language_hint)
                .or_else(|| syntaxes.find_syntax_by_name(language_hint))
                .unwrap_or_else(|| syntaxes.find_syntax_plain_text())
        };

        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        let mut out: Vec<Vec<StyledSpan>> = Vec::new();
        for line in LinesWithEndings::from(text) {
            let regions = highlighter
                .highlight_line(line, syntaxes)
                .unwrap_or_default();
            let mut spans = Vec::with_capacity(regions.len());
            let mut pos = 0usize;
            for (style, fragment) in regions {
                let len = fragment.len();
                spans.push(StyledSpan {
                    range: pos..pos + len,
                    rgb: style_to_rgb(style),
                    style_bits: style_to_bits(style),
                });
                pos += len;
            }
            out.push(spans);
        }
        out
    }
}

fn style_to_rgb(style: Style) -> u32 {
    let c = style.foreground;
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

fn style_to_bits(style: Style) -> u8 {
    let mut b = 0u8;
    if style.font_style.contains(FontStyle::BOLD) {
        b |= STYLE_BOLD;
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        b |= STYLE_ITALIC;
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        b |= STYLE_UNDERLINE;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_returns_one_empty_per_line() {
        let h = NoopHighlighter;
        let out = h.highlight_document("rs", "a\nb\nc\n");
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|spans| spans.is_empty()));
    }

    #[test]
    fn noop_handles_empty_input() {
        let h = NoopHighlighter;
        assert_eq!(h.highlight_document("rs", "").len(), 0);
    }

    #[test]
    fn noop_handles_no_trailing_newline() {
        let h = NoopHighlighter;
        let out = h.highlight_document("rs", "a\nb");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn syntect_highlights_rust_keywords() {
        let h = SyntectHighlighter::dark();
        let out = h.highlight_document("rs", "fn main() {}\n");
        assert_eq!(out.len(), 1, "one line in, one line out");
        let spans = &out[0];
        // The line should have been split into multiple spans (keyword,
        // whitespace, identifier, ...).
        assert!(spans.len() > 1, "got {} spans", spans.len());
        // Spans must cover the whole line contiguously, with no gaps.
        let mut cursor = 0;
        for s in spans {
            assert_eq!(s.range.start, cursor, "spans must be contiguous");
            cursor = s.range.end;
        }
        // And the last span should reach the line length (including \n).
        assert_eq!(cursor, "fn main() {}\n".len());
    }

    #[test]
    fn syntect_falls_back_to_plain_text_for_unknown_extension() {
        let h = SyntectHighlighter::dark();
        let out = h.highlight_document("madeup-no-such-ext", "hello\n");
        assert_eq!(out.len(), 1);
        // Plain text typically yields one big span.
        assert!(!out[0].is_empty());
    }

    #[test]
    fn syntect_preserves_line_count_over_multiline_string() {
        // Multi-line strings exercise the stateful path (per-line state
        // carried across calls).
        let src = "let s = \"\nstill in string\n\";\n";
        let h = SyntectHighlighter::dark();
        let out = h.highlight_document("rs", src);
        assert_eq!(out.len(), 3);
        for spans in &out {
            assert!(!spans.is_empty(), "every line should have spans");
        }
    }

    #[test]
    fn unknown_theme_falls_back_to_dark() {
        let h = SyntectHighlighter::with_theme("no-such-theme");
        let out = h.highlight_document("rs", "fn x() {}\n");
        assert!(!out[0].is_empty());
    }
}
