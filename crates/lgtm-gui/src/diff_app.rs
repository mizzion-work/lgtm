//! Two-file diff window.

// The test module needs unsafe { env::set_var(...) } (Rust 2024) to
// isolate the recent-files config dir. Everything else stays safe.
#![allow(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align, Color32, FontId, Key, Layout, RichText, ScrollArea, Sense};
use lgtm_core::{
    AlignedDiff, BlameCache, BlameInfo, DiffDocument, DiffRow, EditorLauncher, FindState,
    Highlighter, HunkKind, InlineChangeKind, RecentEntry, RecentList, RecentMode, Settings, Side,
    StyledSpan, SyntectHighlighter, extract_lines, resolve_real_path, splice_lines,
};

use crate::menubar::{self, MenuAction, MenuContext};
use crate::theme;

/// Recompute the diff after this much idle time once an edit has landed.
const DIFF_DEBOUNCE: Duration = Duration::from_millis(150);

/// Direction of a per-hunk Copy action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyDirection {
    /// Copy the LEFT version of the hunk over the RIGHT pane.
    LeftToRight,
    /// Copy the RIGHT version of the hunk over the LEFT pane.
    RightToLeft,
}

/// What the user is currently hovering, for status-bar display and for
/// `e` to know which file + line to open in the editor.
#[derive(Debug, Clone)]
pub struct HoverFocus {
    /// Which side of the diff the cursor is over.
    pub side: Side,
    /// 1-based line number on that side, or `None` if the line has no
    /// concept of a stable line number (replace/gap rows).
    pub line: Option<usize>,
    /// Blame info if we have it, `None` otherwise.
    pub blame: Option<BlameInfo>,
    /// `true` when the row is on a modified/replaced line and blame is
    /// intentionally suppressed.
    pub blame_modified: bool,
}

/// The egui app driving the two-file diff window.
pub struct DiffApp {
    /// Left-side document (LOCAL / `$LEFT`).
    pub left: DiffDocument,
    /// Right-side document (REMOTE / `$RIGHT`).
    pub right: DiffDocument,
    /// Currently rendered diff. Recomputed when edits land (step 7).
    pub diff: AlignedDiff,
    /// Disable both panes if true (set by `--read-only`).
    pub read_only: bool,
    /// Index into [`AlignedDiff::hunks`] for "current hunk" navigation.
    pub current_hunk: usize,
    /// Set when the user presses `Esc` or closes the window.
    pub close_requested: bool,
    /// Edit-mode toggle. View mode (default) shows rendered diff;
    /// edit mode swaps to two TextEdits.
    pub edit_mode: bool,
    /// Whether the left pane has unsaved changes.
    pub modified_left: bool,
    /// Whether the right pane has unsaved changes.
    pub modified_right: bool,
    /// Show a confirm-on-quit modal.
    pub show_confirm_quit: bool,
    /// Request a scroll-to-row on the next frame.
    pending_scroll: Option<usize>,
    /// Wall-clock time of the most recent edit; resets after debounce fires.
    last_edit_at: Option<Instant>,
    /// Shared highlighter (cheap to clone — its assets are a global OnceLock).
    highlighter: Arc<dyn Highlighter>,
    /// Per-line styled spans for the left pane, keyed by 0-based line index.
    cached_left_syntax: Vec<Vec<StyledSpan>>,
    /// Per-line styled spans for the right pane, keyed by 0-based line index.
    cached_right_syntax: Vec<Vec<StyledSpan>>,
    /// Repository root to use for blame lookups. Set by `--repo`, or
    /// discovered automatically per-side if `None`.
    pub repo_root: Option<PathBuf>,
    /// Editor used for `e` / `Shift+E`. Resolved at startup; failures
    /// surface as a status-bar message rather than a crash.
    pub editor: Option<EditorLauncher>,
    /// In difftool mode, the path passed via `$LOCAL` is a temp file —
    /// `real_left` / `real_right` are the working-tree paths to open in
    /// the editor instead (resolved from `--repo` + basename match).
    pub real_left: Option<PathBuf>,
    /// See [`real_left`](Self::real_left).
    pub real_right: Option<PathBuf>,
    /// Blame cache for both panes. Loaded lazily on hover.
    blame_cache: BlameCache,
    /// What the cursor is currently over. Drives status bar + `e` key.
    hover_focus: Option<HoverFocus>,
    /// Transient editor-launch error to render in the status bar.
    editor_status: Option<String>,
    /// In-memory mirror of the on-disk recent-files list.
    recents: RecentList,
    /// User preferences (font size, etc.). Persisted on change.
    pub settings: Settings,
    /// Shared scroll offset for the two edit-mode TextEdits so they
    /// scroll together. Updated every frame from whichever pane the user
    /// scrolled.
    edit_scroll_y: f32,
    /// "About" modal visibility.
    show_about: bool,
    /// "Keyboard Shortcuts" modal visibility.
    show_shortcuts: bool,
    /// Pending Open-Files request, drained after the modal confirms (if
    /// the doc is dirty) and the rfd picker runs. `Some((Files,))` or
    /// `Some((Folders,))` is set by the menu action handler.
    pending_open: Option<OpenRequest>,
    /// Find-bar state (query, matches, current index, visibility).
    pub find: FindState,
    /// Whenever the find bar opens we want the input box focused — set
    /// here and consumed by the next render frame.
    find_focus_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenRequest {
    Files,
    Folders,
}

impl DiffApp {
    /// Construct an app from already-loaded documents and a precomputed diff.
    /// Uses the default [`SyntectHighlighter`] (dark theme); call
    /// [`DiffApp::with_highlighter`] to override (mostly useful for tests).
    pub fn new(
        left: DiffDocument,
        right: DiffDocument,
        diff: AlignedDiff,
        read_only: bool,
    ) -> Self {
        Self::with_highlighter(
            left,
            right,
            diff,
            read_only,
            Arc::new(SyntectHighlighter::dark()),
        )
    }

    /// Construct an app with a custom [`Highlighter`].
    pub fn with_highlighter(
        left: DiffDocument,
        right: DiffDocument,
        diff: AlignedDiff,
        read_only: bool,
        highlighter: Arc<dyn Highlighter>,
    ) -> Self {
        let cached_left_syntax = highlighter.highlight_document(&hint_for(&left), &left.content);
        let cached_right_syntax = highlighter.highlight_document(&hint_for(&right), &right.content);
        Self {
            left,
            right,
            diff,
            read_only,
            current_hunk: 0,
            close_requested: false,
            edit_mode: false,
            modified_left: false,
            modified_right: false,
            show_confirm_quit: false,
            pending_scroll: None,
            last_edit_at: None,
            highlighter,
            cached_left_syntax,
            cached_right_syntax,
            repo_root: None,
            editor: None,
            real_left: None,
            real_right: None,
            blame_cache: BlameCache::new(),
            hover_focus: None,
            editor_status: None,
            recents: RecentList::load(),
            settings: Settings::load(),
            edit_scroll_y: 0.0,
            show_about: false,
            show_shortcuts: false,
            pending_open: None,
            find: FindState::default(),
            find_focus_pending: false,
        }
    }

    /// Builder: set the repository root used for blame lookups. When
    /// `repo_root` is set, paths that match a basename inside the working
    /// tree are also resolved back to that working tree for `e` (so
    /// difftool's temp files open as the real working-tree file).
    pub fn with_repo(mut self, repo: PathBuf) -> Self {
        self.real_left = resolve_real_path(&repo, &self.left.path);
        self.real_right = resolve_real_path(&repo, &self.right.path);
        self.repo_root = Some(repo);
        self
    }

    /// Builder: set the editor used for `e` / `Shift+E`.
    pub fn with_editor(mut self, editor: EditorLauncher) -> Self {
        self.editor = Some(editor);
        self
    }

    /// Replace both documents in-place: reset state, recompute the diff,
    /// rebuild syntax caches, and reset the blame cache (so the new file
    /// pair is blamed against the new repo on next hover).
    pub fn swap_documents(&mut self, left: DiffDocument, right: DiffDocument) {
        // Re-resolve the real working-tree paths against repo_root if set.
        if let Some(repo) = self.repo_root.clone() {
            self.real_left = resolve_real_path(&repo, &left.path);
            self.real_right = resolve_real_path(&repo, &right.path);
        } else {
            self.real_left = None;
            self.real_right = None;
        }
        self.left = left;
        self.right = right;
        self.modified_left = false;
        self.modified_right = false;
        self.current_hunk = 0;
        self.hover_focus = None;
        self.blame_cache = BlameCache::new();
        self.diff = AlignedDiff::compute(&self.left, &self.right).with_inline();
        self.refresh_highlights();
        self.find.recompute(&self.diff.rows);
        // Record into the recent list and persist.
        let entry = RecentEntry::new(
            RecentMode::File,
            self.left.path.clone(),
            self.right.path.clone(),
        );
        self.recents.push(entry);
        let _ = self.recents.save();
    }

