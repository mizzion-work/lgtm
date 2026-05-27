//! egui-based GUI for `lgtm`.
//!
//! The CLI dispatches to one of the entry points in this crate depending on
//! the mode requested. Each entry point owns its own `eframe` app and blocks
//! until the window closes.
//!
//! All view code lives here; `lgtm-core` knows nothing about egui.

// `deny` (not `forbid`) so individual test modules can opt back in for
// Rust 2024's unsafe env::set_var (used to isolate per-process config).
#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use lgtm_core::{AlignedDiff, DiffDocument, EditorLauncher, FolderDiff, ThreeWayMerge};

mod diff_app;
mod folder_app;
mod graph_panel;
mod menubar;
mod merge_app;
mod theme;

pub use diff_app::DiffApp;
pub use folder_app::{FolderApp, FolderFilters};
pub use menubar::{MenuAction, MenuContext, render_menubar};
pub use merge_app::{MergeApp, MergeExit};

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
///
/// `repo` is the working-tree root for blame and editor-target resolution
/// (typically passed by the user via `--repo "$(git rev-parse --show-toplevel)"`
/// in their git-difftool config). `editor` overrides the env-var-driven
/// editor resolution.
pub fn run_diff(
    left: DiffDocument,
    right: DiffDocument,
    read_only: bool,
    repo: Option<PathBuf>,
    editor: Option<&str>,
) -> anyhow::Result<GuiOutcome> {
    // Binary-vs-binary or differing-sizes short circuit: still open the
    // window so the user can see the "Binary files differ" stub.
    let outcome = if !left.is_binary && !right.is_binary && left.content == right.content {
        GuiOutcome::Identical
    } else {
        GuiOutcome::Differs
    };

    // Record the initial pair into the persistent recents store so a
    // user who opened these files from the CLI can re-open them next
    // session via File → Open Recent.
    let initial = lgtm_core::RecentEntry::new(
        lgtm_core::RecentMode::File,
        left.path.clone(),
        right.path.clone(),
    );
    let mut recents = lgtm_core::RecentList::load();
    recents.push(initial);
    let _ = recents.save();

    let diff = AlignedDiff::compute(&left, &right).with_inline();
    let mut app = DiffApp::new(left, right, diff, read_only);
    if let Some(repo) = repo {
        app = app.with_repo(repo);
    }
    // Editor resolution is best-effort: a missing editor only matters
    // when the user actually presses `e`, so don't fail the window.
    match EditorLauncher::resolve(editor) {
        Ok(launcher) => app = app.with_editor(launcher),
        Err(e) => tracing::debug!("editor unresolved at startup: {e}"),
    }

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
/// written to `output_path` and `Identical` is returned. On abort,
/// `Differs` is returned and `output_path` is left untouched.
pub fn run_merge(merge: ThreeWayMerge, output_path: PathBuf) -> anyhow::Result<GuiOutcome> {
    let app = MergeApp::new(merge, output_path);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lgtm — merge")
            .with_inner_size([1400.0, 900.0]),
        ..Default::default()
    };
    let exit_box: std::sync::Arc<std::sync::Mutex<Option<MergeExit>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let exit_box_inner = exit_box.clone();
    eframe::run_native(
        "lgtm",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(MergeAppWithExit {
                inner: app,
                exit_out: exit_box_inner,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe init failed: {e}"))?;
    let exit = exit_box.lock().unwrap().unwrap_or(MergeExit::Aborted);
    Ok(match exit {
        MergeExit::Saved => GuiOutcome::Identical,
        MergeExit::Aborted => GuiOutcome::Differs,
    })
}

struct MergeAppWithExit {
    inner: MergeApp,
    exit_out: std::sync::Arc<std::sync::Mutex<Option<MergeExit>>>,
}

impl eframe::App for MergeAppWithExit {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        if let Some(exit) = self.inner.exit {
            *self.exit_out.lock().unwrap() = Some(exit);
        }
    }
}

/// Launch the folder-diff window. Returns [`GuiOutcome::Identical`] if the
/// folder diff contains no non-identical entries, else [`GuiOutcome::Differs`].
pub fn run_folder(diff: FolderDiff) -> anyhow::Result<GuiOutcome> {
    let outcome = if diff
        .entries
        .iter()
        .all(|e| e.status == lgtm_core::FolderEntryStatus::Identical)
    {
        GuiOutcome::Identical
    } else {
        GuiOutcome::Differs
    };

    let app = FolderApp::new(diff);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("lgtm — folder diff")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("lgtm", options, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|e| anyhow::anyhow!("eframe init failed: {e}"))?;
    Ok(outcome)
}
