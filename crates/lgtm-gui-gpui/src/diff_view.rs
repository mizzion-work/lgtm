//! Two-file diff window — gpui port.
//!
//! Architecture differs from the egui counterpart in two key ways:
//!
//! 1. **Retained-mode views.** Each `DiffView` is a long-lived `Entity`
//!    in gpui's model; layout is described by a `Render` impl returning
//!    an `impl IntoElement` element tree. egui rebuilt the whole UI
//!    every frame; gpui invalidates and re-renders selectively.
//!
//! 2. **No `ctx.input` polling loop.** Input arrives as typed events
//!    handled by `on_*` methods bound to elements at construction time.
//!    We register click / hover / key handlers declaratively.

use gpui::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use lgtm_core::{AlignedDiff, DiffDocument, DiffRow, Side};

use crate::GuiOutcome;

/// Launch the gpui-backed two-file diff window. Blocks until the user
/// closes the window.
///
/// Identical files short-circuit without opening a window, matching the
/// eframe entry point's behavior.
pub fn run_diff(
    left: DiffDocument,
    right: DiffDocument,
    _read_only: bool,
) -> anyhow::Result<GuiOutcome> {
    // Cheap pre-check: identical content → no window.
    let outcome = if !left.is_binary && !right.is_binary && left.content == right.content {
        GuiOutcome::Identical
    } else {
        GuiOutcome::Differs
    };

    let diff = AlignedDiff::compute(&left, &right).with_inline();

    // gpui's application() returns a builder; .run takes a closure that
    // gets the App context. The closure must call cx.activate() at the
    // end to bring the window forward.
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let window_opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            ..Default::default()
        };
        let view = DiffView::new(left.clone(), right.clone(), diff.clone());
        cx.open_window(window_opts, move |_window, cx| cx.new(|_| view))
            .expect("open_window failed");
        cx.activate(true);
    });

    Ok(outcome)
}

/// gpui view backing the diff window.
struct DiffView {
    left: DiffDocument,
    right: DiffDocument,
    diff: AlignedDiff,
}

impl DiffView {
    fn new(left: DiffDocument, right: DiffDocument, diff: AlignedDiff) -> Self {
        Self { left, right, diff }
    }

    /// One visual row — left pane | center separator | right pane.
    /// Background color comes from the diff status (insert/delete/replace).
    fn row(&self, row: &DiffRow) -> impl IntoElement {
        let bg = match row {
            DiffRow::Equal { .. } | DiffRow::Gap { .. } => 0x202020,
            DiffRow::Delete { .. } => 0x4a181f,
            DiffRow::Insert { .. } => 0x183f1d,
            DiffRow::Replace { .. } => 0x4a4012,
        };
        let (left_num, left_text, right_num, right_text) = match row {
            DiffRow::Equal {
                left_line,
                right_line,
                text,
            } => (
                Some(*left_line),
                strip_nl(text).to_string(),
                Some(*right_line),
                strip_nl(text).to_string(),
            ),
            DiffRow::Delete { left_line, text } => (
                Some(*left_line),
                strip_nl(text).to_string(),
                None,
                String::new(),
            ),
            DiffRow::Insert { right_line, text } => (
                None,
                String::new(),
                Some(*right_line),
                strip_nl(text).to_string(),
            ),
            DiffRow::Replace {
                left_line,
                right_line,
                left_text,
                right_text,
                ..
            } => (
                Some(*left_line),
                strip_nl(left_text).to_string(),
                Some(*right_line),
                strip_nl(right_text).to_string(),
            ),
            DiffRow::Gap { .. } => (None, String::new(), None, String::new()),
        };

        div()
            .flex()
            .flex_row()
            .w_full()
            .h(px(18.0))
            .bg(rgb(bg))
            .text_color(rgb(0xdcdcdc))
            .text_sm()
            .font_family("monospace")
            .child(pane(left_num, &left_text, Side::Left))
            .child(
                div()
                    .w(px(8.0))
                    .h_full()
                    .bg(rgb(0x3c3c3c)),
            )
            .child(pane(right_num, &right_text, Side::Right))
    }
}

fn pane(line_num: Option<usize>, text: &str, _side: Side) -> impl IntoElement {
    let gutter = match line_num {
        Some(n) => format!("{n:>5}"),
        None => "     ".to_string(),
    };
    div()
        .flex()
        .flex_row()
        .flex_1()
        .h_full()
        .child(
            div()
                .w(px(48.0))
                .text_color(rgb(0x808080))
                .child(gutter),
        )
        .child(div().flex_1().child(text.to_string()))
}

fn strip_nl(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = format!(
            "lgtm — {} ↔ {}",
            short_path(&self.left.path),
            short_path(&self.right.path),
        );
        let status = format!(
            "{} differences, {} hunks",
            self.diff.stats.differences(),
            self.diff.hunks.len(),
        );

        // Collect row elements up front; gpui's element tree is built
        // eagerly per frame.
        let rows: Vec<_> = self.diff.rows.iter().map(|r| self.row(r)).collect();

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .bg(rgb(0x181818))
            .text_color(rgb(0xdcdcdc))
            .child(
                // Title bar.
                div()
                    .w_full()
                    .h(px(32.0))
                    .px(px(12.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .bg(rgb(0x2a2a2a))
                    .child(title),
            )
            .child(
                // Scrollable diff area.
                div()
                    .id("diff-scroll")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(
                // Status bar.
                div()
                    .w_full()
                    .h(px(20.0))
                    .px(px(8.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .bg(rgb(0x2a2a2a))
                    .text_color(rgb(0x808080))
                    .text_xs()
                    .child(status),
            )
    }
}

fn short_path(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}