    /// Dispatch a [`MenuAction`] emitted by the menubar.
    pub fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::OpenFiles => self.pending_open = Some(OpenRequest::Files),
            MenuAction::OpenFolders => self.pending_open = Some(OpenRequest::Folders),
            MenuAction::OpenRecent(entry) => self.open_recent(entry),
            MenuAction::ClearRecent => {
                self.recents.clear();
                let _ = self.recents.save();
            }
            MenuAction::Save => {
                if let Err(e) = self.save() {
                    tracing::error!("save failed: {e}");
                }
            }
            MenuAction::Quit => {
                self.close_requested = true;
            }
            MenuAction::ToggleEditMode => {
                if !self.read_only {
                    self.edit_mode = !self.edit_mode;
                }
            }
            MenuAction::OpenInEditor => self.launch_editor(false),
            MenuAction::OpenBothInEditor => self.launch_editor(true),
            MenuAction::NextHunk => self.jump_hunk(1),
            MenuAction::PrevHunk => self.jump_hunk(-1),
            MenuAction::FirstHunk => self.jump_first(),
            MenuAction::LastHunk => self.jump_last(),
            MenuAction::ShowShortcuts => self.show_shortcuts = true,
            MenuAction::ShowAbout => self.show_about = true,
            MenuAction::IncreaseFontSize => {
                self.settings.increase_font();
                let _ = self.settings.save();
            }
            MenuAction::DecreaseFontSize => {
                self.settings.decrease_font();
                let _ = self.settings.save();
            }
            MenuAction::ResetFontSize => {
                self.settings.reset_font();
                let _ = self.settings.save();
            }
        }
    }

    fn open_recent(&mut self, entry: RecentEntry) {
        match entry.mode {
            RecentMode::File => {
                let l = match DiffDocument::load(&entry.left) {
                    Ok(d) => d,
                    Err(e) => {
                        self.editor_status =
                            Some(format!("Could not open {}: {e}", entry.left.display()));
                        return;
                    }
                };
                let r = match DiffDocument::load(&entry.right) {
                    Ok(d) => d,
                    Err(e) => {
                        self.editor_status =
                            Some(format!("Could not open {}: {e}", entry.right.display()));
                        return;
                    }
                };
                self.swap_documents(l, r);
            }
            RecentMode::Folder => self.spawn_folder_window(&entry.left, &entry.right),
        }
    }

    /// Spawn a new lgtm process in folder mode. The current DiffApp is a
    /// file-mode window; folder mode is a separate window type, so we
    /// shell out to ourselves rather than restructure.
    fn spawn_folder_window(&mut self, left: &std::path::Path, right: &std::path::Path) {
        let me = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.editor_status = Some(format!("Could not locate lgtm binary: {e}"));
                return;
            }
        };
        if let Err(e) = std::process::Command::new(me)
            .arg("--dir")
            .arg(left)
            .arg(right)
            .spawn()
        {
            self.editor_status = Some(format!("Could not spawn lgtm folder window: {e}"));
            return;
        }
        // Record + persist.
        self.recents.push(RecentEntry::new(
            RecentMode::Folder,
            left.to_path_buf(),
            right.to_path_buf(),
        ));
        let _ = self.recents.save();
    }

    /// Apply a per-hunk copy: rewrite the destination pane so the hunk's
    /// region equals the source pane's region.
    ///
    /// No-op if `hunk_idx` is out of range or `read_only` is true. The
    /// existing debounced re-diff picks up the new content automatically
    /// because [`Self::mark_edited`] sets `last_edit_at`.
    pub fn copy_hunk(&mut self, hunk_idx: usize, dir: CopyDirection) {
        if self.read_only {
            return;
        }
        let Some(hunk) = self.diff.hunks.get(hunk_idx).copied() else {
            return;
        };
        let lr = hunk.left_line_range(&self.diff.rows);
        let rr = hunk.right_line_range(&self.diff.rows);
        match dir {
            CopyDirection::LeftToRight => {
                let block = extract_lines(&self.left.content, lr);
                self.right.content = splice_lines(&self.right.content, rr, &block);
                self.mark_edited(Side::Right);
            }
            CopyDirection::RightToLeft => {
                let block = extract_lines(&self.right.content, rr);
                self.left.content = splice_lines(&self.left.content, lr, &block);
                self.mark_edited(Side::Left);
            }
        }
    }

    /// Launch the configured editor on the currently-focused pane.
    /// If `both` is true, launch once for each pane.
    ///
    /// In difftool mode, `real_left` / `real_right` are preferred over
    /// `left.path` / `right.path` so the user lands on the working-tree
    /// file rather than the temp snapshot. The line passed is the
    /// hovered line on that side when known.
    pub fn launch_editor(&mut self, both: bool) {
        let Some(editor) = self.editor.clone() else {
            self.editor_status = Some("No editor configured (set $EDITOR or --editor)".into());
            return;
        };
        let focus = self.hover_focus.clone();
        let line_on = |side: Side| -> Option<usize> {
            focus
                .as_ref()
                .filter(|f| f.side == side)
                .and_then(|f| f.line)
        };
        let pick = |side: Side| -> (PathBuf, bool) {
            match side {
                Side::Left => match (&self.real_left, &self.left.path) {
                    (Some(p), _) => (p.clone(), false),
                    (None, p) => (p.clone(), self.repo_root.is_some()),
                },
                Side::Right => match (&self.real_right, &self.right.path) {
                    (Some(p), _) => (p.clone(), false),
                    (None, p) => (p.clone(), self.repo_root.is_some()),
                },
            }
        };
        let mut targets: Vec<(Side, PathBuf, bool)> = Vec::new();
        if both {
            let (lp, lt) = pick(Side::Left);
            let (rp, rt) = pick(Side::Right);
            targets.push((Side::Left, lp, lt));
            targets.push((Side::Right, rp, rt));
        } else {
            let side = focus.as_ref().map(|f| f.side).unwrap_or(Side::Left);
            let (p, t) = pick(side);
            targets.push((side, p, t));
        }
        let mut notes: Vec<String> = Vec::new();
        for (side, path, was_temp) in targets {
            if was_temp {
                notes.push(format!(
                    "Opening temp file {} (no working-tree path resolved)",
                    path.display()
                ));
            }
            if let Err(e) = editor.open(&path, line_on(side)) {
                notes.push(format!("Could not launch editor: {e}"));
            }
        }
        self.editor_status = if notes.is_empty() {
            None
        } else {
            Some(notes.join("; "))
        };
    }

    /// Refresh the cached syntax spans for both panes from current content.
    pub fn refresh_highlights(&mut self) {
        self.cached_left_syntax = self
            .highlighter
            .highlight_document(&hint_for(&self.left), &self.left.content);
        self.cached_right_syntax = self
            .highlighter
            .highlight_document(&hint_for(&self.right), &self.right.content);
    }

    /// True if either side has been edited since the last save.
    pub fn is_dirty(&self) -> bool {
        self.modified_left || self.modified_right
    }

    /// Save any modified panes back to their original paths.
    /// Encoding and line-ending restoration is handled by [`DiffDocument::save_to`].
    pub fn save(&mut self) -> anyhow::Result<()> {
        if self.modified_left {
            let p = self.left.path.clone();
            self.left.save_to(p)?;
            self.modified_left = false;
        }
        if self.modified_right {
            let p = self.right.path.clone();
            self.right.save_to(p)?;
            self.modified_right = false;
        }
        Ok(())
    }

    /// Recompute the diff from current pane contents. Cheap enough that we
    /// just call it once the debounce window expires.
    pub fn recompute_diff(&mut self) {
        self.diff = AlignedDiff::compute(&self.left, &self.right).with_inline();
        if self.current_hunk >= self.diff.hunks.len() {
            self.current_hunk = self.diff.hunks.len().saturating_sub(1);
        }
        self.refresh_highlights();
        // The diff changed → find matches must be reindexed.
        self.find.recompute(&self.diff.rows);
    }

    fn mark_edited(&mut self, side: Side) {
        match side {
            Side::Left => self.modified_left = true,
            Side::Right => self.modified_right = true,
        }
        self.last_edit_at = Some(Instant::now());
    }

    /// Move the current hunk by `delta`, wrapping at the ends.
    pub fn jump_hunk(&mut self, delta: isize) {
        let n = self.diff.hunks.len();
        if n == 0 {
            return;
        }
        let new = (self.current_hunk as isize + delta).rem_euclid(n as isize) as usize;
        self.current_hunk = new;
        self.pending_scroll = Some(self.diff.hunks[new].start_row);
    }

    /// Jump to the first hunk.
    pub fn jump_first(&mut self) {
        if self.diff.hunks.is_empty() {
            return;
        }
        self.current_hunk = 0;
        self.pending_scroll = Some(self.diff.hunks[0].start_row);
    }

    /// Jump to the last hunk.
    pub fn jump_last(&mut self) {
        if self.diff.hunks.is_empty() {
            return;
        }
        let i = self.diff.hunks.len() - 1;
        self.current_hunk = i;
        self.pending_scroll = Some(self.diff.hunks[i].start_row);
    }

    fn title(&self) -> String {
        let dirty = if self.is_dirty() { "* " } else { "" };
        format!(
            "{dirty}lgtm — {} ↔ {}",
            short_path(&self.left.path),
            short_path(&self.right.path)
        )
    }

    fn render_status(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("{} differences", self.diff.stats.differences()));
            ui.separator();
            if self.diff.hunks.is_empty() {
                ui.label("0 hunks");
            } else {
                ui.label(format!(
                    "hunk {}/{}",
                    self.current_hunk + 1,
                    self.diff.hunks.len()
                ));
            }
            ui.separator();
            ui.label(format!(
                "{} ↔ {}",
                self.left.encoding.name(),
                self.right.encoding.name()
            ));
            ui.separator();
            ui.label(format!(
                "{:?} ↔ {:?}",
                self.left.line_ending, self.right.line_ending
            ));
            if self.read_only {
                ui.separator();
                ui.label(RichText::new("read-only").color(theme::GUTTER_FG));
            }
            if let Some(err) = &self.editor_status {
                ui.separator();
                ui.label(RichText::new(err).color(Color32::from_rgb(0xff, 0x80, 0x80)));
            }
        });
        // Second line: hovered-blame summary (fallback when tooltip is awkward).
        if let Some(focus) = &self.hover_focus {
            ui.horizontal(|ui| {
                let side_label = match focus.side {
                    Side::Left => "L",
                    Side::Right => "R",
                };
                let line_label = focus
                    .line
                    .map(|n| format!("L{n}"))
                    .unwrap_or_else(|| "-".into());
                ui.label(
                    RichText::new(format!("{side_label}:{line_label}"))
                        .color(theme::GUTTER_FG)
                        .monospace(),
                );
                ui.separator();
                if focus.blame_modified {
                    ui.label(
                        RichText::new("modified line — blame unavailable").color(theme::GUTTER_FG),
                    );
                } else if let Some(info) = &focus.blame {
                    let now = chrono::Local::now().fixed_offset();
                    ui.label(
                        RichText::new(format!(
                            "{}  {}  {}  — {}  ({})",
                            info.short_sha(),
                            info.author_name,
                            info.date_string(),
                            info.short_summary(60),
                            info.relative_to(now),
                        ))
                        .monospace(),
                    );
                } else {
                    ui.label(RichText::new("no blame").color(theme::GUTTER_FG));
                }
            });
        }
    }

    fn render_find_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Find:").color(theme::GUTTER_FG));
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.find.query)
                    .desired_width(280.0)
                    .hint_text("type to search both panes"),
            );
            if std::mem::take(&mut self.find_focus_pending) {
                resp.request_focus();
            }
            if resp.changed() {
                self.find.recompute(&self.diff.rows);
            }
            // Enter / Shift+Enter step through matches even when the
            // input has focus.
            if resp.has_focus() {
                ui.ctx().input(|i| {
                    if i.key_pressed(Key::Enter) {
                        let delta = if i.modifiers.shift { -1 } else { 1 };
                        self.find.step(delta);
                    }
                });
            }
            if ui
                .small_button("◀")
                .on_hover_text("Previous match (Shift+Enter)")
                .clicked()
            {
                self.find.step(-1);
            }
            if ui
                .small_button("▶")
                .on_hover_text("Next match (Enter)")
                .clicked()
            {
                self.find.step(1);
            }
            let count_text = if self.find.query.is_empty() {
                String::from("—")
            } else if self.find.is_empty() {
                String::from("no matches")
            } else {
                format!("{} of {}", self.find.current + 1, self.find.len())
            };
            ui.label(RichText::new(count_text).color(theme::GUTTER_FG));
            if ui
                .small_button("✕")
                .on_hover_text("Close find bar (Esc)")
                .clicked()
            {
                self.find.visible = false;
            }
        });
        // Scroll the diff to the current match's row.
        if let Some(m) = self.find.current_match() {
            self.pending_scroll = Some(m.row);
        }
    }

    fn render_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if !self.read_only
                && ui
                    .button(if self.edit_mode { "View" } else { "Edit" })
                    .clicked()
            {
                self.edit_mode = !self.edit_mode;
            }
            if !self.read_only {
                let save_enabled = self.is_dirty();
                let resp = ui.add_enabled(save_enabled, egui::Button::new("Save (Ctrl+S)"));
                if resp.clicked() {
                    if let Err(e) = self.save() {
                        tracing::error!("save failed: {e}");
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(if self.edit_mode { "edit" } else { "view" });
            });
        });
    }

    fn render_edit_panes(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let half = (avail.x - 8.0) * 0.5;
        let read_only = self.read_only;
        let font_size = self.settings.font_size;
        // Shared scroll offset across both panes for synchronized scrolling.
        // We seed both ScrollAreas with `self.edit_scroll_y`, render them,
        // then read whichever has changed and write the new value back —
        // whichever pane the user scrolled this frame "wins" and the
        // other pane catches up next frame.
        let initial_scroll = self.edit_scroll_y;
        let mut new_scroll = initial_scroll;
        let mut left_edited = false;
        let mut right_edited = false;
        ui.horizontal_top(|ui| {
            // Disjoint borrows: each pane gets its content + its cached
            // syntax spans. The layouter borrows the spans immutably;
            // the TextEdit borrows the content mutably.
            let left_content = &mut self.left.content;
            let left_syntax = &self.cached_left_syntax;
            let mut left_layouter =
                move |ui: &egui::Ui, text: &str, _wrap: f32| -> std::sync::Arc<egui::Galley> {
                    let job = build_edit_layout(text, left_syntax, font_size);
                    ui.fonts(|f| f.layout_job(job))
                };
            ui.allocate_ui(egui::vec2(half, avail.y), |ui| {
                let out = ScrollArea::vertical()
                    .id_salt("lgtm-edit-left")
                    .auto_shrink([false, false])
                    .vertical_scroll_offset(initial_scroll)
                    .show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::multiline(left_content)
                                .font(FontId::monospace(font_size))
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(40)
                                .interactive(!read_only)
                                .layouter(&mut left_layouter),
                        );
                        if resp.changed() {
                            left_edited = true;
                        }
                    });
                if (out.state.offset.y - initial_scroll).abs() > 0.5 {
                    new_scroll = out.state.offset.y;
                }
            });
            ui.separator();
            let right_content = &mut self.right.content;
            let right_syntax = &self.cached_right_syntax;
            let mut right_layouter =
                move |ui: &egui::Ui, text: &str, _wrap: f32| -> std::sync::Arc<egui::Galley> {
                    let job = build_edit_layout(text, right_syntax, font_size);
                    ui.fonts(|f| f.layout_job(job))
                };
            ui.allocate_ui(egui::vec2(half, avail.y), |ui| {
                let out = ScrollArea::vertical()
                    .id_salt("lgtm-edit-right")
                    .auto_shrink([false, false])
                    .vertical_scroll_offset(new_scroll)
                    .show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::multiline(right_content)
                                .font(FontId::monospace(font_size))
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(40)
                                .interactive(!read_only)
                                .layouter(&mut right_layouter),
                        );
                        if resp.changed() {
                            right_edited = true;
                        }
                    });
                if (out.state.offset.y - new_scroll).abs() > 0.5 {
                    new_scroll = out.state.offset.y;
                }
            });
        });
        self.edit_scroll_y = new_scroll;
        if left_edited {
            self.mark_edited(Side::Left);
        }
        if right_edited {
            self.mark_edited(Side::Right);
        }
    }

    fn render_diff(&mut self, ui: &mut egui::Ui) {
        if self.edit_mode {
            self.render_edit_panes(ui);
            return;
        }
        if self.left.is_binary || self.right.is_binary {
            render_binary_stub(ui, &self.left, &self.right);
            return;
        }

        let font_size = self.settings.font_size;
        let row_height = font_size + 4.0;
        let total = self.diff.rows.len();
        let pending = self.pending_scroll.take();

        let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(row) = pending {
            // Place the target row a third of the way down the viewport.
            scroll = scroll.vertical_scroll_offset((row as f32 * row_height) - 60.0);
        }

        // Build a row_idx → Option<hunk_idx> lookup so render_row_with_blame
        // can show Copy buttons only on the first row of each hunk.
        let mut hunk_at_row_start: Vec<Option<usize>> = vec![None; total];
        for (i, hunk) in self.diff.hunks.iter().enumerate() {
            if let Some(slot) = hunk_at_row_start.get_mut(hunk.start_row) {
                *slot = Some(i);
            }
        }

        // Disjoint borrows so the show_rows closure can mutate blame_cache
        // while reading the diff + syntax caches + paths.
        let diff = &self.diff;
        let left_syntax = &self.cached_left_syntax;
        let right_syntax = &self.cached_right_syntax;
        let blame_cache = &mut self.blame_cache;
        let left_path = self.left.path.clone();
        let right_path = self.right.path.clone();
        let repo_root = self.repo_root.clone();
        let read_only = self.read_only;
        let find_matches = &self.find.matches;
        let find_current = self.find.current;
        let mut hover_focus: Option<HoverFocus> = None;
        let mut pending_copy: Option<(usize, CopyDirection)> = None;

        scroll.show_rows(ui, row_height, total, |ui, row_range| {
            ui.style_mut().override_font_id = Some(FontId::monospace(font_size));
            for idx in row_range {
                let row = &diff.rows[idx];
                let hunk_idx = hunk_at_row_start.get(idx).copied().flatten();
                render_row_with_blame(
                    ui,
                    row,
                    row_height,
                    left_syntax,
                    right_syntax,
                    blame_cache,
                    repo_root.as_deref(),
                    &left_path,
                    &right_path,
                    &mut hover_focus,
                    hunk_idx,
                    read_only,
                    &mut pending_copy,
                    font_size,
                    idx,
                    find_matches,
                    find_current,
                );
            }
        });

        self.hover_focus = hover_focus;
        if let Some((i, dir)) = pending_copy {
            self.copy_hunk(i, dir);
        }
    }

    fn render_minimap(&mut self, ui: &mut egui::Ui) {
        let total = self.diff.rows.len().max(1) as f32;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ui.available_height()),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(0x14));
        for (i, hunk) in self.diff.hunks.iter().enumerate() {
            let y0 = rect.top() + (hunk.start_row as f32 / total) * rect.height();
            let y1 = rect.top() + (hunk.end_row as f32 / total) * rect.height();
            let h = (y1 - y0).max(2.0);
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, y0),
                egui::vec2(rect.width() - 4.0, h),
            );
            let color = match hunk.kind {
                HunkKind::Insert => theme::INSERT_BG,
                HunkKind::Delete => theme::DELETE_BG,
                HunkKind::Replace => theme::REPLACE_BG,
            };
            painter.rect_filled(bar, 0.0, color);
            if i == self.current_hunk {
                painter.rect_stroke(bar, 0.0, egui::Stroke::new(1.5, Color32::WHITE));
            }
        }
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let frac = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                let target_row = (frac * total) as usize;
                // pick the nearest hunk to that row
                if let Some((idx, h)) = self
                    .diff
                    .hunks
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, h)| h.start_row.abs_diff(target_row))
                {
                    self.current_hunk = idx;
                    self.pending_scroll = Some(h.start_row);
                }
            }
        }
    }
}

