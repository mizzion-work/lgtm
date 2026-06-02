//! Qt-based GUI for `lgtm` — parallel implementation alongside
//! `lgtm-gui` (eframe/egui) and `lgtm-gui-gpui` (Zed's gpui).
//!
//! Status: **experimental.** This crate exists so a Qt migration can land
//! on its own branch without breaking either of the other backends. All
//! three backends consume the same `lgtm-core` types so none of them has
//! to re-implement the diff / blame / find / settings logic.
//!
//! ## What's ported
//!
//! v0: a single read-only diff window with synchronized side-by-side
//! row rendering and the diff color tints. The CLI's `--backend qt` flag
//! routes to this crate; the default stays `eframe` for now.
//!
//! ## What's not ported yet
//!
//! Everything else: edit mode, the menubar, find, blame on hover, the
//! per-hunk copy buttons, the merge / folder windows, the git graph
//! drawer. Each will land as a follow-up patch on this branch.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod diff_view;

pub use diff_view::run_diff;

/// Outcome of a GUI session, mapped by the CLI to a process exit code.
/// Same shape as `lgtm_gui::GuiOutcome` and `lgtm_gui_gpui::GuiOutcome`
/// so the dispatcher can treat all backends uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiOutcome {
    /// Files were identical at load.
    Identical,
    /// Files differed (or the user quit without saving).
    Differs,
}
