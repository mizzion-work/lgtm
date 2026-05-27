//! # lgtm-core
//!
//! Diff engine, file I/O, folder walking, and three-way merge logic for `lgtm`.
//!
//! This crate is deliberately free of UI dependencies. The GUI ([`lgtm-gui`])
//! consumes the data structures defined here and does not reach around them.
//!
//! ## Modules
//!
//! - [`document`] — Loading and saving files with encoding / line-ending awareness.
//! - [`diff`]     — Two-file alignment with gap insertion ([`AlignedDiff`]).
//! - [`merge`]    — Three-way merge model ([`ThreeWayMerge`]).
//! - [`folder`]   — Recursive folder comparison ([`FolderDiff`]).
//! - [`highlight`]— Integration hook for syntax highlighting (no-op in v1).
//! - [`error`]    — Library error type.
//!
//! All public types live in this skeleton with documented signatures; the
//! implementations land in the steps that follow.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod diff;
pub mod document;
pub mod error;
pub mod folder;
pub mod highlight;
pub mod merge;

pub use diff::{
    AlignedDiff, DiffRow, DiffStats, HunkKind, HunkRange, InlineChange, InlineChangeKind, Side,
    unified_diff,
};
pub use document::{DiffDocument, HARD_SIZE_LIMIT, LineEnding, SOFT_SIZE_LIMIT};
pub use error::{Error, Result};
pub use folder::{FolderDiff, FolderDiffOptions, FolderEntry, FolderEntryStatus};
pub use highlight::{Highlighter, NoopHighlighter};
pub use merge::{MergeRegion, ThreeWayMerge};