impl eframe::App for DiffApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keyboard shortcuts. Hunk-nav keys are inactive in edit mode so
        // they don't fight with the TextEdit consuming `n` / `p`.
        let mut delta: isize = 0;
        let mut first = false;
        let mut last = false;
        let mut want_save = false;
        let mut want_open: Option<bool> = None; // Some(true) = both panes
        let mut want_open_files = false;
        let mut want_open_folders = false;
        ctx.input(|i| {
            if i.modifiers.command_only() && i.key_pressed(Key::S) {
                want_save = true;
            }
            if i.modifiers.command_only() && i.key_pressed(Key::O) {
                want_open_files = true;
            }
            if i.modifiers.command && i.modifiers.shift && i.key_pressed(Key::O) {
                want_open_folders = true;
            }
            if i.modifiers.command_only() && i.key_pressed(Key::Q) {
                self.close_requested = true;
            }
            if i.modifiers.command_only() && i.key_pressed(Key::Equals) {
                self.settings.increase_font();
                let _ = self.settings.save();
            }
            if i.modifiers.command_only() && i.key_pressed(Key::Minus) {
                self.settings.decrease_font();
                let _ = self.settings.save();
            }
            if i.modifiers.command_only() && i.key_pressed(Key::Num0) {
                self.settings.reset_font();
                let _ = self.settings.save();
            }
            if i.modifiers.command_only() && i.key_pressed(Key::F) {
                self.find.visible = true;
                self.find_focus_pending = true;
            }
            if i.key_pressed(Key::Escape) {
                // Esc dismisses the find bar first, falling through to
                // window close only when find isn't active.
                if self.find.visible {
                    self.find.visible = false;
                } else {
                    self.close_requested = true;
                }
            }
            if !self.edit_mode {
                if i.key_pressed(Key::Q) && !i.modifiers.command {
                    self.close_requested = true;
                }
                if i.key_pressed(Key::N) {
                    delta = 1;
                }
                if i.key_pressed(Key::P) {
                    delta = -1;
                }
                if i.modifiers.ctrl && i.key_pressed(Key::Home) {
                    first = true;
                }
                if i.modifiers.ctrl && i.key_pressed(Key::End) {
                    last = true;
                }
                if i.key_pressed(Key::E) {
                    want_open = Some(i.modifiers.shift);
                }
            }
        });
        if let Some(both) = want_open {
            self.launch_editor(both);
        }
        if want_open_files {
            self.pending_open = Some(OpenRequest::Files);
        }
        if want_open_folders {
            self.pending_open = Some(OpenRequest::Folders);
        }
        if want_save {
            if let Err(e) = self.save() {
                tracing::error!("save failed: {e}");
            }
        }
        if delta != 0 {
            self.jump_hunk(delta);
        }
        if first {
            self.jump_first();
        }
        if last {
            self.jump_last();
        }

        if self.close_requested {
            if self.is_dirty() {
                self.show_confirm_quit = true;
                self.close_requested = false;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        // Debounced re-diff: if enough idle time has passed since the last
        // edit, refresh the diff state and clear the timer.
        if let Some(t) = self.last_edit_at {
            if t.elapsed() >= DIFF_DEBOUNCE {
                self.recompute_diff();
                self.last_edit_at = None;
            } else {
                // Wake the UI up when the debounce expires.
                ctx.request_repaint_after(DIFF_DEBOUNCE - t.elapsed());
            }
        }

        // Menubar (File / Edit / View / Help) sits above the title.
        let mut actions: Vec<MenuAction> = Vec::new();
        egui::TopBottomPanel::top("lgtm-menubar").show(ctx, |ui| {
            let mctx = MenuContext {
                dirty: self.is_dirty(),
                supports_edit_mode: true,
                in_edit_mode: self.edit_mode,
                supports_hunk_nav: true,
                supports_editor: self.editor.is_some(),
                read_only: self.read_only,
            };
            crate::menubar::render_menubar(ui, mctx, &self.recents, &mut actions);
        });
        for action in actions {
            self.handle_menu_action(action);
        }

        // Drain any pending Open Files / Open Folders request raised
        // from the menu. Native dialogs are blocking but the user-perceived
        // wait is acceptable for a click-driven flow.
        if let Some(req) = self.pending_open.take() {
            self.run_open_dialog(req);
        }

        if self.find.visible {
            egui::TopBottomPanel::top("lgtm-find").show(ctx, |ui| {
                self.render_find_bar(ui);
            });
        }

        egui::TopBottomPanel::top("lgtm-title").show(ctx, |ui| {
            ui.heading(self.title());
            self.render_toolbar(ui);
        });
        egui::TopBottomPanel::bottom("lgtm-status").show(ctx, |ui| {
            self.render_status(ui);
        });
        egui::SidePanel::right("lgtm-minimap")
            .exact_width(20.0)
            .resizable(false)
            .show(ctx, |ui| {
                self.render_minimap(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_diff(ui);
        });

        if self.show_confirm_quit {
            self.render_confirm_quit(ctx);
        }
        if self.show_about {
            self.render_about(ctx);
        }
        if self.show_shortcuts {
            self.render_shortcuts(ctx);
        }
    }
}

impl DiffApp {
    fn run_open_dialog(&mut self, req: OpenRequest) {
        match req {
            OpenRequest::Files => {
                let left = match rfd::FileDialog::new()
                    .set_title("lgtm — pick LEFT file")
                    .pick_file()
                {
                    Some(p) => p,
                    None => return,
                };
                let right = match rfd::FileDialog::new()
                    .set_title("lgtm — pick RIGHT file")
                    .pick_file()
                {
                    Some(p) => p,
                    None => return,
                };
                let l = match DiffDocument::load(&left) {
                    Ok(d) => d,
                    Err(e) => {
                        self.editor_status =
                            Some(format!("Could not open {}: {e}", left.display()));
                        return;
                    }
                };
                let r = match DiffDocument::load(&right) {
                    Ok(d) => d,
                    Err(e) => {
                        self.editor_status =
                            Some(format!("Could not open {}: {e}", right.display()));
                        return;
                    }
                };
                self.swap_documents(l, r);
            }
            OpenRequest::Folders => {
                let left = match rfd::FileDialog::new()
                    .set_title("lgtm — pick LEFT folder")
                    .pick_folder()
                {
                    Some(p) => p,
                    None => return,
                };
                let right = match rfd::FileDialog::new()
                    .set_title("lgtm — pick RIGHT folder")
                    .pick_folder()
                {
                    Some(p) => p,
                    None => return,
                };
                self.spawn_folder_window(&left, &right);
            }
        }
    }

    fn render_about(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("About lgtm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(RichText::new(menubar::about_body()).monospace());
                ui.separator();
                ui.label("the diff tool that lets you say lgtm with confidence.");
                if ui.button("OK").clicked() {
                    self.show_about = false;
                }
            });
        if !open {
            self.show_about = false;
        }
    }

    fn render_shortcuts(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Keyboard Shortcuts")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(RichText::new(menubar::shortcuts_body()).monospace());
                if ui.button("OK").clicked() {
                    self.show_shortcuts = false;
                }
            });
        if !open {
            self.show_shortcuts = false;
        }
    }

    fn render_confirm_quit(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. What now?");
                ui.horizontal(|ui| {
                    if ui.button("Save and quit").clicked() {
                        if let Err(e) = self.save() {
                            tracing::error!("save failed: {e}");
                        } else {
                            self.show_confirm_quit = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if ui.button("Quit without saving").clicked() {
                        self.show_confirm_quit = false;
                        self.modified_left = false;
                        self.modified_right = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_confirm_quit = false;
                    }
                });
            });
        if !open {
            self.show_confirm_quit = false;
        }
    }
}

