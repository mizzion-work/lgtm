//! Two-file diff alignment with gap insertion.
//!
//! The hardest pure-logic problem in `lgtm-core` is producing a row sequence
//! in which the left and right sides line up visually. Each [`DiffRow`] is
//! either a real line on one or both sides, or a [`Gap`](DiffRow::Gap) that
//! pads one side so the other side's content aligns.

use similar::{ChangeTag, DiffOp, TextDiff};

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
    /// Span is identical on both sides (context within a replace).
    Equal,
}

/// A sub-line range to be highlighted inside a [`DiffRow::Replace`].
///
/// `range` is a byte range into the corresponding side's text.
#[derive(Debug, Clone, PartialEq, Eq)]
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
///
/// Text payloads include their trailing newline (if any) so trailing-newline
/// differences round-trip; the UI strips it for display.
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
        /// Word-level changes inside the row, for both sides. Empty unless
        /// [`AlignedDiff::with_inline`] has been called.
        inline: Vec<InlineChange>,
    },
    /// A blank row inserted to align the other side. (Currently emitted only
    /// by callers that want manual visual padding — the default
    /// [`AlignedDiff::compute`] uses [`Insert`](DiffRow::Insert) /
    /// [`Delete`](DiffRow::Delete) rows, which already render with the other
    /// side blank.)
    Gap {
        /// Which side is blank. The opposite side has real content here.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    /// First row of the hunk (inclusive) in [`AlignedDiff::rows`].
    pub start_row: usize,
    /// One past the last row (exclusive).
    pub end_row: usize,
    /// The kind of change.
    pub kind: HunkKind,
}

impl HunkRange {
    /// Range of 1-based **left** line numbers covered by this hunk,
    /// end-exclusive. For a pure-Insert hunk (no left content), returns
    /// an empty range at the correct insertion point so it can be passed
    /// straight to [`splice_lines`].
    pub fn left_line_range(&self, rows: &[DiffRow]) -> std::ops::Range<usize> {
        line_range_for_side(self, rows, Side::Left)
    }

    /// Range of 1-based **right** line numbers covered by this hunk,
    /// end-exclusive. See [`left_line_range`](Self::left_line_range) for
    /// pure-Delete behavior.
    pub fn right_line_range(&self, rows: &[DiffRow]) -> std::ops::Range<usize> {
        line_range_for_side(self, rows, Side::Right)
    }
}

fn row_line(row: &DiffRow, side: Side) -> Option<usize> {
    match (row, side) {
        (DiffRow::Equal { left_line, .. }, Side::Left) => Some(*left_line),
        (DiffRow::Equal { right_line, .. }, Side::Right) => Some(*right_line),
        (DiffRow::Delete { left_line, .. }, Side::Left) => Some(*left_line),
        (DiffRow::Insert { right_line, .. }, Side::Right) => Some(*right_line),
        (DiffRow::Replace { left_line, .. }, Side::Left) => Some(*left_line),
        (DiffRow::Replace { right_line, .. }, Side::Right) => Some(*right_line),
        _ => None,
    }
}

fn line_range_for_side(hunk: &HunkRange, rows: &[DiffRow], side: Side) -> std::ops::Range<usize> {
    let mut min: Option<usize> = None;
    let mut max: Option<usize> = None;
    for row in &rows[hunk.start_row..hunk.end_row] {
        if let Some(n) = row_line(row, side) {
            min = Some(min.map_or(n, |m| m.min(n)));
            max = Some(max.map_or(n, |m| m.max(n)));
        }
    }
    if let (Some(lo), Some(hi)) = (min, max) {
        return lo..(hi + 1);
    }
    // No content on this side within the hunk: find the previous line on
    // this side and use the position after it as the insertion point.
    let mut prev: usize = 0;
    for row in &rows[..hunk.start_row] {
        if let Some(n) = row_line(row, side) {
            prev = n;
        }
    }
    let insertion = prev + 1;
    insertion..insertion
}

