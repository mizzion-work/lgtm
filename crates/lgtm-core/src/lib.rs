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

// `unsafe` is denied by default; opt back in per-module (currently only
// `editor` needs it, for `setsid()` and Rust 2024's unsafe `env::set_var`).
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod blame;
pub mod diff;
pub mod document;
pub mod editor;
pub mod error;
pub mod folder;
pub mod highlight;
pub mod merge;
pub mod recent;
pub mod settings;

pub use blame::{BLAME_LINE_CAP, BlameCache, BlameInfo};
pub use diff::{
    AlignedDiff, DiffRow, DiffStats, HunkKind, HunkRange, InlineChange, InlineChangeKind, Side,
    extract_lines, splice_lines, unified_diff,
};
pub use document::{DiffDocument, HARD_SIZE_LIMIT, LineEnding, SOFT_SIZE_LIMIT};
pub use editor::{EditorLauncher, LineArgStyle, resolve_real_path};
pub use error::{Error, Result};
pub use folder::{FolderDiff, FolderDiffOptions, FolderEntry, FolderEntryStatus};
pub use highlight::{
    Highlighter, NoopHighlighter, STYLE_BOLD, STYLE_ITALIC, STYLE_UNDERLINE, StyledSpan,
    SyntectHighlighter,
};
pub use merge::{MergeRegion, ThreeWayMerge};
pub use recent::{MAX_RECENT, RecentEntry, RecentList, RecentMode};
pub use settings::{DEFAULT_FONT_SIZE, FONT_SIZE_STEP, MAX_FONT_SIZE, MIN_FONT_SIZE, Settings};