fn render_binary_stub(ui: &mut egui::Ui, left: &DiffDocument, right: &DiffDocument) {
    ui.centered_and_justified(|ui| {
        ui.vertical(|ui| {
            ui.heading("Binary files differ");
            ui.label(format!(
                "left:  {} ({} bytes)",
                left.path.display(),
                left.size_bytes
            ));
            ui.label(format!(
                "right: {} ({} bytes)",
                right.path.display(),
                right.size_bytes
            ));
        });
    });
}

/// Width of the center column hosting per-hunk Copy buttons.
const CENTER_COL_WIDTH: f32 = 56.0;

#[allow(clippy::too_many_arguments)]
fn render_row_with_blame(
    ui: &mut egui::Ui,
    row: &DiffRow,
    row_height: f32,
    left_syntax: &[Vec<StyledSpan>],
    right_syntax: &[Vec<StyledSpan>],
    blame_cache: &mut BlameCache,
    repo_root: Option<&std::path::Path>,
    left_path: &std::path::Path,
    right_path: &std::path::Path,
    hover_focus: &mut Option<HoverFocus>,
    hunk_idx: Option<usize>,
    read_only: bool,
    pending_copy: &mut Option<(usize, CopyDirection)>,
    font_size: f32,
    row_idx: usize,
    find_matches: &[lgtm_core::FindMatch],
    find_current: usize,
) {
    let bg = row_background(row);
    let avail = ui.available_width();
    let half = (avail - CENTER_COL_WIDTH) * 0.5;

    let (rect, _resp) = ui.allocate_exact_size(egui::vec2(avail, row_height), Sense::hover());
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, bg);
    }

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );

    let parts = decompose(row);
    let left_spans = parts
        .left_num
        .and_then(|n| left_syntax.get(n.saturating_sub(1)).map(|v| v.as_slice()))
        .unwrap_or(&[]);
    let right_spans = parts
        .right_num
        .and_then(|n| right_syntax.get(n.saturating_sub(1)).map(|v| v.as_slice()))
        .unwrap_or(&[]);

    // Collect find-match ranges scoped to this (row, side). The current
    // match is flagged so build_layout can render it in a brighter color.
    let mut left_finds: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    let mut right_finds: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    for (i, m) in find_matches.iter().enumerate() {
        if m.row != row_idx {
            continue;
        }
        let is_current = i == find_current;
        match m.side {
            Side::Left => left_finds.push((m.range.clone(), is_current)),
            Side::Right => right_finds.push((m.range.clone(), is_current)),
        }
    }
    let left_resp = render_pane(
        &mut child,
        half,
        parts.left_num,
        parts.left_text,
        Side::Left,
        &parts.inline_left,
        left_spans,
        &left_finds,
        font_size,
    );
    render_center_column(&mut child, row_height, hunk_idx, read_only, pending_copy);
    let right_resp = render_pane(
        &mut child,
        half,
        parts.right_num,
        parts.right_text,
        Side::Right,
        &parts.inline_right,
        right_spans,
        &right_finds,
        font_size,
    );

    // Blame on hover. Replace rows are "modified (no blame)" by design —
    // line N in the view doesn't correspond to line N in the committed
    // file once both sides have changed.
    let is_replace = matches!(row, DiffRow::Replace { .. });
    let mut blame_resp =
        |resp: egui::Response, side: Side, line: Option<usize>, file: &std::path::Path| {
            if !resp.hovered() {
                return;
            }
            if is_replace || line.is_none() {
                *hover_focus = Some(HoverFocus {
                    side,
                    line,
                    blame: None,
                    blame_modified: true,
                });
                resp.on_hover_text("modified line — blame unavailable");
                return;
            }
            let line = line.unwrap();
            let root = repo_root.unwrap_or(file);
            // Lazy load. Silent failure: if blame isn't available, we just
            // don't show a tooltip.
            let _ = blame_cache.load(root, file);
            let info = blame_cache.get(root, file, line).cloned();
            *hover_focus = Some(HoverFocus {
                side,
                line: Some(line),
                blame: info.clone(),
                blame_modified: false,
            });
            if let Some(info) = info {
                let now = chrono::Local::now().fixed_offset();
                let tooltip = format!(
                    "{}  {}  {}\n{}\n\n{}",
                    info.short_sha(),
                    info.author_name,
                    info.date_string(),
                    info.short_summary(60),
                    info.relative_to(now),
                );
                resp.on_hover_text(tooltip);
            }
        };
    blame_resp(left_resp, Side::Left, parts.left_num, left_path);
    blame_resp(right_resp, Side::Right, parts.right_num, right_path);
}

