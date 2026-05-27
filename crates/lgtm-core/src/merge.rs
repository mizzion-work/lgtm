//! Three-way merge model.
//!
//! Given LOCAL, BASE, and REMOTE versions of a file, [`ThreeWayMerge::compute`]
//! partitions the result into [`MergeRegion`]s:
//!
//! - [`Stable`](MergeRegion::Stable): all three versions agree, or both
//!   sides applied the same change.
//! - [`Resolvable`](MergeRegion::Resolvable): exactly one side changed
//!   relative to BASE; the change is taken automatically.
//! - [`Conflict`](MergeRegion::Conflict): both LOCAL and REMOTE changed the
//!   same region differently; the user must resolve.

use std::collections::HashMap;

use similar::{DiffOp, TextDiff};

use crate::diff::Side;
use crate::document::DiffDocument;
use crate::error::{Error, Result};

/// One region of a three-way merge result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeRegion {
    /// LOCAL, BASE, and REMOTE all agree (or both sides made the same edit).
    Stable {
        /// Agreed-upon text.
        text: String,
    },
    /// Exactly one side diverged from BASE; the change is taken automatically.
    Resolvable {
        /// Which side ([`Side::Left`] = LOCAL, [`Side::Right`] = REMOTE)
        /// provided the change.
        take: Side,
        /// The resulting text for this region.
        text: String,
    },
    /// Both LOCAL and REMOTE changed this region differently from BASE.
    /// The user must choose a resolution.
    Conflict {
        /// LOCAL version of the conflicting region.
        local: String,
        /// BASE version of the conflicting region.
        base: String,
        /// REMOTE version of the conflicting region.
        remote: String,
        /// User-provided resolution. `None` means unresolved.
        resolution: Option<String>,
    },
}

impl MergeRegion {
    /// Whether this region requires user attention before saving.
    pub fn is_unresolved_conflict(&self) -> bool {
        matches!(
            self,
            MergeRegion::Conflict {
                resolution: None,
                ..
            }
        )
    }
}

/// The full three-way merge state.
#[derive(Debug, Clone)]
pub struct ThreeWayMerge {
    /// LOCAL version (typically `$LOCAL` from git).
    pub local: DiffDocument,
    /// BASE version — the common ancestor.
    pub base: DiffDocument,
    /// REMOTE version (typically `$REMOTE` from git).
    pub remote: DiffDocument,
    /// Partitioned regions, in document order.
    pub regions: Vec<MergeRegion>,
}

impl ThreeWayMerge {
    /// Compute the three-way merge over three loaded documents.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// # use lgtm_core::{DiffDocument, ThreeWayMerge};
    /// let local  = DiffDocument::load("local.txt")?;
    /// let base   = DiffDocument::load("base.txt")?;
    /// let remote = DiffDocument::load("remote.txt")?;
    /// let merge  = ThreeWayMerge::compute(local, base, remote)?;
    /// println!("{} conflicts", merge.unresolved_conflicts());
    /// # Ok::<(), lgtm_core::Error>(())
    /// ```
    pub fn compute(local: DiffDocument, base: DiffDocument, remote: DiffDocument) -> Result<Self> {
        if local.is_binary || base.is_binary || remote.is_binary {
            return Err(Error::InvalidMergeInputs(
                "binary files cannot be three-way merged".into(),
            ));
        }
        let l_lines: Vec<&str> = local.content.split_inclusive('\n').collect();
        let b_lines: Vec<&str> = base.content.split_inclusive('\n').collect();
        let r_lines: Vec<&str> = remote.content.split_inclusive('\n').collect();
        let regions = compute_regions(&l_lines, &b_lines, &r_lines);
        Ok(Self {
            local,
            base,
            remote,
            regions,
        })
    }

    /// Count of conflict regions still missing a resolution.
    pub fn unresolved_conflicts(&self) -> usize {
        self.regions
            .iter()
            .filter(|r| r.is_unresolved_conflict())
            .count()
    }

    /// Count of conflict regions in total (resolved or not).
    pub fn total_conflicts(&self) -> usize {
        self.regions
            .iter()
            .filter(|r| matches!(r, MergeRegion::Conflict { .. }))
            .count()
    }

    /// Render the merge result as a single string.
    ///
    /// Returns `Err` if any [`MergeRegion::Conflict`] is still unresolved.
    pub fn render(&self) -> Result<String> {
        let mut out = String::new();
        for r in &self.regions {
            match r {
                MergeRegion::Stable { text } => out.push_str(text),
                MergeRegion::Resolvable { text, .. } => out.push_str(text),
                MergeRegion::Conflict {
                    resolution: Some(t),
                    ..
                } => out.push_str(t),
                MergeRegion::Conflict {
                    resolution: None, ..
                } => {
                    return Err(Error::InvalidMergeInputs(
                        "cannot render: unresolved conflict".into(),
                    ));
                }
            }
        }
        Ok(out)
    }

