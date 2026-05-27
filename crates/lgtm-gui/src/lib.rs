//! egui-based GUI for `lgtm`.
//!
//! The CLI dispatches to one of the entry points in this crate depending on
//! the mode requested. Each entry point owns its own `eframe` app and blocks
//! until the window closes.
//!
//! All view code lives here; `lgtm-core` knows nothing about egui.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use lgtm_core::{DiffDocument, FolderDiff, ThreeWayMerge};

/// Outcome of a GUI session, mapped by the CLI to a process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiOutcome {
    /// Sides were (or became) identical / user saved the merge.
    Identical,
    /// Sides differ / user quit merge without saving.
    Differs,
}

/// Launch the two-file diff window.
pub fn run_diff(
    left: DiffDocument,
    right: DiffDocument,
    read_only: bool,
) -> anyhow::Result<GuiOutcome> {
    let _ = (left, right, read_only);
    todo!("step 4+: minimal egui diff view")
}

/// Launch the three-way merge window. On success, the merged content is
/// written to `output_path`.
pub fn run_merge(merge: ThreeWayMerge, output_path: PathBuf) -> anyhow::Result<GuiOutcome> {
    let _ = (merge, output_path);
    todo!("step 8: merge window")
}

/// Launch the folder-diff window.
pub fn run_folder(diff: FolderDiff) -> anyhow::Result<GuiOutcome> {
    let _ = diff;
    todo!("step 9: folder window")
}