type InlineSpans = Vec<(std::ops::Range<usize>, InlineChangeKind)>;

struct RowParts<'a> {
    left_num: Option<usize>,
    left_text: &'a str,
    right_num: Option<usize>,
    right_text: &'a str,
    inline_left: InlineSpans,
    inline_right: InlineSpans,
}

#[allow(clippy::too_many_arguments)]
fn render_pane(
    ui: &mut egui::Ui,
    width: f32,
    line_num: Option<usize>,
    text: &str,
    side: Side,
    inline: &[(std::ops::Range<usize>, InlineChangeKind)],
    syntax: &[StyledSpan],
    find_ranges: &[(std::ops::Range<usize>, bool)],
    font_size: f32,
) -> egui::Response {
    let resp = ui.scope(|ui| {
        ui.set_max_width(width);
        ui.horizontal(|ui| {
            let gutter = match line_num {
                Some(n) => format!("{n:>width$}", width = theme::GUTTER_WIDTH_CHARS),
                None => " ".repeat(theme::GUTTER_WIDTH_CHARS),
            };
            ui.label(
                RichText::new(gutter)
                    .color(theme::GUTTER_FG)
                    .font(FontId::monospace(font_size)),
            );
            let layout = build_layout(strip_nl(text), syntax, inline, find_ranges, side, font_size);
            ui.label(layout);
        });
    });
    // Promote the scope to a hover-sensing rect.
    let r = resp.response.rect;
    let side_tag = if matches!(side, Side::Left) { 0u8 } else { 1u8 };
    ui.interact(
        r,
        ui.id().with(("pane", line_num, side_tag)),
        Sense::hover(),
    )
}

/// Render the fixed-width column between the panes. On the first row of
/// each hunk (when not read-only), draws two compact Copy buttons; on
/// every other row, a thin vertical separator. The two-state design
/// keeps the visual rhythm of the diff unbroken while making every
/// hunk one click away from being accepted on either side.
fn render_center_column(
    ui: &mut egui::Ui,
    row_height: f32,
    hunk_idx: Option<usize>,
    read_only: bool,
    pending_copy: &mut Option<(usize, CopyDirection)>,
) {
    let (rect, _resp) =
        ui.allocate_exact_size(egui::vec2(CENTER_COL_WIDTH, row_height), Sense::hover());
    // Vertical separator backdrop, drawn first so buttons sit on top.
    let center_x = rect.center().x;
    ui.painter().line_segment(
        [
            egui::pos2(center_x, rect.top()),
            egui::pos2(center_x, rect.bottom()),
        ],
        egui::Stroke::new(1.0, Color32::from_gray(0x40)),
    );

    let Some(idx) = hunk_idx else {
        return;
    };
    if read_only {
        return;
    }

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 2.0;
    let right_btn = child
        .add(egui::Button::new(RichText::new("→").monospace()).small())
        .on_hover_text("Copy this hunk: LEFT → RIGHT");
    if right_btn.clicked() {
        *pending_copy = Some((idx, CopyDirection::LeftToRight));
    }
    let left_btn = child
        .add(egui::Button::new(RichText::new("←").monospace()).small())
        .on_hover_text("Copy this hunk: RIGHT → LEFT");
    if left_btn.clicked() {
        *pending_copy = Some((idx, CopyDirection::RightToLeft));
    }
}

/// Build a [`LayoutJob`] for the **whole document** shown in edit mode's
/// `TextEdit`. Applies the per-line `syntax` spans we already keep in
/// `DiffApp::cached_*_syntax`; falls back to plain text for any line
/// whose cached spans no longer fit (the user just typed there and the
/// debounced re-highlight hasn't fired yet) or for which we have no
/// cache entry at all (a line that was just added).
fn build_edit_layout(
    text: &str,
    syntax: &[Vec<StyledSpan>],
    font_size: f32,
) -> egui::text::LayoutJob {
    let font = FontId::monospace(font_size);
    let plain = egui::TextFormat {
        font_id: font.clone(),
        // PLACEHOLDER tells egui "use the surrounding text color" — i.e.
        // honor the user's light/dark theme rather than baking in gray.
        color: Color32::PLACEHOLDER,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    for (line_idx, line_with_nl) in text.split_inclusive('\n').enumerate() {
        let has_nl = line_with_nl.ends_with('\n');
        let line = if has_nl {
            &line_with_nl[..line_with_nl.len() - 1]
        } else {
            line_with_nl
        };
        let spans = syntax.get(line_idx).map(Vec::as_slice).unwrap_or(&[]);
        let line_len = line.len();
        let max_end = spans.iter().map(|s| s.range.end).max().unwrap_or(0);
        let stale_or_empty = spans.is_empty() || max_end > line_len;
        if stale_or_empty {
            job.append(line, 0.0, plain.clone());
            if has_nl {
                job.append("\n", 0.0, plain.clone());
            }
            continue;
        }
        let mut cursor = 0usize;
        for span in spans {
            let start = span.range.start.min(line_len);
            let end = span.range.end.min(line_len);
            if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                continue;
            }
            if start > cursor {
                job.append(&line[cursor..start], 0.0, plain.clone());
            }
            let r = ((span.rgb >> 16) & 0xff) as u8;
            let g = ((span.rgb >> 8) & 0xff) as u8;
            let b = (span.rgb & 0xff) as u8;
            let color = if r == 0 && g == 0 && b == 0 {
                Color32::PLACEHOLDER
            } else {
                Color32::from_rgb(r, g, b)
            };
            let fmt = egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            };
            if end > start {
                job.append(&line[start..end], 0.0, fmt);
            }
            cursor = end;
        }
        if cursor < line_len {
            job.append(&line[cursor..], 0.0, plain.clone());
        }
        if has_nl {
            job.append("\n", 0.0, plain.clone());
        }
    }
    job
}

