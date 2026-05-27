//! Two-file diff window.

use std::time::{Duration, Instant};

use egui::{Align, Color32, FontId, Key, Layout, RichText, ScrollArea, Sense, TextStyle};
use lgtm_core::{AlignedDiff, DiffDocument, DiffRow, HunkKind, InlineChangeKind, Side};

use crate::theme;

/// Recompute the diff after this much idle time once an edit has landed.
const DIFF_DEBOUNCE: Duration = Duration::from_millis(150);

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
}

impl DiffApp {
    /// Construct an app from already-loaded documents and a precomputed diff.
    pub fn new(
        left: DiffDocument,
        right: DiffDocument,
        diff: AlignedDiff,
        read_only: bool,
    ) -> Self {
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
        }
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
        });
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
        ui.horizontal_top(|ui| {
            let left_resp = ui.allocate_ui(egui::vec2(half, avail.y), |ui| {
                ScrollArea::vertical()
                    .id_salt("lgtm-edit-left")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut self.left.content)
                                .font(FontId::monospace(13.0))
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(40)
                                .interactive(!self.read_only),
                        );
                        if resp.changed() {
                            self.mark_edited(Side::Left);
                        }
                    });
            });
            let _ = left_resp;
            ui.separator();
            let right_resp = ui.allocate_ui(egui::vec2(half, avail.y), |ui| {
                ScrollArea::vertical()
                    .id_salt("lgtm-edit-right")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut self.right.content)
                                .font(FontId::monospace(13.0))
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(40)
                                .interactive(!self.read_only),
                        );
                        if resp.changed() {
                            self.mark_edited(Side::Right);
                        }
                    });
            });
            let _ = right_resp;
        });
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

        let row_height = ui.text_style_height(&TextStyle::Monospace) + 2.0;
        let total = self.diff.rows.len();
        let pending = self.pending_scroll.take();

        let mut scroll = ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(row) = pending {
            // Place the target row a third of the way down the viewport.
            scroll = scroll.vertical_scroll_offset((row as f32 * row_height) - 60.0);
        }
        scroll.show_rows(ui, row_height, total, |ui, row_range| {
            ui.style_mut().override_font_id = Some(FontId::monospace(13.0));
            for idx in row_range {
                render_row(ui, &self.diff.rows[idx], row_height);
            }
        });
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
        ctx.input(|i| {
            if i.modifiers.command_only() && i.key_pressed(Key::S) {
                want_save = true;
            }
            if i.key_pressed(Key::Escape) {
                self.close_requested = true;
            }
            if !self.edit_mode {
                if i.key_pressed(Key::Q) {
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
            }
        });
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
    }
}

impl DiffApp {
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

fn render_row(ui: &mut egui::Ui, row: &DiffRow, row_height: f32) {
    let bg = row_background(row);
    let avail = ui.available_width();
    let half = (avail - 8.0) * 0.5;

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

    render_pane(
        &mut child,
        half,
        parts.left_num,
        parts.left_text,
        Side::Left,
        &parts.inline_left,
    );
    child.add(egui::Separator::default().vertical().spacing(8.0));
    render_pane(
        &mut child,
        half,
        parts.right_num,
        parts.right_text,
        Side::Right,
        &parts.inline_right,
    );
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

fn render_pane(
    ui: &mut egui::Ui,
    width: f32,
    line_num: Option<usize>,
    text: &str,
    side: Side,
    inline: &[(std::ops::Range<usize>, InlineChangeKind)],
) {
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.horizontal(|ui| {
            let gutter = match line_num {
                Some(n) => format!("{n:>width$}", width = theme::GUTTER_WIDTH_CHARS),
                None => " ".repeat(theme::GUTTER_WIDTH_CHARS),
            };
            ui.label(RichText::new(gutter).color(theme::GUTTER_FG).monospace());
            if inline.is_empty() {
                ui.label(RichText::new(strip_nl(text)).monospace());
            } else {
                render_inline_text(ui, text, inline, side);
            }
        });
    });
}

fn render_inline_text(
    ui: &mut egui::Ui,
    text: &str,
    inline: &[(std::ops::Range<usize>, InlineChangeKind)],
    side: Side,
) {
    let mut layout = egui::text::LayoutJob::default();
    let display = strip_nl(text);
    let display_len = display.len();
    let mut cursor = 0usize;
    for (range, kind) in inline {
        let start = range.start.min(display_len);
        let end = range.end.min(display_len);
        if start > cursor {
            layout.append(
                &display[cursor..start],
                0.0,
                egui::TextFormat {
                    font_id: FontId::monospace(13.0),
                    ..Default::default()
                },
            );
        }
        let want_highlight = matches!(
            (kind, side),
            (InlineChangeKind::Delete, Side::Left) | (InlineChangeKind::Insert, Side::Right)
        );
        let fmt = egui::TextFormat {
            font_id: FontId::monospace(13.0),
            background: if want_highlight {
                theme::INLINE_BG
            } else {
                Color32::TRANSPARENT
            },
            ..Default::default()
        };
        if end > start {
            layout.append(&display[start..end], 0.0, fmt);
        }
        cursor = end;
    }
    if cursor < display_len {
        layout.append(
            &display[cursor..],
            0.0,
            egui::TextFormat {
                font_id: FontId::monospace(13.0),
                ..Default::default()
            },
        );
    }
    ui.label(layout);
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

    fn fixture(left: &str, right: &str) -> DiffApp {
        let l = DiffDocument::empty_for("l");
        let mut l = l;
        l.content = left.into();
        let mut r = DiffDocument::empty_for("r");
        r.content = right.into();
        let diff = lgtm_core::AlignedDiff::compute(&l, &r);
        DiffApp::new(l, r, diff, false)
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
}
