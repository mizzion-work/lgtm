//! Syntax-highlighting integration hook.
//!
//! v1 ships a no-op implementation ([`NoopHighlighter`]). v2 will provide a
//! `syntect`-backed implementation behind the same trait so the GUI does
//! not need to change.

/// One styled span within a line.
#[derive(Debug, Clone)]
pub struct StyledSpan {
    /// Byte range within the line.
    pub range: std::ops::Range<usize>,
    /// Foreground color as 0xRRGGBB. The GUI maps this to its theme palette.
    pub rgb: u32,
    /// Bold / italic / underline bitfield. v1 ignores this.
    pub style_bits: u8,
}

/// Trait the GUI calls to syntax-highlight a single line of source.
///
/// Implementations should be cheap to clone — the GUI may hold one per
/// document. The [`NoopHighlighter`] default returns a single span covering
/// the whole line with no style information.
pub trait Highlighter: Send + Sync {
    /// Return the styled spans for `line`. `language_hint` is typically the
    /// file extension (without the dot) and may be empty.
    fn highlight_line(&self, language_hint: &str, line: &str) -> Vec<StyledSpan>;
}

/// No-op highlighter. The default in v1.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopHighlighter;

impl Highlighter for NoopHighlighter {
    fn highlight_line(&self, _language_hint: &str, line: &str) -> Vec<StyledSpan> {
        if line.is_empty() {
            Vec::new()
        } else {
            vec![StyledSpan {
                range: 0..line.len(),
                rgb: 0,
                style_bits: 0,
            }]
        }
    }
}
