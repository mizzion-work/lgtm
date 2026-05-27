//! Collapsible side drawer showing the repo's git graph.
//!
//! The drawer is opt-in (toggled via the menubar's View → Show Git Graph)
//! and only does anything when the diff documents live inside a git
//! working tree — for difftool temp files the drawer renders an empty
//! state with a brief explanation.

use egui::{Color32, FontId, RichText, ScrollArea, Sense, Stroke};
use lgtm_core::{Graph, GraphEdge};

/// Pixel width of one swimlane column.
const LANE_WIDTH: f32 = 14.0;
/// Vertical pixel height of one commit row.
const ROW_HEIGHT: f32 = 22.0;
/// Radius of the commit dot.
const DOT_RADIUS: f32 = 4.0;
/// Width of the leftmost graph gutter (max lanes shown).
const GRAPH_GUTTER_WIDTH: f32 = LANE_WIDTH * 6.0;

/// Render the git-graph drawer. Caller has already loaded a [`Graph`]
/// (or is passing the cached one). Returns nothing — the drawer is
/// read-only for now.
pub fn render_graph_panel(ui: &mut egui::Ui, graph: &Graph) {
    ui.heading("Git graph");
    ui.separator();
    if graph.commits.is_empty() {
        ui.label(RichText::new("(no git history available)").weak());
        return;
    }
    ui.label(
        RichText::new(format!("{} commits", graph.commits.len()))
            .small()
            .weak(),
    );
    ScrollArea::vertical()
        .id_salt("lgtm-graph")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, commit) in graph.commits.iter().enumerate() {
                render_row(ui, i, commit.lane, commit, &graph.edges);
            }
        });
}

fn render_row(
    ui: &mut egui::Ui,
    row_idx: usize,
    lane: usize,
    commit: &lgtm_core::CommitNode,
    edges: &[GraphEdge],
) {
    let avail = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(avail, ROW_HEIGHT), Sense::hover());

    // ---- graph gutter -------------------------------------------------
    let gutter_left = rect.left();
    let gutter_top = rect.top();
    let gutter_bottom = rect.bottom();
    let center_y = (gutter_top + gutter_bottom) * 0.5;
    let painter = ui.painter_at(rect);

    // Draw any edges that pass through this row (either originating
    // from this row, or that span between visible rows that cross this
    // y position).
    for edge in edges {
        // Only draw an edge once at its child row, dropping down to the
        // parent (or off the bottom of the window).
        if edge.child != row_idx {
            continue;
        }
        let from = egui::pos2(gutter_left + lane_x(edge.child_lane), center_y);
        let to_y = if edge.parent == usize::MAX {
            // Edge exits off the bottom.
            gutter_bottom + ROW_HEIGHT * 2.0
        } else if edge.parent > row_idx {
            gutter_top + (edge.parent - row_idx) as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5
        } else {
            center_y
        };
        let to = egui::pos2(gutter_left + lane_x(edge.parent_lane), to_y);
        painter.line_segment([from, to], Stroke::new(1.5, lane_color(edge.parent_lane)));
    }

    // Draw this commit's dot on top of any lines.
    let dot = egui::pos2(gutter_left + lane_x(lane), center_y);
    painter.circle_filled(dot, DOT_RADIUS, lane_color(lane));
    painter.circle_stroke(dot, DOT_RADIUS, Stroke::new(1.0, Color32::WHITE));

    // ---- text columns ------------------------------------------------
    let text_left = gutter_left + GRAPH_GUTTER_WIDTH;
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(text_left, gutter_top),
        egui::pos2(rect.right(), gutter_bottom),
    );
    let mut text_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    text_ui.label(
        RichText::new(commit.short_sha())
            .color(Color32::from_rgb(0xaa, 0xaa, 0x55))
            .font(FontId::monospace(12.0)),
    );
    text_ui.label(
        RichText::new(format!(" {}  ", commit.author))
            .color(Color32::from_gray(0x88))
            .small(),
    );
    let summary = truncate(&commit.summary, 60);
    text_ui.label(RichText::new(summary).font(FontId::monospace(12.0)));
}

fn lane_x(lane: usize) -> f32 {
    LANE_WIDTH * (lane as f32 + 0.5)
}

/// Stable per-lane color so branching lines are visually distinguishable.
fn lane_color(lane: usize) -> Color32 {
    const PALETTE: &[Color32] = &[
        Color32::from_rgb(0x55, 0xaa, 0xff),
        Color32::from_rgb(0xff, 0x88, 0x55),
        Color32::from_rgb(0x55, 0xff, 0xaa),
        Color32::from_rgb(0xff, 0x55, 0xaa),
        Color32::from_rgb(0xaa, 0xaa, 0x55),
        Color32::from_rgb(0xff, 0xff, 0x88),
    ];
    PALETTE[lane % PALETTE.len()]
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let head: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_x_is_monotonic_in_lane() {
        assert!(lane_x(0) < lane_x(1));
        assert!(lane_x(1) < lane_x(2));
    }

    #[test]
    fn lane_color_wraps_through_palette() {
        // Same color at lane 0 and lane PALETTE_LEN.
        assert_eq!(lane_color(0), lane_color(6));
    }

    #[test]
    fn truncate_passes_short_strings_through() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_with_ellipsis_for_long_strings() {
        let t = truncate("abcdefghij", 5);
        assert_eq!(t.chars().count(), 5);
        assert!(t.ends_with('…'));
    }
}