/// Build a [`LayoutJob`] for one displayed line that combines syntax
/// foregrounds (from `syntax`) and inline-change backgrounds (from `inline`).
///
/// The two span sets are independently produced (syntect vs. similar) so we
/// walk a sorted union of their boundaries and emit one segment per gap.
fn build_layout(
    display: &str,
    syntax: &[StyledSpan],
    inline: &[(std::ops::Range<usize>, InlineChangeKind)],
    find_ranges: &[(std::ops::Range<usize>, bool)],
    side: Side,
    font_size: f32,
) -> egui::text::LayoutJob {
    let font = FontId::monospace(font_size);
    let mut job = egui::text::LayoutJob::default();
    if display.is_empty() {
        return job;
    }

    let display_len = display.len();
    let mut boundaries: Vec<usize> = vec![0, display_len];
    for s in syntax {
        boundaries.push(s.range.start.min(display_len));
        boundaries.push(s.range.end.min(display_len));
    }
    for (r, _) in inline {
        boundaries.push(r.start.min(display_len));
        boundaries.push(r.end.min(display_len));
    }
    for (r, _) in find_ranges {
        boundaries.push(r.start.min(display_len));
        boundaries.push(r.end.min(display_len));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];
        if start >= end {
            continue;
        }
        // Avoid splitting in the middle of a multi-byte UTF-8 codepoint.
        if !display.is_char_boundary(start) || !display.is_char_boundary(end) {
            continue;
        }
        let fg = syntax_fg_at(syntax, start);
        // Find matches take priority over inline highlights so the user
        // can see the search hit clearly inside a Replace row's tinting.
        let bg = match find_bg_at(find_ranges, start) {
            Some(c) => c,
            None => inline_bg_at(inline, side, start),
        };
        let fmt = egui::TextFormat {
            font_id: font.clone(),
            color: fg,
            background: bg,
            ..Default::default()
        };
        job.append(&display[start..end], 0.0, fmt);
    }
    job
}

fn find_bg_at(ranges: &[(std::ops::Range<usize>, bool)], pos: usize) -> Option<Color32> {
    let mut best: Option<bool> = None;
    for (r, current) in ranges {
        if r.contains(&pos) {
            best = Some(best.unwrap_or(false) || *current);
        }
    }
    best.map(|is_current| {
        if is_current {
            theme::FIND_CURRENT_BG
        } else {
            theme::FIND_BG
        }
    })
}

fn syntax_fg_at(spans: &[StyledSpan], pos: usize) -> Color32 {
    for s in spans {
        if s.range.contains(&pos) {
            let r = ((s.rgb >> 16) & 0xff) as u8;
            let g = ((s.rgb >> 8) & 0xff) as u8;
            let b = (s.rgb & 0xff) as u8;
            // syntect occasionally returns black (#000000) for unstyled
            // regions in light themes; treat as "use default".
            if r == 0 && g == 0 && b == 0 {
                return Color32::PLACEHOLDER;
            }
            return Color32::from_rgb(r, g, b);
        }
    }
    Color32::PLACEHOLDER
}

fn inline_bg_at(
    spans: &[(std::ops::Range<usize>, InlineChangeKind)],
    side: Side,
    pos: usize,
) -> Color32 {
    for (range, kind) in spans {
        if range.contains(&pos) {
            let want_highlight = matches!(
                (kind, side),
                (InlineChangeKind::Delete, Side::Left) | (InlineChangeKind::Insert, Side::Right)
            );
            if want_highlight {
                return theme::INLINE_BG;
            }
        }
    }
    Color32::TRANSPARENT
}

fn row_background(row: &DiffRow) -> Color32 {
    match row {
        DiffRow::Equal { .. } | DiffRow::Gap { .. } => Color32::TRANSPARENT,
        DiffRow::Delete { .. } => theme::DELETE_BG,
        DiffRow::Insert { .. } => theme::INSERT_BG,
        DiffRow::Replace { .. } => theme::REPLACE_BG,
    }
}

fn decompose(row: &DiffRow) -> RowParts<'_> {
    match row {
        DiffRow::Equal {
            left_line,
            right_line,
            text,
        } => RowParts {
            left_num: Some(*left_line),
            left_text: text,
            right_num: Some(*right_line),
            right_text: text,
            inline_left: vec![],
            inline_right: vec![],
        },
        DiffRow::Delete { left_line, text } => RowParts {
            left_num: Some(*left_line),
            left_text: text,
            right_num: None,
            right_text: "",
            inline_left: vec![],
            inline_right: vec![],
        },
        DiffRow::Insert { right_line, text } => RowParts {
            left_num: None,
            left_text: "",
            right_num: Some(*right_line),
            right_text: text,
            inline_left: vec![],
            inline_right: vec![],
        },
        DiffRow::Replace {
            left_line,
            right_line,
            left_text,
            right_text,
            inline,
        } => {
            let mut il = Vec::new();
            let mut ir = Vec::new();
            for c in inline {
                match c.side {
                    Side::Left => il.push((c.range.clone(), c.kind)),
                    Side::Right => ir.push((c.range.clone(), c.kind)),
                }
            }
            RowParts {
                left_num: Some(*left_line),
                left_text,
                right_num: Some(*right_line),
                right_text,
                inline_left: il,
                inline_right: ir,
            }
        }
        DiffRow::Gap { .. } => RowParts {
            left_num: None,
            left_text: "",
            right_num: None,
            right_text: "",
            inline_left: vec![],
            inline_right: vec![],
        },
    }
}

fn strip_nl(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}

/// Pick a language hint (file extension, lowercased, no dot) for a document.
/// Empty string means "let the highlighter fall back to plain text".
fn hint_for(doc: &DiffDocument) -> String {
    doc.path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default()
}