    /// Render the merge result with conflict markers for any unresolved
    /// conflicts. Suitable for headless mode / fallback output.
    pub fn render_with_markers(&self) -> String {
        let mut out = String::new();
        for r in &self.regions {
            match r {
                MergeRegion::Stable { text } => out.push_str(text),
                MergeRegion::Resolvable { text, .. } => out.push_str(text),
                MergeRegion::Conflict {
                    resolution: Some(t),
                    ..
                } => out.push_str(t),
                MergeRegion::Conflict {
                    local,
                    base,
                    remote,
                    resolution: None,
                } => {
                    out.push_str("<<<<<<< LOCAL\n");
                    out.push_str(local);
                    if !local.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("||||||| BASE\n");
                    out.push_str(base);
                    if !base.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("=======\n");
                    out.push_str(remote);
                    if !remote.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(">>>>>>> REMOTE\n");
                }
            }
        }
        out
    }
}

/// Find lines that appear in all three of LOCAL, BASE, REMOTE in the same
/// relative order. Each entry is `(local_idx, base_idx, remote_idx)`.
fn three_way_matches(local: &[&str], base: &[&str], remote: &[&str]) -> Vec<(usize, usize, usize)> {
    let lb = lcs_pairs(local, base);
    let rb = lcs_pairs(remote, base);

    let mut rb_by_base: HashMap<usize, usize> = HashMap::new();
    for (r, b) in &rb {
        rb_by_base.entry(*b).or_insert(*r);
    }

    let mut triples: Vec<(usize, usize, usize)> = Vec::new();
    for (l, b) in lb {
        if let Some(&r) = rb_by_base.get(&b) {
            // enforce monotonicity in (l, b, r)
            let ok = match triples.last() {
                None => true,
                Some((pl, pb, pr)) => l > *pl && b > *pb && r > *pr,
            };
            if ok {
                triples.push((l, b, r));
            }
        }
    }
    triples
}

/// Common pairs from the LCS implicit in `similar`'s edit script.
fn lcs_pairs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let diff = TextDiff::from_slices(a, b);
    let mut out = Vec::new();
    for op in diff.ops() {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for i in 0..*len {
                out.push((old_index + i, new_index + i));
            }
        }
    }
    out
}

