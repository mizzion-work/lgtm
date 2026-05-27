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

use lgtm_core::{AlignedDiff, DiffDocument, FolderDiff, ThreeWayMerge};

mod diff_app;
mod theme;

pub use diff_app::DiffApp;

/// Outcome of a GUI session, mapped by the CLI to a process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiOutcome {
    /// Sides were identical at load, or the merge was saved successfully.
    Identical,
    /// Sides differ, or the user quit merge without saving.
    Differs,
}

/// Launch the two-file diff window.
///
/// Identical files short-circuit without opening a window. The window blocks
/// the calling thread until the user dismisses it.
pub fn run_diff(
    left: DiffDocument,
    right: DiffDocument,
    read_only: bool,
) -> anyhow::Result<GuiOutcome> {
    // Binary-vs-binary or differing-sizes short circuit: still open the
    // window so the user can see the "Binary files differ" stub.
    let outcome = if !left.is_binary && !right.is_binary && left.content == right.content {
        GuiOutcome::Identical
    } else {
        GuiOutcome::Differs
    };

    let diff = AlignedDiff::compute(&left, &right).with_inline();
    let app = DiffApp::new(left, right, diff, read_only);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lgtm")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native("lgtm", options, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|e| anyhow::anyhow!("eframe init failed: {e}"))?;

    Ok(outcome)
}

/// Launch the three-way merge window. On save, the merged content is
/// written to `output_path` and `Identical` is returned.
pub fn run_merge(merge: ThreeWayMerge, output_path: PathBuf) -> anyhow::Result<GuiOutcome> {
    let _ = (merge, output_path);
    todo!("step 8: merge window")
}

/// Launch the folder-diff window.
pub fn run_folder(diff: FolderDiff) -> anyhow::Result<GuiOutcome> {
    let _ = diff;
    todo!("step 9: folder window")
}
