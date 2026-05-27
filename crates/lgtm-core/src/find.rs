//! Plain-text / regex "find" over the diff rows or raw pane content.
//!
//! Two-mode search:
//!
//! - **Diff-rows mode** (`find_matches_with`): scans the rendered diff
//!   so view-mode hits can be highlighted in the pane row layout. Each
//!   match's `row` field indexes into `AlignedDiff::rows`.
//! - **Content mode** (`find_in_content`): scans the raw text of one
//!   pane so edit-mode hits can be highlighted in the `TextEdit`'s
//!   layouter. The match's `row` field is then the 0-based line index
//!   in that content; `side` records which pane it came from.
//!
//! Both modes honor optional case-sensitivity and an optional regex
//! interpretation of the query.

use crate::diff::{DiffRow, Side};

/// One hit somewhere inside the diff (or one pane's content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindMatch {
    /// In diff-rows mode: 0-based index into
    /// [`AlignedDiff::rows`](crate::AlignedDiff::rows).
    /// In content mode: 0-based line index in the pane's content.
    pub row: usize,
    /// Which pane the hit is on.
    pub side: Side,
    /// Byte range of the hit within the row/line's text on that side.
    pub range: std::ops::Range<usize>,
}

/// Find every plain-substring (case-insensitive) occurrence of `query`
/// inside `rows`. Retained as a tiny wrapper for backwards compatibility
/// with the original API; prefer [`find_matches_with`] for new code.
pub fn find_matches(rows: &[DiffRow], query: &str) -> Vec<FindMatch> {
    find_matches_with(rows, query, false, false)
}

/// Find every occurrence of `query` inside `rows`. When `use_regex` is
/// true `query` is compiled as a `regex::Regex` (case-insensitive flag
/// inverted from `case_sensitive`). Invalid regexes return an empty
/// result so the find bar can keep accepting characters.
pub fn find_matches_with(
    rows: &[DiffRow],
    query: &str,
    case_sensitive: bool,
    use_regex: bool,
) -> Vec<FindMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let matcher = match Matcher::new(query, case_sensitive, use_regex) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for (row_idx, row) in rows.iter().enumerate() {
        match row {
            DiffRow::Equal { text, .. } => {
                matcher.push_hits(&mut out, row_idx, Side::Left, text);
                matcher.push_hits(&mut out, row_idx, Side::Right, text);
            }
            DiffRow::Delete { text, .. } => {
                matcher.push_hits(&mut out, row_idx, Side::Left, text);
            }
            DiffRow::Insert { text, .. } => {
                matcher.push_hits(&mut out, row_idx, Side::Right, text);
            }
            DiffRow::Replace {
                left_text,
                right_text,
                ..
            } => {
                matcher.push_hits(&mut out, row_idx, Side::Left, left_text);
                matcher.push_hits(&mut out, row_idx, Side::Right, right_text);
            }
            DiffRow::Gap { .. } => {}
        }
    }
    out
}

/// Find every occurrence of `query` in `content` (raw pane text), tagged
/// with `side`. Used by edit mode where there are no diff rows to scan.
/// `row` in the returned matches is the 0-based line index.
pub fn find_in_content(
    content: &str,
    side: Side,
    query: &str,
    case_sensitive: bool,
    use_regex: bool,
) -> Vec<FindMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let matcher = match Matcher::new(query, case_sensitive, use_regex) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for (line_idx, line_with_nl) in content.split_inclusive('\n').enumerate() {
        let line = line_with_nl.strip_suffix('\n').unwrap_or(line_with_nl);
        matcher.push_hits(&mut out, line_idx, side, line);
    }
    out
}

/// Internal substring/regex matcher. Built once per find call so we
/// don't recompile the regex per row/line.
enum Matcher {
    Substring {
        needle: String,
        case_sensitive: bool,
    },
    Regex(regex::Regex),
}

impl Matcher {
    fn new(query: &str, case_sensitive: bool, use_regex: bool) -> Option<Self> {
        if use_regex {
            let mut builder = regex::RegexBuilder::new(query);
            builder.case_insensitive(!case_sensitive);
            builder.build().ok().map(Matcher::Regex)
        } else {
            let needle = if case_sensitive {
                query.to_string()
            } else {
                query.to_lowercase()
            };
            Some(Matcher::Substring {
                needle,
                case_sensitive,
            })
        }
    }

    fn push_hits(&self, out: &mut Vec<FindMatch>, row: usize, side: Side, haystack: &str) {
        if haystack.is_empty() {
            return;
        }
        match self {
            Matcher::Substring {
                needle,
                case_sensitive,
            } => {
                let hay_owned;
                let hay: &str = if *case_sensitive {
                    haystack
                } else {
                    hay_owned = haystack.to_lowercase();
                    &hay_owned
                };
                let mut start = 0usize;
                while let Some(pos) = hay[start..].find(needle.as_str()) {
                    let abs = start + pos;
                    let end = abs + needle.len();
                    out.push(FindMatch {
                        row,
                        side,
                        range: abs..end,
                    });
                    start = abs + 1;
                    if start >= hay.len() {
                        break;
                    }
                }
            }
            Matcher::Regex(re) => {
                for m in re.find_iter(haystack) {
                    if m.start() == m.end() {
                        // Skip zero-width matches; otherwise we'd loop.
                        continue;
                    }
                    out.push(FindMatch {
                        row,
                        side,
                        range: m.start()..m.end(),
                    });
                }
            }
        }
    }
}

