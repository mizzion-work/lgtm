//! Folder-diff window.

use egui::{Color32, FontId, Key, RichText, ScrollArea};
use lgtm_core::{FolderDiff, FolderEntryStatus};

use crate::theme;

/// Visibility toggles applied on top of [`FolderDiff::entries`].
#[derive(Debug, Clone, Copy)]
pub struct FolderFilters {
    /// Hide [`FolderEntryStatus::Identical`] entries.
    pub hide_identical: bool,
    /// Hide [`FolderEntryStatus::LeftOnly`] entries.
    pub hide_left_only: bool,
    /// Hide [`FolderEntryStatus::RightOnly`] entries.
    pub hide_right_only: bool,
}

impl Default for FolderFilters {
    fn default() -> Self {
        Self {
            hide_identical: true,
            hide_left_only: false,
            hide_right_only: false,
        }
    }
}

/// The egui app driving the folder-diff window.
pub struct FolderApp {
    /// The computed diff.
    pub diff: FolderDiff,
    /// Filter toggles.
    pub filters: FolderFilters,
    /// Set when the user requests close.
    pub close_requested: bool,
}

impl FolderApp {
    /// Construct an app from a precomputed [`FolderDiff`].
    pub fn new(diff: FolderDiff) -> Self {
        Self {
            diff,
            filters: FolderFilters::default(),
            close_requested: false,
        }
    }

    /// Apply current filters to the entries.
    pub fn visible_entries(&self) -> impl Iterator<Item = &lgtm_core::FolderEntry> {
        let f = self.filters;
        self.diff.entries.iter().filter(move |e| {
            !(f.hide_identical && e.status == FolderEntryStatus::Identical)
                && !(f.hide_left_only && e.status == FolderEntryStatus::LeftOnly)
                && !(f.hide_right_only && e.status == FolderEntryStatus::RightOnly)
        })
    }
}

impl eframe::App for FolderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) || i.key_pressed(Key::Q) {
                self.close_requested = true;
            }
        });
        if self.close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::TopBottomPanel::top("lgtm-folder-title").show(ctx, |ui| {
            ui.heading(format!(
                "lgtm — {} ↔ {}",
                self.diff.left_root.display(),
                self.diff.right_root.display()
            ));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.filters.hide_identical, "Hide identical");
                ui.checkbox(&mut self.filters.hide_left_only, "Hide left-only");
                ui.checkbox(&mut self.filters.hide_right_only, "Hide right-only");
                ui.label(format!("{} total entries", self.diff.entries.len()));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for e in self.visible_entries() {
                        let (badge, color) = badge_for(e.status);
                        ui.horizontal(|ui| {
                            ui.scope(|ui| {
                                ui.style_mut().override_font_id = Some(FontId::monospace(13.0));
                                ui.label(
                                    RichText::new(format!(" {badge} ")).background_color(color),
                                );
                                ui.label(
                                    RichText::new(e.relative_path.display().to_string())
                                        .monospace(),
                                );
                                let sizes = format!(
                                    "  L={}  R={}",
                                    e.left_size
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| "-".into()),
                                    e.right_size
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| "-".into()),
                                );
                                ui.label(RichText::new(sizes).color(theme::GUTTER_FG));
                            });
                        });
                    }
                });
        });
    }
}

fn badge_for(status: FolderEntryStatus) -> (&'static str, Color32) {
    match status {
        FolderEntryStatus::Identical => ("=", Color32::from_gray(0x30)),
        FolderEntryStatus::Modified => ("~", theme::REPLACE_BG),
        FolderEntryStatus::LeftOnly => ("<", theme::DELETE_BG),
        FolderEntryStatus::RightOnly => (">", theme::INSERT_BG),
        FolderEntryStatus::TypeChanged => ("!", Color32::from_rgb(0x50, 0x10, 0x50)),
        FolderEntryStatus::BinaryDiffers => ("b", Color32::from_rgb(0x40, 0x40, 0x40)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgtm_core::{FolderEntry, FolderEntryStatus};
    use std::path::PathBuf;

    fn fixture() -> FolderApp {
        let entries = vec![
            FolderEntry {
                relative_path: PathBuf::from("same.txt"),
                status: FolderEntryStatus::Identical,
                left_size: Some(1),
                right_size: Some(1),
            },
            FolderEntry {
                relative_path: PathBuf::from("changed.txt"),
                status: FolderEntryStatus::Modified,
                left_size: Some(10),
                right_size: Some(20),
            },
            FolderEntry {
                relative_path: PathBuf::from("only_l.txt"),
                status: FolderEntryStatus::LeftOnly,
                left_size: Some(1),
                right_size: None,
            },
            FolderEntry {
                relative_path: PathBuf::from("only_r.txt"),
                status: FolderEntryStatus::RightOnly,
                left_size: None,
                right_size: Some(1),
            },
        ];
        FolderApp::new(FolderDiff {
            left_root: PathBuf::from("/l"),
            right_root: PathBuf::from("/r"),
            entries,
        })
    }

    #[test]
    fn hide_identical_default_drops_identical() {
        let app = fixture();
        let v: Vec<_> = app.visible_entries().collect();
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|e| e.status != FolderEntryStatus::Identical));
    }

    #[test]
    fn hide_left_only_drops_left_only() {
        let mut app = fixture();
        app.filters.hide_identical = false;
        app.filters.hide_left_only = true;
        let v: Vec<_> = app.visible_entries().collect();
        assert!(v.iter().all(|e| e.status != FolderEntryStatus::LeftOnly));
    }

    #[test]
    fn all_filters_off_shows_everything() {
        let mut app = fixture();
        app.filters.hide_identical = false;
        let v: Vec<_> = app.visible_entries().collect();
        assert_eq!(v.len(), 4);
    }
}
