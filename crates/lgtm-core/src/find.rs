//! Plain-text "find" over the diff rows.
//!
//! Searches the rendered diff (both panes simultaneously) for occurrences
//! of a query string and returns positioned [`FindMatch`]es the GUI can
//! highlight + cycle through.
//!
//! No regex support in v1 — case-insensitive substring matching only.
//! Empty queries yield an empty result.

use crate::diff::{DiffRow, Side};

/// One hit somewhere inside the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindMatch {
    /// 0-based row index into [`AlignedDiff::rows`](crate::AlignedDiff::rows).
    pub row: usize,
    /// Which pane the hit is on. For `Equal` rows both sides match the
    /// same text — we emit one match per side so navigation lands on
    /// each pane in turn.
    pub side: Side,
    /// Byte range of the hit within the row's text on that side.
    pub range: std::ops::Range<usize>,
}

/// Find every occurrence of `query` (case-insensitive) inside `rows`.
/// Returns an empty Vec for empty queries.
pub fn find_matches(rows: &[DiffRow], query: &str) -> Vec<FindMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let mut out = Vec::new();
    for (row_idx, row) in rows.iter().enumerate() {
        match row {
            DiffRow::Equal { text, .. } => {
                push_hits(&mut out, row_idx, Side::Left, text, &needle);
                push_hits(&mut out, row_idx, Side::Right, text, &needle);
            }
            DiffRow::Delete { text, .. } => {
                push_hits(&mut out, row_idx, Side::Left, text, &needle);
            }
            DiffRow::Insert { text, .. } => {
                push_hits(&mut out, row_idx, Side::Right, text, &needle);
            }
            DiffRow::Replace {
                left_text,
                right_text,
                ..
            } => {
                push_hits(&mut out, row_idx, Side::Left, left_text, &needle);
                push_hits(&mut out, row_idx, Side::Right, right_text, &needle);
            }
            DiffRow::Gap { .. } => {}
        }
    }
    out
}

fn push_hits(out: &mut Vec<FindMatch>, row: usize, side: Side, haystack: &str, needle: &str) {
    if needle.is_empty() || haystack.is_empty() {
        return;
    }
    let hay = haystack.to_lowercase();
    let mut start = 0usize;
    while let Some(pos) = hay[start..].find(needle) {
        let abs = start + pos;
        let end = abs + needle.len();
        out.push(FindMatch {
            row,
            side,
            range: abs..end,
        });
        // Advance by one byte to find overlapping matches too.
        start = abs + 1;
        if start >= hay.len() {
            break;
        }
    }
}

/// Tracks the active find-bar state: query string + current match index.
#[derive(Debug, Default, Clone)]
pub struct FindState {
    /// The text the user has typed into the find bar.
    pub query: String,
    /// Cached matches against the current diff. The GUI should call
    /// [`Self::recompute`] whenever the diff changes or the query changes.
    pub matches: Vec<FindMatch>,
    /// Index into `matches` for "current" highlight. 0-based; wraps.
    pub current: usize,
    /// Whether the find bar is visible.
    pub visible: bool,
}

impl FindState {
    /// Recompute `matches` against `rows`, preserving `current` if still
    /// in range (clamped otherwise).
    pub fn recompute(&mut self, rows: &[DiffRow]) {
        self.matches = find_matches(rows, &self.query);
        if self.matches.is_empty() {
            self.current = 0;
        } else if self.current >= self.matches.len() {
            self.current = self.matches.len() - 1;
        }
    }

    /// Advance `current` by `delta`, wrapping at both ends. No-op when
    /// there are no matches.
    pub fn step(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len() as isize;
        self.current = ((self.current as isize + delta).rem_euclid(n)) as usize;
    }

    /// The currently-focused match, if any.
    pub fn current_match(&self) -> Option<&FindMatch> {
        self.matches.get(self.current)
    }

    /// Number of matches.
    pub fn len(&self) -> usize {
        self.matches.len()
    }