fn short_path(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_nl_handles_trailing_newline() {
        assert_eq!(strip_nl("hello\n"), "hello");
        assert_eq!(strip_nl("hello"), "hello");
        assert_eq!(strip_nl(""), "");
    }

    #[test]
    fn short_path_picks_file_name() {
        assert_eq!(
            short_path(std::path::Path::new("/tmp/x/y/foo.txt")),
            "foo.txt"
        );
    }

    /// Redirect LGTM_CONFIG_DIR to a per-process temp dir so recent-list
    /// writes from `swap_documents` / `ClearRecent` don't touch the
    /// developer's real `~/.config/lgtm/recent.tsv`.
    fn isolate_config_dir_once() {
        use std::sync::OnceLock;
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!("lgtm-diffapp-tests-{nanos}"));
            std::fs::create_dir_all(&p).unwrap();
            // SAFETY: protected by OnceLock; runs exactly once per process,
            // before any test touches RecentList.
            unsafe {
                std::env::set_var("LGTM_CONFIG_DIR", &p);
            }
        });
    }

    fn fixture(left: &str, right: &str) -> DiffApp {
        isolate_config_dir_once();
        let l = DiffDocument::empty_for("l");
        let mut l = l;
        l.content = left.into();
        let mut r = DiffDocument::empty_for("r");
        r.content = right.into();
        let diff = lgtm_core::AlignedDiff::compute(&l, &r);
        // Tests use the Noop highlighter to avoid loading syntect's
        // default assets — it shaves ~100 ms off each test run.
        DiffApp::with_highlighter(l, r, diff, false, Arc::new(lgtm_core::NoopHighlighter))
    }

    #[test]
    fn jump_hunk_wraps_at_ends() {
        let mut app = fixture("a\nb\nc\nd\n", "X\nb\nY\nd\n");
        assert_eq!(app.diff.hunks.len(), 2);
        assert_eq!(app.current_hunk, 0);
        app.jump_hunk(1);
        assert_eq!(app.current_hunk, 1);
        app.jump_hunk(1);
        assert_eq!(app.current_hunk, 0, "wrap to first");
        app.jump_hunk(-1);
        assert_eq!(app.current_hunk, 1, "wrap to last when going back from 0");
    }

    #[test]
    fn jump_hunk_noop_on_empty() {
        let mut app = fixture("a\n", "a\n");
        assert_eq!(app.diff.hunks.len(), 0);
        app.jump_hunk(1);
        assert_eq!(app.current_hunk, 0);
        app.jump_first();
        app.jump_last();
        // shouldn't crash, no pending scroll
        assert!(app.pending_scroll.is_none());
    }

    #[test]
    fn mark_edited_sets_dirty_and_timer() {
        let mut app = fixture("a\n", "a\n");
        assert!(!app.is_dirty());
        app.mark_edited(Side::Left);
        assert!(app.modified_left);
        assert!(!app.modified_right);
        assert!(app.is_dirty());
        assert!(app.last_edit_at.is_some());
    }

    #[test]
    fn recompute_diff_after_edit_clamps_current_hunk() {
        let mut app = fixture("a\nb\nc\nd\n", "X\nb\nY\nd\n");
        assert_eq!(app.diff.hunks.len(), 2);
        app.current_hunk = 1;
        // Now mutate to a single-hunk state and recompute.
        app.left.content = "a\nb\n".into();
        app.right.content = "X\nb\n".into();
        app.recompute_diff();
        assert_eq!(app.diff.hunks.len(), 1);
        assert_eq!(app.current_hunk, 0, "current_hunk must be clamped");
    }

    #[test]
    fn save_round_trips_edited_content() {
        let dir = std::env::temp_dir().join("lgtm-edit-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let lpath = dir.join("save_l.txt");
        let rpath = dir.join("save_r.txt");
        std::fs::write(&lpath, b"a\nb\n").unwrap();
        std::fs::write(&rpath, b"a\nB\n").unwrap();

        let l = DiffDocument::load(&lpath).unwrap();
        let r = DiffDocument::load(&rpath).unwrap();
        let diff = lgtm_core::AlignedDiff::compute(&l, &r);
        let mut app = DiffApp::new(l, r, diff, false);

        app.left.content = "a\nBB\n".into();
        app.modified_left = true;
        app.save().unwrap();

        assert!(!app.modified_left);
        let written = std::fs::read(&lpath).unwrap();
        assert_eq!(written, b"a\nBB\n");
    }

    #[test]
    fn hint_for_picks_lowercase_extension() {
        let mut d = DiffDocument::empty_for("/tmp/foo.RS");
        d.content = "x".into();
        assert_eq!(hint_for(&d), "rs");
    }

    #[test]
    fn hint_for_returns_empty_when_no_extension() {
        let d = DiffDocument::empty_for("/tmp/Makefile");
        assert_eq!(hint_for(&d), "");
    }

    #[test]
    fn build_layout_handles_empty_text() {
        let job = build_layout("", &[], &[], &[], Side::Left, 13.0);
        assert!(job.sections.is_empty());
    }

    #[test]
    fn build_layout_with_only_syntax_splits_at_span_boundaries() {
        let spans = vec![
            StyledSpan {
                range: 0..2,
                rgb: 0xff0000,
                style_bits: 0,
            },
            StyledSpan {
                range: 2..5,
                rgb: 0x00ff00,
                style_bits: 0,
            },
        ];
        let job = build_layout("abcde", &spans, &[], &[], Side::Left, 13.0);
        // expect two non-empty sections, one per span.
        assert_eq!(job.sections.len(), 2);
        let s0 = &job.text[job.sections[0].byte_range.clone()];
        let s1 = &job.text[job.sections[1].byte_range.clone()];
        assert_eq!(s0, "ab");
        assert_eq!(s1, "cde");
    }

    #[test]
    fn build_layout_combines_syntax_and_inline_boundaries() {
        let syntax = vec![StyledSpan {
            range: 0..5,
            rgb: 0xff0000,
            style_bits: 0,
        }];
        let inline = vec![(2..4, InlineChangeKind::Insert)];
        let job = build_layout("abcde", &syntax, &inline, &[], Side::Right, 13.0);
        // boundaries: 0, 2, 4, 5 => 3 sections: "ab", "cd", "e"
        assert_eq!(job.sections.len(), 3);
        let texts: Vec<&str> = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.clone()])
            .collect();
        assert_eq!(texts, vec!["ab", "cd", "e"]);
        // The middle section should carry the inline background (right side
        // + Insert kind triggers highlight).
        assert_eq!(job.sections[1].format.background, theme::INLINE_BG);
        assert_eq!(job.sections[0].format.background, Color32::TRANSPARENT);
    }

    #[test]
    fn build_layout_drops_inline_background_on_wrong_side() {
        let inline = vec![(0..3, InlineChangeKind::Insert)];
        // Insert kind on the Left side: should NOT highlight.
        let job = build_layout("abcdef", &[], &inline, &[], Side::Left, 13.0);
        assert!(
            job.sections
                .iter()
                .all(|s| s.format.background == Color32::TRANSPARENT)
        );
    }

    #[test]
    fn build_layout_respects_utf8_boundaries() {
        // "héllo" — the é is 2 bytes (0xc3 0xa9). A span ending at byte 2
        // would split the character; build_layout must skip that segment.
        let spans = vec![
            StyledSpan {
                range: 0..2,
                rgb: 0xff0000,
                style_bits: 0,
            },
            StyledSpan {
                range: 2..6,
                rgb: 0x00ff00,
                style_bits: 0,
            },
        ];
        // Should not panic even though boundary 2 splits "é".
        let _ = build_layout("héllo", &spans, &[], &[], Side::Left, 13.0);
    }

    #[test]
    fn recompute_diff_refreshes_syntax_caches() {
        let mut app = fixture("a\nb\n", "a\nb\n");
        let before_len = app.cached_left_syntax.len();
        app.left.content = "a\nb\nc\nd\n".into();
        app.recompute_diff();
        assert_eq!(app.cached_left_syntax.len(), 4);
        assert!(app.cached_left_syntax.len() > before_len);
    }

    #[test]
    fn jump_first_and_last_set_pending_scroll() {
        let mut app = fixture("a\nb\nc\nd\n", "X\nb\nY\nd\n");
        app.jump_last();
        assert_eq!(app.current_hunk, 1);
        assert!(app.pending_scroll.is_some());
        app.pending_scroll = None;
        app.jump_first();
        assert_eq!(app.current_hunk, 0);
        assert!(app.pending_scroll.is_some());
    }

    // ---- per-hunk Copy ---------------------------------------------

    #[test]
    fn copy_hunk_left_to_right_rewrites_right_pane_and_marks_dirty() {
        let mut app = fixture("a\nLOCAL\nc\n", "a\nREMOTE\nc\n");
        assert_eq!(app.diff.hunks.len(), 1);
        app.copy_hunk(0, CopyDirection::LeftToRight);
        assert_eq!(app.right.content, "a\nLOCAL\nc\n");
        assert!(app.modified_right);
        assert!(!app.modified_left);
        assert!(app.last_edit_at.is_some());
    }

    #[test]
    fn copy_hunk_right_to_left_rewrites_left_pane_and_marks_dirty() {
        let mut app = fixture("a\nLOCAL\nc\n", "a\nREMOTE\nc\n");
        app.copy_hunk(0, CopyDirection::RightToLeft);
        assert_eq!(app.left.content, "a\nREMOTE\nc\n");
        assert!(app.modified_left);
        assert!(!app.modified_right);
    }

    #[test]
    fn copy_hunk_pure_insert_left_to_right_drops_inserted_lines() {
        // Right has extra lines that don't exist on the left. Copy →
        // means "make right look like left here" → delete those lines.
        let mut app = fixture("a\nb\n", "a\nX\nY\nb\n");
        assert_eq!(app.diff.hunks.len(), 1);
        app.copy_hunk(0, CopyDirection::LeftToRight);
        assert_eq!(app.right.content, "a\nb\n");
    }

    #[test]
    fn copy_hunk_pure_insert_right_to_left_adds_lines_to_left() {
        let mut app = fixture("a\nb\n", "a\nX\nY\nb\n");
        app.copy_hunk(0, CopyDirection::RightToLeft);
        assert_eq!(app.left.content, "a\nX\nY\nb\n");
    }

    #[test]
    fn copy_hunk_pure_delete_left_to_right_restores_lines_on_right() {
        // Left has extra lines that don't exist on right. Copy →
        // means add them to right.
        let mut app = fixture("a\nX\nY\nb\n", "a\nb\n");
        app.copy_hunk(0, CopyDirection::LeftToRight);
        assert_eq!(app.right.content, "a\nX\nY\nb\n");
    }

    #[test]
    fn copy_hunk_then_recompute_collapses_the_hunk() {
        let mut app = fixture("a\nLOCAL\nc\n", "a\nREMOTE\nc\n");
        assert_eq!(app.diff.hunks.len(), 1);
        app.copy_hunk(0, CopyDirection::LeftToRight);
        app.recompute_diff();
        assert_eq!(
            app.diff.hunks.len(),
            0,
            "after copy both sides should match → no hunks remain"
        );
    }

    #[test]
    fn copy_hunk_is_noop_in_read_only_mode() {
        let mut app = fixture("a\nLOCAL\nc\n", "a\nREMOTE\nc\n");
        app.read_only = true;
        app.copy_hunk(0, CopyDirection::LeftToRight);
        assert_eq!(app.right.content, "a\nREMOTE\nc\n");
        assert!(!app.modified_right);
    }

    #[test]
    fn copy_hunk_out_of_range_is_silent_noop() {
        let mut app = fixture("a\n", "a\n");
        // No hunks at all → index 0 is OOB.
        app.copy_hunk(0, CopyDirection::LeftToRight);
        assert_eq!(app.right.content, "a\n");
        assert!(!app.is_dirty());
    }

    #[test]
    fn copy_hunk_at_file_start_pure_insert() {
        // The hunk lives at the very top of the file; the empty side's
        // line range must be 1..1 (the insertion-point edge case).
        let mut app = fixture("a\n", "X\na\n");
        app.copy_hunk(0, CopyDirection::LeftToRight);
        assert_eq!(app.right.content, "a\n");
    }

    #[test]
    fn copy_hunk_at_file_end_pure_insert() {
        let mut app = fixture("a\n", "a\nX\n");
        app.copy_hunk(0, CopyDirection::RightToLeft);
        assert_eq!(app.left.content, "a\nX\n");
    }

    // ---- edit-mode syntax layouter ---------------------------------

    #[test]
    fn build_edit_layout_empty_text_yields_empty_job() {
        let job = build_edit_layout("", &[], 13.0);
        assert!(job.sections.is_empty());
    }

    #[test]
    fn build_edit_layout_plain_text_with_no_spans_renders_each_line() {
        let job = build_edit_layout("a\nb\nc\n", &[], 13.0);
        let text: String = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.clone()])
            .collect();
        assert_eq!(text, "a\nb\nc\n");
        // Every section should be plain (no color override).
        for s in &job.sections {
            assert_eq!(s.format.color, Color32::PLACEHOLDER);
        }
    }

    #[test]
    fn build_edit_layout_applies_spans_per_line() {
        // Two lines, two spans on line 1, one span on line 2.
        let syntax = vec![
            vec![
                StyledSpan {
                    range: 0..2,
                    rgb: 0xff_00_00,
                    style_bits: 0,
                },
                StyledSpan {
                    range: 2..5,
                    rgb: 0x00_ff_00,
                    style_bits: 0,
                },
            ],
            vec![StyledSpan {
                range: 0..3,
                rgb: 0x00_00_ff,
                style_bits: 0,
            }],
        ];
        let job = build_edit_layout("ab cd\nfoo\n", &syntax, 13.0);
        // Expect: red "ab", green " cd", newline, blue "foo", newline.
        let texts: Vec<&str> = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.clone()])
            .collect();
        assert_eq!(texts, vec!["ab", " cd", "\n", "foo", "\n"]);
        assert_eq!(job.sections[0].format.color, Color32::from_rgb(0xff, 0, 0));
        assert_eq!(job.sections[1].format.color, Color32::from_rgb(0, 0xff, 0));
        assert_eq!(job.sections[3].format.color, Color32::from_rgb(0, 0, 0xff));
    }

    #[test]
    fn build_edit_layout_falls_back_to_plain_when_spans_are_stale() {
        // The cache says line 1 has 10 bytes of spans, but the user has
        // since deleted half the line so it's only 3 bytes long. The
        // layouter must render the line plain rather than slicing past
        // the end (which would panic).
        let syntax = vec![vec![StyledSpan {
            range: 0..10,
            rgb: 0xff_00_00,
            style_bits: 0,
        }]];
        let job = build_edit_layout("abc\n", &syntax, 13.0);
        let texts: Vec<&str> = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.clone()])
            .collect();
        assert_eq!(texts, vec!["abc", "\n"]);
        // Plain, since spans were stale.
        assert_eq!(job.sections[0].format.color, Color32::PLACEHOLDER);
    }

    #[test]
    fn build_edit_layout_handles_lines_added_since_last_highlight() {
        // Cache covers one line, the buffer now has three.
        let syntax = vec![vec![StyledSpan {
            range: 0..1,
            rgb: 0xff_00_00,
            style_bits: 0,
        }]];
        let job = build_edit_layout("a\nb\nc\n", &syntax, 13.0);
        // Line 1 is colored; lines 2 and 3 are plain.
        let texts: Vec<&str> = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.clone()])
            .collect();
        assert_eq!(texts, vec!["a", "\n", "b", "\n", "c", "\n"]);
        assert_eq!(job.sections[0].format.color, Color32::from_rgb(0xff, 0, 0));
        assert_eq!(job.sections[2].format.color, Color32::PLACEHOLDER);
        assert_eq!(job.sections[4].format.color, Color32::PLACEHOLDER);
    }

    #[test]
    fn build_edit_layout_handles_final_line_without_newline() {
        let syntax = vec![vec![StyledSpan {
            range: 0..3,
            rgb: 0xff_00_00,
            style_bits: 0,
        }]];
        let job = build_edit_layout("abc", &syntax, 13.0);
        let texts: Vec<&str> = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.clone()])
            .collect();
        assert_eq!(texts, vec!["abc"]);
    }

    // ---- menubar dispatch ------------------------------------------

    #[test]
    fn swap_documents_replaces_both_panes_and_recomputes() {
        let mut app = fixture("a\n", "a\n");
        // The fixture has no hunks; new content has one.
        let mut nl = DiffDocument::empty_for("nl");
        nl.content = "a\nLOCAL\nb\n".into();
        let mut nr = DiffDocument::empty_for("nr");
        nr.content = "a\nREMOTE\nb\n".into();
        app.swap_documents(nl, nr);
        assert_eq!(app.left.content, "a\nLOCAL\nb\n");
        assert_eq!(app.right.content, "a\nREMOTE\nb\n");
        assert_eq!(app.diff.hunks.len(), 1);
        assert!(!app.is_dirty(), "fresh load is never dirty");
    }

    #[test]
    fn handle_menu_action_toggle_edit_mode_flips() {
        let mut app = fixture("a\n", "b\n");
        assert!(!app.edit_mode);
        app.handle_menu_action(MenuAction::ToggleEditMode);
        assert!(app.edit_mode);
        app.handle_menu_action(MenuAction::ToggleEditMode);
        assert!(!app.edit_mode);
    }

    #[test]
    fn handle_menu_action_toggle_edit_mode_is_noop_when_read_only() {
        let mut app = fixture("a\n", "b\n");
        app.read_only = true;
        app.handle_menu_action(MenuAction::ToggleEditMode);
        assert!(!app.edit_mode);
    }

    #[test]
    fn handle_menu_action_show_modals_flags_set() {
        let mut app = fixture("a\n", "b\n");
        app.handle_menu_action(MenuAction::ShowAbout);
        assert!(app.show_about);
        app.handle_menu_action(MenuAction::ShowShortcuts);
        assert!(app.show_shortcuts);
    }

    #[test]
    fn handle_menu_action_increase_decrease_reset_font() {
        let mut app = fixture("a\n", "b\n");
        let base = app.settings.font_size;
        app.handle_menu_action(MenuAction::IncreaseFontSize);
        assert!(app.settings.font_size > base);
        app.handle_menu_action(MenuAction::DecreaseFontSize);
        assert_eq!(app.settings.font_size, base);
        for _ in 0..3 {
            app.handle_menu_action(MenuAction::IncreaseFontSize);
        }
        app.handle_menu_action(MenuAction::ResetFontSize);
        assert_eq!(app.settings.font_size, lgtm_core::DEFAULT_FONT_SIZE);
    }

    #[test]
    fn font_size_clamps_at_min_and_max() {
        let mut app = fixture("a\n", "b\n");
        for _ in 0..50 {
            app.handle_menu_action(MenuAction::IncreaseFontSize);
        }
        assert_eq!(app.settings.font_size, lgtm_core::MAX_FONT_SIZE);
        for _ in 0..50 {
            app.handle_menu_action(MenuAction::DecreaseFontSize);
        }
        assert_eq!(app.settings.font_size, lgtm_core::MIN_FONT_SIZE);
    }

    #[test]
    fn edit_scroll_y_starts_at_zero() {
        // The shared scroll offset for the edit-mode panes initializes
        // to 0 so both panes line up at the top on first render. The
        // actual cross-pane sync is driven by egui's ScrollAreaOutput
        // offsets and is only observable in an integration test, but
        // the field itself is unit-testable.
        let app = fixture("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(app.edit_scroll_y, 0.0);
    }

    #[test]
    fn find_recomputes_on_diff_swap() {
        let mut app = fixture("hello world\n", "hello world\n");
        app.find.query = "world".into();
        app.find.recompute(&app.diff.rows);
        assert_eq!(app.find.len(), 2);
        let mut nl = DiffDocument::empty_for("nl");
        nl.content = "x\n".into();
        let mut nr = DiffDocument::empty_for("nr");
        nr.content = "y\n".into();
        app.swap_documents(nl, nr);
        assert_eq!(app.find.len(), 0);
    }

    #[test]
    fn find_recomputes_after_recompute_diff() {
        let mut app = fixture("alpha beta\n", "alpha gamma\n");
        app.find.query = "alpha".into();
        app.find.recompute(&app.diff.rows);
        assert_eq!(app.find.len(), 2);
        app.left.content = "x beta\n".into();
        app.recompute_diff();
        assert_eq!(app.find.len(), 1);
    }

    #[test]
    fn find_bg_at_picks_current_over_normal() {
        let ranges = vec![(0..5, false), (2..4, true)];
        assert_eq!(find_bg_at(&ranges, 3), Some(theme::FIND_CURRENT_BG));
        assert_eq!(find_bg_at(&ranges, 1), Some(theme::FIND_BG));
        assert_eq!(find_bg_at(&ranges, 10), None);
    }

    #[test]
    fn handle_menu_action_hunk_nav_drives_jump_methods() {
        let mut app = fixture("a\nb\nc\nd\n", "X\nb\nY\nd\n");
        assert_eq!(app.diff.hunks.len(), 2);
        app.handle_menu_action(MenuAction::NextHunk);
        assert_eq!(app.current_hunk, 1);
        app.handle_menu_action(MenuAction::FirstHunk);
        assert_eq!(app.current_hunk, 0);
        app.handle_menu_action(MenuAction::LastHunk);
        assert_eq!(app.current_hunk, 1);
        app.handle_menu_action(MenuAction::PrevHunk);
        assert_eq!(app.current_hunk, 0);
    }

    #[test]
    fn handle_menu_action_clear_recent_empties_list() {
        let mut app = fixture("a\n", "b\n");
        let mut nl = DiffDocument::empty_for("nl");
        nl.content = "x".into();
        let mut nr = DiffDocument::empty_for("nr");
        nr.content = "y".into();
        app.swap_documents(nl, nr);
        assert!(app.recents.entries().count() >= 1);
        app.handle_menu_action(MenuAction::ClearRecent);
        assert!(app.recents.is_empty());
    }

    #[test]
    fn build_edit_layout_skips_spans_that_split_utf8_codepoints() {
        // "é" is 2 bytes; a span ending at byte 1 would split it. The
        // layouter must skip such a span rather than panic.
        let syntax = vec![vec![StyledSpan {
            range: 0..1,
            rgb: 0xff_00_00,
            style_bits: 0,
        }]];
        // Should not panic.
        let _ = build_edit_layout("élan\n", &syntax, 13.0);
    }
}
