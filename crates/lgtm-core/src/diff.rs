//! Two-file diff alignment with gap insertion.
//!
//! The hardest pure-logic problem in `lgtm-core` is producing a row sequence
//! in which the left and right sides line up visually. Each [`DiffRow`] is
//! either a real line on one or both sides, or a [`Gap`](DiffRow::Gap) that
//! pads one side so the other side's content aligns.

use crate::document::DiffDocument;

/// Which pane a piece of content belongs to.
///
/// Reused by the merge module where `Left` maps to `Local` and `Right` maps
/// to `Remote` (`Base` is never a "side" for the purpose of taking changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Left pane (also: LOCAL in three-way merge).
    Left,
    /// Right pane (also: REMOTE in three-way merge).
    Right,
}

/// What kind of inline change applies to a span of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineChangeKind {
    /// Span exists only on the right (inserted).
    Insert,
    /// Span exists only on the left (deleted).
    Delete,
    /// Span is identical on both sides (used for context within a replace).
    Equal,
}

/// A sub-line range to be highlighted inside a [`DiffRow::Replace`].
///
/// `range` is a byte range into the corresponding side's text. The UI uses
/// `side` to decide which sub-string to color.
#[derive(Debug, Clone)]
pub struct InlineChange {
    /// Which side this change applies to.
    pub side: Side,
    /// Byte range within `left_text` or `right_text`.
    pub range: std::ops::Range<usize>,
    /// Insert / Delete / Equal.
    pub kind: InlineChangeKind,
}

/// One visual row of the side-by-side diff.
///
/// Row indices into the parent [`AlignedDiff::rows`] are 0-based and contiguous.
/// `left_line` / `right_line` are 1-based line numbers within the original file.
#[derive(Debug, Clone)]
pub enum DiffRow {
    /// Both sides agree.
    Equal {
        /// 1-based line number on the left.
        left_line: usize,
        /// 1-based line number on the right.
        right_line: usize,
        /// Shared text.
        text: String,
    },
    /// Only the right side has a line here.
    Insert {
        /// 1-based line number on the right.
        right_line: usize,
        /// Inserted text.
        text: String,
    },
    /// Only the left side has a line here.
    Delete {
        /// 1-based line number on the left.
        left_line: usize,
        /// Deleted text.
        text: String,
    },
    /// Both sides have a line here but they differ.
    Replace {
        /// 1-based line number on the left.
        left_line: usize,
        /// 1-based line number on the right.
        right_line: usize,
        /// Text on the left side.
        left_text: String,
        /// Text on the right side.
        right_text: String,
        /// Word/character-level changes inside the row, for both sides.
        inline: Vec<InlineChange>,
    },
    /// A blank row inserted to align the other side.
    Gap {
        /// Which side this blank padding belongs to. (The opposite side has
        /// real content on the same row.)
        side: Side,
    },
}

/// What kind of change a [`HunkRange`] covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    /// Pure insertions (only right side has content).
    Insert,
    /// Pure deletions (only left side has content).
    Delete,
    /// Mixed change including replacements.
    Replace,
}

/// A contiguous run of non-`Equal` rows. Used for hunk navigation and minimap.
#[derive(Debug, Clone, Copy)]
pub struct HunkRange {
    /// First row of the hunk (inclusive) in [`AlignedDiff::rows`].
    pub start_row: usize,
    /// One past the last row (exclusive).
    pub end_row: usize,
    /// The kind of change.
    pub kind: HunkKind,
}

/// Summary statistics for an [`AlignedDiff`].
#[derive(Debug, Clone, Copy, Default)]
pub struct DiffStats {
    /// Number of insert rows.
    pub insertions: usize,
    /// Number of delete rows.
    pub deletions: usize,
    /// Number of replace rows.
    pub replacements: usize,
    /// Number of equal rows.
    pub equal_lines: usize,
}

impl DiffStats {
    /// Total number of non-equal rows. Shown as "N differences" in the status bar.
    pub fn differences(&self) -> usize {
        self.insertions + self.deletions + self.replacements
    }
}

/// A two-file diff laid out as a single ordered sequence of rows with gaps,
/// plus indexed hunks for navigation.
#[derive(Debug, Clone)]
pub struct AlignedDiff {
    /// Rows in display order. Left and right are aligned by row index.
    pub rows: Vec<DiffRow>,
    /// Hunk ranges over `rows`.
    pub hunks: Vec<HunkRange>,
    /// Summary statistics.
    pub stats: DiffStats,
}

impl AlignedDiff {
    /// Compute the aligned diff for two documents.
    ///
    /// Uses [`similar::TextDiff`] under the hood with Myers/Patience and
    /// post-processes the edit script to insert [`DiffRow::Gap`] padding so
    /// corresponding lines render side-by-side.
    ///
    /// Inline (word-level) changes are computed lazily per hunk; callers
    /// that need them eagerly can call [`AlignedDiff::with_inline`] after.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// # use lgtm_core::{DiffDocument, AlignedDiff};
    /// let left  = DiffDocument::load("a.txt")?;
    /// let right = DiffDocument::load("b.txt")?;
    /// let diff  = AlignedDiff::compute(&left, &right);
    /// println!("{} hunks, {} differences", diff.hunks.len(), diff.stats.differences());
    /// # Ok::<(), lgtm_core::Error>(())
    /// ```
    pub fn compute(left: &DiffDocument, right: &DiffDocument) -> Self {
        let _ = (left, right);
        todo!("step 3: alignment + gap insertion algorithm")
    }

    /// Populate [`InlineChange`]s inside [`DiffRow::Replace`] rows.
    pub fn with_inline(self) -> Self {
        todo!("step 3: word-level highlighting via TextDiff::iter_inline_changes")
    }
}