/// Tracks the active find-bar state: query string + current match index.
#[derive(Debug, Default, Clone)]
pub struct FindState {
    /// The text the user has typed into the find bar.
    pub query: String,
    /// `Aa` toggle.
    pub case_sensitive: bool,
    /// `.*` toggle — interpret `query` as a regex.
    pub use_regex: bool,
    /// Cached matches against the current source. The GUI should call
    /// [`Self::recompute`] or [`Self::recompute_content`] whenever the
    /// source or any flag changes.
    pub matches: Vec<FindMatch>,
    /// Index into `matches` for "current" highlight. 0-based; wraps.
    pub current: usize,
    /// Whether the find bar is visible.
    pub visible: bool,
}

impl FindState {
    /// Recompute against the diff-row view (the source used in view mode).
    pub fn recompute(&mut self, rows: &[DiffRow]) {
        self.matches = find_matches_with(rows, &self.query, self.case_sensitive, self.use_regex);
        self.clamp_current();
    }

    /// Recompute against the raw left + right pane contents (the source
    /// used in edit mode). The two halves are concatenated into a single
    /// match list, sorted by (side, row) so navigation feels natural.
    pub fn recompute_content(&mut self, left: &str, right: &str) {
        let mut a = find_in_content(
            left,
            Side::Left,
            &self.query,
            self.case_sensitive,
            self.use_regex,
        );
        let b = find_in_content(
            right,
            Side::Right,
            &self.query,
            self.case_sensitive,
            self.use_regex,
        );
        a.extend(b);
        self.matches = a;
        self.clamp_current();
    }

    fn clamp_current(&mut self) {
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

    /// `true` when `use_regex` is on and the query fails to compile.
    /// Lets the UI render an "invalid regex" indicator without trying
    /// to do its own pattern validation.
    pub fn regex_invalid(&self) -> bool {
        self.use_regex && !self.query.is_empty() && regex::Regex::new(&self.query).is_err()
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
        assert_eq!(m.len(), 2);
        assert!(m.iter().any(|f| f.side == Side::Left));
        assert!(m.iter().any(|f| f.side == Side::Right));
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
    fn search_is_case_insensitive_by_default() {
        let r = rows("HELLO\n", "HELLO\n");
        let m = find_matches(&r, "hello");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn case_sensitive_misses_what_insensitive_finds() {
        let r = rows("HELLO\n", "hello\n");
        let insensitive = find_matches_with(&r, "hello", false, false);
        assert_eq!(insensitive.len(), 2);
        let sensitive = find_matches_with(&r, "hello", true, false);
        assert_eq!(sensitive.len(), 1);
        assert_eq!(sensitive[0].side, Side::Right);
    }

    #[test]
    fn regex_word_boundary_works() {
        let r = rows("foo foobar\n", "x\n");
        let regex_only = find_matches_with(&r, r"\bfoo\b", false, true);
        // "foo" matches the standalone "foo" but not the "foo" inside "foobar".
        assert_eq!(regex_only.len(), 1);
    }

    #[test]
    fn regex_case_sensitive_flag_honored() {
        let r = rows("FOO\n", "foo\n");
        let insens = find_matches_with(&r, "foo", false, true);
        assert_eq!(insens.len(), 2);
        let sens = find_matches_with(&r, "foo", true, true);
        assert_eq!(sens.len(), 1);
        assert_eq!(sens[0].side, Side::Right);
    }

    #[test]
    fn invalid_regex_returns_empty() {
        let r = rows("anything\n", "anything\n");
        // Unbalanced parenthesis = invalid pattern.
        let m = find_matches_with(&r, "(unclosed", false, true);
        assert!(m.is_empty());
    }

    #[test]
    fn regex_skips_zero_width_matches() {
        // ^ as a regex matches the start of each line — zero width.
        let r = rows("a\n", "b\n");
        let m = find_matches_with(&r, "^", false, true);
        // Skipped to avoid an infinite loop / spurious hits.
        assert!(m.is_empty());
    }

    #[test]
    fn find_in_content_returns_per_line_hits() {
        let content = "alpha\nbeta\nalpha gamma\n";
        let m = find_in_content(content, Side::Left, "alpha", false, false);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].row, 0); // line 0
        assert_eq!(m[1].row, 2); // line 2
        assert!(m.iter().all(|f| f.side == Side::Left));
    }

    #[test]
    fn find_in_content_byte_ranges_are_correct() {
        let m = find_in_content("xx alpha\n", Side::Right, "alpha", false, false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].range, 3..8);
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
        assert_eq!(s.current, 0);
        s.step(-1);
        assert_eq!(s.current, 2);
    }

    #[test]
    fn find_state_recompute_content_uses_left_and_right() {
        let mut s = FindState {
            query: "foo".into(),
            ..FindState::default()
        };
        s.recompute_content("foo bar\n", "baz foo qux foo\n");
        // 1 hit on left + 2 on right = 3 total.
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn find_state_regex_invalid_flag() {
        let mut s = FindState {
            query: "(unclosed".into(),
            use_regex: true,
            ..FindState::default()
        };
        assert!(s.regex_invalid());
        s.use_regex = false;
        assert!(!s.regex_invalid());
        s.use_regex = true;
        s.query = "valid.*".into();
        assert!(!s.regex_invalid());
    }
}
