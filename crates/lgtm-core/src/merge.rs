//! Three-way merge model.
//!
//! Given LOCAL, BASE, and REMOTE versions of a file, [`ThreeWayMerge::compute`]
//! partitions the result into [`MergeRegion`]s:
//!
//! - [`Stable`](MergeRegion::Stable): all three versions agree.
//! - [`Resolvable`](MergeRegion::Resolvable): exactly one side changed
//!   relative to BASE; the change is taken automatically.
//! - [`Conflict`](MergeRegion::Conflict): both LOCAL and REMOTE changed the
//!   same region; the user must resolve.

use crate::diff::Side;
use crate::document::DiffDocument;
use crate::error::Result;

/// One region of a three-way merge result.
#[derive(Debug, Clone)]
pub enum MergeRegion {
    /// LOCAL, BASE, and REMOTE all agree. The text is emitted as-is.
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
        let _ = (local, base, remote);
        todo!("step 8: three-way merge partitioning")
    }

    /// Count of conflict regions still missing a resolution.
    pub fn unresolved_conflicts(&self) -> usize {
        self.regions
            .iter()
            .filter(|r| r.is_unresolved_conflict())
            .count()
    }

    /// Render the merge result as a single string.
    ///
    /// Returns `Err` if any [`MergeRegion::Conflict`] is still unresolved.
    pub fn render(&self) -> Result<String> {
        todo!("step 8: assemble resolved regions into final text")
    }
}