/// Extract the substring of `content` covering the 1-based, end-exclusive
/// `line_range`. Lines include their trailing `\n`. An empty range
/// returns `""`.
///
/// `line_range.end` past the last line clamps to end-of-content.
pub fn extract_lines(content: &str, line_range: std::ops::Range<usize>) -> String {
    if line_range.start >= line_range.end {
        return String::new();
    }
    let (start, end) = byte_offsets_for(content, line_range);
    content[start..end].to_string()
}

/// Replace lines `line_range` in `content` with `replacement`. An empty
/// `line_range` means "insert at this position". `line_range.end` past
/// EOF clamps to end-of-content.
pub fn splice_lines(
    content: &str,
    line_range: std::ops::Range<usize>,
    replacement: &str,
) -> String {
    let (start, end) = byte_offsets_for(content, line_range);
    let mut out = String::with_capacity(content.len() - (end - start) + replacement.len());
    out.push_str(&content[..start]);
    out.push_str(replacement);
    out.push_str(&content[end..]);
    out
}

/// Convert a 1-based line range (end-exclusive) into the matching byte
/// offsets in `content`. Out-of-range starts/ends clamp to `content.len()`.
/// An empty `line_range` returns `(byte_at_start, byte_at_start)`.
fn byte_offsets_for(content: &str, line_range: std::ops::Range<usize>) -> (usize, usize) {
    if line_range.start == 0 {
        // Line numbers are 1-based; treat 0 like 1 for caller robustness.
        let r = 1..line_range.end.max(1);
        return byte_offsets_for(content, r);
    }
    let target_start = line_range.start;
    let target_end = line_range.end.max(target_start);
    let mut start = content.len();
    let mut end = content.len();
    let mut cursor = 0usize;
    for (i, line) in content.split_inclusive('\n').enumerate() {
        let n = i + 1; // 1-based line number
        if n == target_start {
            start = cursor;
        }
        if n == target_end {
            end = cursor;
        }
        cursor += line.len();
    }
    // If the range starts past EOF, both clamp to EOF (empty insertion).
    if start > end {
        start = end;
    }
    (start, end)
}

/// Summary statistics for an [`AlignedDiff`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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

/// Options controlling [`AlignedDiff::compute_with`] /
/// [`AlignedDiff::compute_from_text_with`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// Treat whitespace-only differences as equal. The displayed text
    /// still shows the original (unstripped) line content.
    pub ignore_whitespace: bool,
}