    /// `true` when there are no matches for the current query.
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AlignedDiff;

    fn rows(left: &str, right: &str) -> Vec<DiffRow> {
        AlignedDiff::compute_from_text(left, right).rows
    }

    #[test]
    fn empty_query_yields_no_matches() {
        let r = rows("hello\n", "hello\n");
        assert!(find_matches(&r, "").is_empty());
    }

    #[test]
    fn equal_row_matches_on_both_sides() {
        let r = rows("hello world\n", "hello world\n");
        let m = find_matches(&r, "world");
        assert_eq!(m.len(), 2, "equal rows emit a hit per side");
        assert!(m.iter().any(|f| f.side == Side::Left));
        assert!(m.iter().any(|f| f.side == Side::Right));
        assert_eq!(m[0].range, 6..11);
    }

    #[test]
    fn delete_row_only_matches_on_left() {
        let r = rows("alpha\n", "");
        let m = find_matches(&r, "alpha");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].side, Side::Left);
    }

    #[test]
    fn insert_row_only_matches_on_right() {
        let r = rows("", "alpha\n");
        let m = find_matches(&r, "alpha");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].side, Side::Right);
    }

    #[test]
    fn replace_row_matches_each_side_independently() {
        let r = rows("alpha\n", "beta\n");
        let mut m = find_matches(&r, "alpha");
        m.extend(find_matches(&r, "beta"));
        assert!(m.iter().any(|f| f.side == Side::Left && f.range == (0..5)));
        assert!(m.iter().any(|f| f.side == Side::Right && f.range == (0..4)));
    }

    #[test]
    fn search_is_case_insensitive() {
        let r = rows("HELLO\n", "HELLO\n");
        let m = find_matches(&r, "hello");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn multiple_matches_per_line_are_all_found() {
        let r = rows("ababab\n", "x\n");
        let m = find_matches(&r, "ab");
        // 3 on left + 0 on right (x doesn't match ab) = 3
        assert_eq!(m.iter().filter(|f| f.side == Side::Left).count(), 3);
    }

    #[test]
    fn find_state_step_wraps() {
        let r = rows("ab ab ab\n", "x\n");
        let mut s = FindState {
            query: "ab".into(),
            ..FindState::default()
        };
        s.recompute(&r);
        assert_eq!(s.len(), 3);
        s.step(1);
        assert_eq!(s.current, 1);
        s.step(1);
        assert_eq!(s.current, 2);
        s.step(1);
        assert_eq!(s.current, 0, "wraps forward");
        s.step(-1);
        assert_eq!(s.current, 2, "wraps backward");
    }

    #[test]
    fn find_state_step_noop_with_no_matches() {
        let mut s = FindState::default();
        s.recompute(&[]);
        s.step(1);
        assert_eq!(s.current, 0);
        assert!(s.is_empty());
    }

    #[test]
    fn find_state_recompute_clamps_current_when_matches_shrink() {
        let r1 = rows("ab ab ab\n", "x\n");
        let mut s = FindState {
            query: "ab".into(),
            ..FindState::default()
        };
        s.recompute(&r1);
        s.current = 2;

        // Now run against a row set where "ab" appears only once.
        let r2 = rows("ab x x\n", "y\n");
        s.recompute(&r2);
        assert_eq!(s.len(), 1);
        assert_eq!(s.current, 0, "out-of-range current must be clamped");
    }

    #[test]
    fn current_match_returns_none_when_empty() {
        let s = FindState::default();
        assert!(s.current_match().is_none());
    }

    #[test]
    fn current_match_returns_focused_entry() {
        let r = rows("a a a\n", "x\n");
        let mut s = FindState {
            query: "a".into(),
            ..FindState::default()
        };
        s.recompute(&r);
        s.current = 1;
        let m = s.current_match().unwrap();
        assert_eq!(m.range, 2..3);
    }
}
