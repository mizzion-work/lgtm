//! Recursive folder comparison.
//!
//! Uses `ignore::WalkBuilder` to walk both trees in parallel, applies cheap
//! identity checks (size + mtime) before content comparison, and produces a
//! flat, sorted list of [`FolderEntry`]s the UI can render as a tree.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Per-file comparison outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderEntryStatus {
    /// Same on both sides (by cheap or deep comparison).
    Identical,
    /// Present on both sides but content differs.
    Modified,
    /// Present only on the left.
    LeftOnly,
    /// Present only on the right.
    RightOnly,
    /// One side is a file and the other is a directory (or other type mismatch).
    TypeChanged,
    /// Both sides binary and the contents differ. Distinguished from
    /// `Modified` so the UI can disable double-click-to-diff.
    BinaryDiffers,
}

/// One row of a [`FolderDiff`]. Paths are relative to the respective root.
#[derive(Debug, Clone)]
pub struct FolderEntry {
    /// Path relative to [`FolderDiff::left_root`] / [`FolderDiff::right_root`].
    pub relative_path: PathBuf,
    /// Comparison status.
    pub status: FolderEntryStatus,
    /// Size on the left, if present.
    pub left_size: Option<u64>,
    /// Size on the right, if present.
    pub right_size: Option<u64>,
}

/// Result of comparing two directory trees.
#[derive(Debug, Clone)]
pub struct FolderDiff {
    /// Left root the comparison started from.
    pub left_root: PathBuf,
    /// Right root the comparison started from.
    pub right_root: PathBuf,
    /// Sorted, recursive list of entries.
    pub entries: Vec<FolderEntry>,
}

/// Options for [`FolderDiff::compute`].
#[derive(Debug, Clone, Default)]
pub struct FolderDiffOptions {
    /// Follow symlinks (with cycle detection). Default: `false`.
    pub follow_symlinks: bool,
    /// Honor `.gitignore` / `.ignore` files. Default: `true`.
    pub respect_ignore: bool,
}

impl FolderDiff {
    /// Compute the folder diff between two roots.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// # use lgtm_core::FolderDiff;
    /// # use lgtm_core::folder::FolderDiffOptions;
    /// let diff = FolderDiff::compute("./a", "./b", &FolderDiffOptions::default())?;
    /// println!("{} entries", diff.entries.len());
    /// # Ok::<(), lgtm_core::Error>(())
    /// ```
    pub fn compute(
        left: impl AsRef<Path>,
        right: impl AsRef<Path>,
        options: &FolderDiffOptions,
    ) -> Result<Self> {
        let _ = (left, right, options);
        todo!("step 9: parallel walk + cheap identity check + sort")
    }
}