impl AlignedDiff {
    /// Compute the aligned diff for two documents.
    ///
    /// Uses [`similar::TextDiff::from_lines`] and post-processes the edit
    /// script. Inline (word-level) changes are *not* computed here — call
    /// [`AlignedDiff::with_inline`] to populate them.
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
        Self::compute_from_text(&left.content, &right.content)
    }

    /// Compute with options. Same as [`Self::compute`] when
    /// `opts == DiffOptions::default()`.
    pub fn compute_with(left: &DiffDocument, right: &DiffDocument, opts: DiffOptions) -> Self {
        Self::compute_from_text_with(&left.content, &right.content, opts)
    }

    /// Compute the aligned diff directly from two strings. Used by tests
    /// and by the GUI's debounced re-diff path.
    pub fn compute_from_text(left: &str, right: &str) -> Self {
        Self::compute_from_text_with(left, right, DiffOptions::default())
    }

    /// Same as [`Self::compute_from_text`] but honors `opts`. When
    /// `opts.ignore_whitespace` is true, lines are normalized (all
    /// runs of whitespace collapsed to nothing) for the **equality
    /// test only** — the rows emitted into [`AlignedDiff::rows`] still
    /// carry the original line text so the GUI displays it verbatim.
    pub fn compute_from_text_with(left: &str, right: &str, opts: DiffOptions) -> Self {
        if !opts.ignore_whitespace {
            return Self::compute_from_text_raw(left, right);
        }
        // Normalize for diffing but keep originals for display.
        let left_orig: Vec<&str> = left.split_inclusive('\n').collect();
        let right_orig: Vec<&str> = right.split_inclusive('\n').collect();
        let left_norm: Vec<String> = left_orig.iter().map(|s| strip_ws(s)).collect();
        let right_norm: Vec<String> = right_orig.iter().map(|s| strip_ws(s)).collect();
        let left_norm_refs: Vec<&str> = left_norm.iter().map(String::as_str).collect();
        let right_norm_refs: Vec<&str> = right_norm.iter().map(String::as_str).collect();
        let diff = TextDiff::from_slices(&left_norm_refs, &right_norm_refs);
        Self::build_from_ops(diff.ops(), &left_orig, &right_orig)
    }

    fn compute_from_text_raw(left: &str, right: &str) -> Self {
        let diff = TextDiff::from_lines(left, right);
        let left_lines = diff.old_slices();
        let right_lines = diff.new_slices();
        Self::build_from_ops(diff.ops(), left_lines, right_lines)
    }

    fn build_from_ops(ops: &[DiffOp], left_lines: &[&str], right_lines: &[&str]) -> Self {
        let mut rows: Vec<DiffRow> = Vec::new();
        let mut stats = DiffStats::default();

        for op in ops {
            match *op {
                DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => {
                    for i in 0..len {
                        rows.push(DiffRow::Equal {
                            left_line: old_index + i + 1,
                            right_line: new_index + i + 1,
                            text: left_lines[old_index + i].to_string(),
                        });
                        stats.equal_lines += 1;
                    }
                }
                DiffOp::Delete {
                    old_index, old_len, ..
                } => {
                    for i in 0..old_len {
                        rows.push(DiffRow::Delete {
                            left_line: old_index + i + 1,
                            text: left_lines[old_index + i].to_string(),
                        });
                        stats.deletions += 1;
                    }
                }
                DiffOp::Insert {
                    new_index, new_len, ..
                } => {
                    for i in 0..new_len {
                        rows.push(DiffRow::Insert {
                            right_line: new_index + i + 1,
                            text: right_lines[new_index + i].to_string(),
                        });
                        stats.insertions += 1;
                    }
                }
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    let pair = old_len.min(new_len);
                    for i in 0..pair {
                        rows.push(DiffRow::Replace {
                            left_line: old_index + i + 1,
                            right_line: new_index + i + 1,
                            left_text: left_lines[old_index + i].to_string(),
                            right_text: right_lines[new_index + i].to_string(),
                            inline: Vec::new(),
                        });
                        stats.replacements += 1;
                    }
                    for i in pair..old_len {
                        rows.push(DiffRow::Delete {
                            left_line: old_index + i + 1,
                            text: left_lines[old_index + i].to_string(),
                        });
                        stats.deletions += 1;
                    }
                    for i in pair..new_len {
                        rows.push(DiffRow::Insert {
                            right_line: new_index + i + 1,
                            text: right_lines[new_index + i].to_string(),
                        });
                        stats.insertions += 1;
                    }
                }
            }
        }

        let hunks = compute_hunks(&rows);
        Self { rows, hunks, stats }
    }

    /// Populate [`InlineChange`]s inside each [`DiffRow::Replace`] row,
    /// using word-level diff.
    pub fn with_inline(mut self) -> Self {
        for row in &mut self.rows {
            if let DiffRow::Replace {
                left_text,
                right_text,
                inline,
                ..
            } = row
            {
                *inline = compute_inline(left_text, right_text);
            }
        }
        self
    }
}

/// Render a unified-diff between two strings, used by `lgtm --no-gui`.
///
/// `context_radius` is the number of unchanged lines kept around each hunk
/// (matching `diff -u`'s default of 3).
pub fn unified_diff(
    left: &str,
    right: &str,
    left_label: &str,
    right_label: &str,
    context_radius: usize,
) -> String {
    let diff = TextDiff::from_lines(left, right);
    let mut out = format!("--- {left_label}\n+++ {right_label}\n");
    out.push_str(
        &diff
            .unified_diff()
            .context_radius(context_radius)
            .to_string(),
    );
    out
}