fn compute_regions(local: &[&str], base: &[&str], remote: &[&str]) -> Vec<MergeRegion> {
    let mut syncs = three_way_matches(local, base, remote);
    syncs.push((local.len(), base.len(), remote.len()));

    let mut out: Vec<MergeRegion> = Vec::new();
    let mut pl = 0;
    let mut pb = 0;
    let mut pr = 0;
    let mut stable_run = String::new();

    for (li, bi, ri) in syncs {
        let l_text: String = local[pl..li].concat();
        let b_text: String = base[pb..bi].concat();
        let r_text: String = remote[pr..ri].concat();

        if !l_text.is_empty() || !b_text.is_empty() || !r_text.is_empty() {
            // Flush any pending stable run before emitting the non-stable region.
            if !stable_run.is_empty() {
                out.push(MergeRegion::Stable {
                    text: std::mem::take(&mut stable_run),
                });
            }

            let l_eq_b = l_text == b_text;
            let r_eq_b = r_text == b_text;
            let l_eq_r = l_text == r_text;

            if l_eq_b && r_eq_b {
                out.push(MergeRegion::Stable { text: b_text });
            } else if r_eq_b {
                out.push(MergeRegion::Resolvable {
                    take: Side::Left,
                    text: l_text,
                });
            } else if l_eq_b {
                out.push(MergeRegion::Resolvable {
                    take: Side::Right,
                    text: r_text,
                });
            } else if l_eq_r {
                // Both sides made the same change — auto-accept.
                out.push(MergeRegion::Stable { text: l_text });
            } else {
                out.push(MergeRegion::Conflict {
                    local: l_text,
                    base: b_text,
                    remote: r_text,
                    resolution: None,
                });
            }
        }

        if li < local.len() {
            // The sync line itself is stable.
            stable_run.push_str(local[li]);
            pl = li + 1;
            pb = bi + 1;
            pr = ri + 1;
        }
    }

    if !stable_run.is_empty() {
        out.push(MergeRegion::Stable { text: stable_run });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(content: &str) -> DiffDocument {
        let mut d = DiffDocument::empty_for("merge-test");
        d.content = content.into();
        d
    }

    #[test]
    fn all_three_identical_produces_one_stable() {
        let m =
            ThreeWayMerge::compute(doc("a\nb\nc\n"), doc("a\nb\nc\n"), doc("a\nb\nc\n")).unwrap();
        assert_eq!(m.regions.len(), 1);
        assert!(matches!(&m.regions[0], MergeRegion::Stable { text } if text == "a\nb\nc\n"));
        assert_eq!(m.unresolved_conflicts(), 0);
        assert_eq!(m.render().unwrap(), "a\nb\nc\n");
    }

    #[test]
    fn only_local_changed_is_resolvable_left() {
        let m = ThreeWayMerge::compute(doc("a\nLOCAL\nc\n"), doc("a\nb\nc\n"), doc("a\nb\nc\n"))
            .unwrap();
        assert_eq!(m.unresolved_conflicts(), 0);
        assert!(m.regions.iter().any(|r| matches!(
            r,
            MergeRegion::Resolvable {
                take: Side::Left,
                text
            } if text == "LOCAL\n"
        )));
        assert_eq!(m.render().unwrap(), "a\nLOCAL\nc\n");
    }

    #[test]
    fn only_remote_changed_is_resolvable_right() {
        let m = ThreeWayMerge::compute(doc("a\nb\nc\n"), doc("a\nb\nc\n"), doc("a\nREMOTE\nc\n"))
            .unwrap();
        assert_eq!(m.unresolved_conflicts(), 0);
        assert!(m.regions.iter().any(|r| matches!(
            r,
            MergeRegion::Resolvable {
                take: Side::Right,
                text
            } if text == "REMOTE\n"
        )));
        assert_eq!(m.render().unwrap(), "a\nREMOTE\nc\n");
    }

    #[test]
    fn both_sides_make_same_change_is_stable() {
        let m = ThreeWayMerge::compute(doc("a\nNEW\nc\n"), doc("a\nb\nc\n"), doc("a\nNEW\nc\n"))
            .unwrap();
        assert_eq!(m.unresolved_conflicts(), 0);
        assert_eq!(m.render().unwrap(), "a\nNEW\nc\n");
    }

    #[test]
    fn both_sides_make_different_changes_conflicts() {
        let m = ThreeWayMerge::compute(
            doc("a\nLOCAL\nc\n"),
            doc("a\nb\nc\n"),
            doc("a\nREMOTE\nc\n"),
        )
        .unwrap();
        assert_eq!(m.total_conflicts(), 1);
        assert_eq!(m.unresolved_conflicts(), 1);
        assert!(
            m.render().is_err(),
            "render must error on unresolved conflict"
        );
        let with_markers = m.render_with_markers();
        assert!(with_markers.contains("<<<<<<< LOCAL"));
        assert!(with_markers.contains("LOCAL\n"));
        assert!(with_markers.contains("REMOTE\n"));
        assert!(with_markers.contains(">>>>>>> REMOTE"));
    }

    #[test]
    fn resolved_conflict_renders_chosen_text() {
        let mut m = ThreeWayMerge::compute(
            doc("a\nLOCAL\nc\n"),
            doc("a\nb\nc\n"),
            doc("a\nREMOTE\nc\n"),
        )
        .unwrap();
        for r in &mut m.regions {
            if let MergeRegion::Conflict { resolution, .. } = r {
                *resolution = Some("CHOSEN\n".into());
            }
        }
        assert_eq!(m.unresolved_conflicts(), 0);
        assert_eq!(m.render().unwrap(), "a\nCHOSEN\nc\n");
    }

    #[test]
    fn local_inserts_remote_unchanged_is_resolvable_left() {
        let m = ThreeWayMerge::compute(doc("a\nb\nNEW\nc\n"), doc("a\nb\nc\n"), doc("a\nb\nc\n"))
            .unwrap();
        assert_eq!(m.unresolved_conflicts(), 0);
        assert_eq!(m.render().unwrap(), "a\nb\nNEW\nc\n");
    }

    #[test]
    fn disjoint_changes_resolve_independently() {
        let m = ThreeWayMerge::compute(
            doc("LOCAL_HEAD\na\nb\nc\nd\n"),
            doc("HEAD\na\nb\nc\nd\n"),
            doc("HEAD\na\nb\nc\nREMOTE_TAIL\n"),
        )
        .unwrap();
        assert_eq!(m.unresolved_conflicts(), 0);
        assert_eq!(m.render().unwrap(), "LOCAL_HEAD\na\nb\nc\nREMOTE_TAIL\n");
    }

    #[test]
    fn binary_inputs_are_rejected() {
        let mut bin = doc("");
        bin.is_binary = true;
        let err = ThreeWayMerge::compute(bin.clone(), doc(""), doc("")).unwrap_err();
        assert!(matches!(err, Error::InvalidMergeInputs(_)));
    }
}