/// Strip all whitespace from a line for ignore-whitespace diffing.
/// Used as a normalization key; the *displayed* text is the original.
fn strip_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compute_inline(left: &str, right: &str) -> Vec<InlineChange> {
    let diff = TextDiff::from_words(left, right);
    let mut left_pos = 0usize;
    let mut right_pos = 0usize;
    let mut out = Vec::new();
    for change in diff.iter_all_changes() {
        let value: &str = change.value();
        let len = value.len();
        match change.tag() {
            ChangeTag::Equal => {
                out.push(InlineChange {
                    side: Side::Left,
                    range: left_pos..left_pos + len,
                    kind: InlineChangeKind::Equal,
                });
                left_pos += len;
                right_pos += len;
            }
            ChangeTag::Delete => {
                out.push(InlineChange {
                    side: Side::Left,
                    range: left_pos..left_pos + len,
                    kind: InlineChangeKind::Delete,
                });
                left_pos += len;
            }
            ChangeTag::Insert => {
                out.push(InlineChange {
                    side: Side::Right,
                    range: right_pos..right_pos + len,
                    kind: InlineChangeKind::Insert,
                });
                right_pos += len;
            }
        }
    }
    out
}

fn compute_hunks(rows: &[DiffRow]) -> Vec<HunkRange> {
    let mut hunks = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if matches!(rows[i], DiffRow::Equal { .. }) {
            i += 1;
            continue;
        }
        let start = i;
        let mut has_insert = false;
        let mut has_delete = false;
        let mut has_replace = false;
        while i < rows.len() {
            match &rows[i] {
                DiffRow::Equal { .. } => break,
                DiffRow::Insert { .. } => has_insert = true,
                DiffRow::Delete { .. } => has_delete = true,
                DiffRow::Replace { .. } => has_replace = true,
                DiffRow::Gap { .. } => {}
            }
            i += 1;
        }
        let kind = if has_replace || (has_insert && has_delete) {
            HunkKind::Replace
        } else if has_insert {
            HunkKind::Insert
        } else {
            HunkKind::Delete
        };
        hunks.push(HunkRange {
            start_row: start,
            end_row: i,
            kind,
        });
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(rows: &[DiffRow]) -> Vec<&'static str> {
        rows.iter()
            .map(|r| match r {
                DiffRow::Equal { .. } => "Eq",
                DiffRow::Insert { .. } => "In",
                DiffRow::Delete { .. } => "De",
                DiffRow::Replace { .. } => "Re",
                DiffRow::Gap { .. } => "Gp",
            })
            .collect()
    }

    #[test]
    fn empty_both_sides() {
        let d = AlignedDiff::compute_from_text("", "");
        assert!(d.rows.is_empty());
        assert!(d.hunks.is_empty());
        assert_eq!(d.stats, DiffStats::default());
    }

    #[test]
    fn identical_files() {
        let d = AlignedDiff::compute_from_text("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(kinds(&d.rows), &["Eq", "Eq", "Eq"]);
        assert!(d.hunks.is_empty());
        assert_eq!(d.stats.differences(), 0);
        assert_eq!(d.stats.equal_lines, 3);
    }

    #[test]
    fn left_empty_right_has_content() {
        let d = AlignedDiff::compute_from_text("", "a\nb\n");
        assert_eq!(kinds(&d.rows), &["In", "In"]);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].kind, HunkKind::Insert);
        assert_eq!(d.stats.insertions, 2);
    }

    #[test]
    fn right_empty_left_has_content() {
        let d = AlignedDiff::compute_from_text("a\nb\n", "");
        assert_eq!(kinds(&d.rows), &["De", "De"]);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].kind, HunkKind::Delete);
        assert_eq!(d.stats.deletions, 2);
    }

    #[test]
    fn single_replacement_in_middle() {
        let d = AlignedDiff::compute_from_text("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(kinds(&d.rows), &["Eq", "Re", "Eq"]);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].start_row, 1);
        assert_eq!(d.hunks[0].end_row, 2);
        assert_eq!(d.hunks[0].kind, HunkKind::Replace);
        assert_eq!(d.stats.replacements, 1);
        if let DiffRow::Replace {
            left_line,
            right_line,
            left_text,
            right_text,
            ..
        } = &d.rows[1]
        {
            assert_eq!(*left_line, 2);
            assert_eq!(*right_line, 2);
            assert_eq!(left_text, "b\n");
            assert_eq!(right_text, "B\n");
        } else {
            panic!("expected Replace");
        }
    }

    #[test]
    fn trailing_newline_difference() {
        let d = AlignedDiff::compute_from_text("a\nb", "a\nb\n");
        assert_eq!(d.stats.differences(), 1);
        let last = d.rows.last().unwrap();
        match last {
            DiffRow::Replace {
                left_text,
                right_text,
                ..
            } => {
                assert_eq!(left_text, "b");
                assert_eq!(right_text, "b\n");
            }
            other => panic!("expected Replace at EOF, got {other:?}"),
        }
    }

    #[test]
    fn unequal_replace_emits_extras() {
        let d = AlignedDiff::compute_from_text("a\nb\n", "X\nY\nZ\n");
        let k = kinds(&d.rows);
        assert_eq!(d.stats.replacements, 2);
        assert_eq!(d.stats.insertions, 1);
        assert_eq!(d.stats.deletions, 0);
        assert!(k.iter().filter(|s| **s == "Re").count() == 2);
        assert!(k.iter().filter(|s| **s == "In").count() == 1);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].kind, HunkKind::Replace);
    }

    #[test]
    fn multiple_hunks() {
        let d = AlignedDiff::compute_from_text(
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n",
            "1\n2\nX\n4\n5\n6\nY\n8\n9\n",
        );
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(d.hunks[0].kind, HunkKind::Replace);
        assert_eq!(d.hunks[1].kind, HunkKind::Replace);
        assert_eq!(d.stats.replacements, 2);
        assert_eq!(d.stats.equal_lines, 7);
    }

    #[test]
    fn line_numbers_are_one_based_and_correct() {
        let d = AlignedDiff::compute_from_text("a\nb\nc\n", "a\nb\nB\nc\n");
        // expect: Eq(1,1) Eq(2,2) In(3) Eq(3,4)
        let mut nums = Vec::new();
        for r in &d.rows {
            match r {
                DiffRow::Equal {
                    left_line,
                    right_line,
                    ..
                } => nums.push(("Eq", *left_line, *right_line)),
                DiffRow::Insert { right_line, .. } => nums.push(("In", 0, *right_line)),
                _ => {}
            }
        }
        assert_eq!(
            nums,
            vec![("Eq", 1, 1), ("Eq", 2, 2), ("In", 0, 3), ("Eq", 3, 4)]
        );
    }

    #[test]
    fn hunks_skip_equal_rows() {
        let d = AlignedDiff::compute_from_text("a\nb\nc\nd\n", "a\nB\nc\nD\n");
        assert_eq!(d.hunks.len(), 2);
        for h in &d.hunks {
            for r in &d.rows[h.start_row..h.end_row] {
                assert!(!matches!(r, DiffRow::Equal { .. }));
            }
        }
    }

    #[test]
    fn with_inline_populates_replace_rows() {
        let d =
            AlignedDiff::compute_from_text("hello world\n", "hello brave world\n").with_inline();
        let replace = d
            .rows
            .iter()
            .find(|r| matches!(r, DiffRow::Replace { .. }))
            .expect("a replace row");
        if let DiffRow::Replace { inline, .. } = replace {
            assert!(!inline.is_empty(), "inline changes should be populated");
            assert!(
                inline
                    .iter()
                    .any(|c| c.kind == InlineChangeKind::Insert && c.side == Side::Right),
                "must mark inserted span on right side"
            );
        }
    }

    #[test]
    fn with_inline_leaves_non_replace_rows_alone() {
        let d = AlignedDiff::compute_from_text("a\nb\n", "a\nb\nc\n").with_inline();
        for r in &d.rows {
            assert!(!matches!(r, DiffRow::Replace { .. }));
        }
    }

    #[test]
    fn differences_counter() {
        let s = DiffStats {
            insertions: 2,
            deletions: 3,
            replacements: 4,
            equal_lines: 100,
        };
        assert_eq!(s.differences(), 9);
    }

    #[test]
    fn pure_insertion_then_pure_deletion_collapse_into_replace_hunk() {
        // adjacent insert+delete should be folded into one Replace hunk by
        // similar's ops emitter; verify hunks reflect that.
        let d = AlignedDiff::compute_from_text("a\nx\nc\n", "a\ny\nc\n");
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].kind, HunkKind::Replace);
    }

    // ---- extract_lines / splice_lines -------------------------------

    #[test]
    fn extract_lines_empty_range_yields_empty_string() {
        assert_eq!(extract_lines("a\nb\nc\n", 2..2), "");
        // Reversed/degenerate range also yields empty (defensive).
        #[allow(clippy::reversed_empty_ranges)]
        let degen = 5..3;
        assert_eq!(extract_lines("a\nb\nc\n", degen), "");
    }

    #[test]
    fn extract_lines_middle_range() {
        assert_eq!(extract_lines("a\nb\nc\nd\n", 2..4), "b\nc\n");
    }

    #[test]
    fn extract_lines_full_file() {
        assert_eq!(extract_lines("a\nb\nc\n", 1..4), "a\nb\nc\n");
    }

    #[test]
    fn extract_lines_past_eof_clamps() {
        assert_eq!(extract_lines("a\nb\n", 2..9999), "b\n");
    }

    #[test]
    fn extract_lines_no_trailing_newline() {
        // last line lacks \n; it must still be extractable.
        assert_eq!(extract_lines("a\nb\nc", 3..4), "c");
        assert_eq!(extract_lines("a\nb\nc", 2..4), "b\nc");
    }

    #[test]
    fn splice_lines_replace_middle() {
        let out = splice_lines("a\nb\nc\nd\n", 2..4, "X\nY\n");
        assert_eq!(out, "a\nX\nY\nd\n");
    }

    #[test]
    fn splice_lines_replace_at_start() {
        let out = splice_lines("a\nb\nc\n", 1..2, "FIRST\n");
        assert_eq!(out, "FIRST\nb\nc\n");
    }

    #[test]
    fn splice_lines_replace_at_end() {
        let out = splice_lines("a\nb\nc\n", 3..4, "LAST\n");
        assert_eq!(out, "a\nb\nLAST\n");
    }

    #[test]
    fn splice_lines_insertion_via_empty_range() {
        // Insert "X\n" before line 2.
        let out = splice_lines("a\nb\nc\n", 2..2, "X\n");
        assert_eq!(out, "a\nX\nb\nc\n");
    }

    #[test]
    fn splice_lines_append_past_eof() {
        // Inserting at a line one past EOF means appending.
        let out = splice_lines("a\nb\n", 3..3, "C\n");
        assert_eq!(out, "a\nb\nC\n");
    }

    #[test]
    fn splice_lines_removal() {
        // Replace with empty: deletes the lines.
        let out = splice_lines("a\nb\nc\n", 2..3, "");
        assert_eq!(out, "a\nc\n");
    }

    #[test]
    fn splice_lines_replacement_without_trailing_newline_preserves_shape() {
        // Replacing the last line of a file without trailing \n.
        let out = splice_lines("a\nb\nc", 3..4, "Z");
        assert_eq!(out, "a\nb\nZ");
    }

    #[test]
    fn extract_then_splice_round_trips() {
        let s = "alpha\nbeta\ngamma\ndelta\n";
        let block = extract_lines(s, 2..4);
        let out = splice_lines(s, 2..4, &block);
        assert_eq!(out, s);
    }

    // ---- HunkRange line ranges --------------------------------------

    #[test]
    fn hunk_left_line_range_replace() {
        let d = AlignedDiff::compute_from_text("a\nb\nc\n", "a\nB\nc\n");
        let h = d.hunks[0];
        assert_eq!(h.left_line_range(&d.rows), 2..3);
        assert_eq!(h.right_line_range(&d.rows), 2..3);
    }

    #[test]
    fn hunk_left_line_range_pure_insert_is_empty_at_insertion_point() {
        let d = AlignedDiff::compute_from_text("a\nb\n", "a\nX\nY\nb\n");
        assert_eq!(d.hunks.len(), 1);
        let h = d.hunks[0];
        // The insert lands between left lines 1 and 2 → empty range at 2..2.
        assert_eq!(h.left_line_range(&d.rows), 2..2);
        assert_eq!(h.right_line_range(&d.rows), 2..4);
    }

    #[test]
    fn hunk_right_line_range_pure_delete_is_empty_at_insertion_point() {
        let d = AlignedDiff::compute_from_text("a\nX\nY\nb\n", "a\nb\n");
        let h = d.hunks[0];
        assert_eq!(h.left_line_range(&d.rows), 2..4);
        assert_eq!(h.right_line_range(&d.rows), 2..2);
    }

    #[test]
    fn hunk_line_range_at_very_start_is_one_one_for_missing_side() {
        // Pure insert at the very top of the file: no prior left content.
        let d = AlignedDiff::compute_from_text("a\n", "X\na\n");
        let h = d.hunks[0];
        assert_eq!(h.left_line_range(&d.rows), 1..1);
        assert_eq!(h.right_line_range(&d.rows), 1..2);
    }

    #[test]
    fn hunk_line_range_at_very_end() {
        let d = AlignedDiff::compute_from_text("a\n", "a\nX\n");
        let h = d.hunks[0];
        assert_eq!(h.left_line_range(&d.rows), 2..2);
        assert_eq!(h.right_line_range(&d.rows), 2..3);
    }

    #[test]
    fn copy_left_to_right_via_extract_and_splice_matches_left() {
        // End-to-end use of the helpers as the GUI will use them.
        let left = "a\nLOCAL\nc\n";
        let right = "a\nREMOTE\nc\n";
        let d = AlignedDiff::compute_from_text(left, right);
        let h = d.hunks[0];
        let lr = h.left_line_range(&d.rows);
        let rr = h.right_line_range(&d.rows);
        let block = extract_lines(left, lr);
        let new_right = splice_lines(right, rr, &block);
        assert_eq!(new_right, "a\nLOCAL\nc\n");
    }

    // ---- DiffOptions::ignore_whitespace ----------------------------

    #[test]
    fn ignore_whitespace_collapses_pure_whitespace_diff_into_equal() {
        let left = "if (x)\n";
        let right = "if   (x)\n";
        let strict = AlignedDiff::compute_from_text(left, right);
        assert_eq!(strict.stats.differences(), 1);
        let lax = AlignedDiff::compute_from_text_with(
            left,
            right,
            DiffOptions {
                ignore_whitespace: true,
            },
        );
        assert_eq!(lax.stats.differences(), 0);
    }

    #[test]
    fn ignore_whitespace_still_surfaces_real_changes() {
        let left = "if (x)\n";
        let right = "if (y)\n";
        let lax = AlignedDiff::compute_from_text_with(
            left,
            right,
            DiffOptions {
                ignore_whitespace: true,
            },
        );
        assert_eq!(lax.stats.differences(), 1);
    }

    #[test]
    fn ignore_whitespace_preserves_original_line_text() {
        // The diff is computed against normalized lines but emitted
        // rows must carry the original (un-stripped) text.
        let left = "  foo\n";
        let right = "foo\n";
        let lax = AlignedDiff::compute_from_text_with(
            left,
            right,
            DiffOptions {
                ignore_whitespace: true,
            },
        );
        assert_eq!(lax.stats.differences(), 0);
        // Find the Equal row and verify the rendered text is preserved.
        if let DiffRow::Equal { text, .. } = &lax.rows[0] {
            assert_eq!(text, "  foo\n");
        } else {
            panic!("expected Equal row, got {:?}", lax.rows[0]);
        }
    }
}
