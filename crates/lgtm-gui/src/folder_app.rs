//! Folder-diff window.

use egui::{Color32, FontId, Key, RichText, ScrollArea, Sense};
use lgtm_core::{FolderDiff, FolderEntryStatus, RecentList, Settings};

use crate::menubar::{self, MenuAction, MenuContext};
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
    /// User preferences (font, theme); shared store with other windows.
    pub settings: Settings,
    /// Recent-files list (for File → Open Recent).
    pub recents: RecentList,
    /// About modal visibility.
    show_about: bool,
    /// Shortcuts modal visibility.
    show_shortcuts: bool,
}

impl FolderApp {
    /// Construct an app from a precomputed [`FolderDiff`].
    pub fn new(diff: FolderDiff) -> Self {
        Self {
            diff,
            filters: FolderFilters::default(),
            close_requested: false,
            settings: Settings::load(),
            recents: RecentList::load(),
            show_about: false,
            show_shortcuts: false,
        }
    }

    /// Apply current filters to the entries.
    pub fn visible_entries(&self) -> impl Iterator<Item = &lgtm_core::FolderEntry> {
        let f = self.filters;
        self.diff.entries.iter().filter(move |e| match e.status {
            FolderEntryStatus::Identical => !f.hide_identical,
            FolderEntryStatus::LeftOnly => !f.hide_left_only,
            FolderEntryStatus::RightOnly => !f.hide_right_only,
            _ => true,
        })
    }

    fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Quit => self.close_requested = true,
            MenuAction::ShowAbout => self.show_about = true,
            MenuAction::ShowShortcuts => self.show_shortcuts = true,
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
            MenuAction::SetAppTheme(t) => {
                self.settings.app_theme = t;
                let _ = self.settings.save();
            }
            MenuAction::SetEditorTheme(t) => {
                self.settings.editor_theme = t;
                let _ = self.settings.save();
            }
            MenuAction::OpenFiles => spawn_lgtm_files(),
            MenuAction::OpenFolders => spawn_lgtm_folders(),
            MenuAction::OpenRecent(entry) => spawn_from_entry(&entry),
            MenuAction::ClearRecent => {
                self.recents.clear();
                let _ = self.recents.save();
            }
            // No save (folder view is read-only), no edit/hunk/editor.
            _ => {}
        }
    }
}

impl eframe::App for FolderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::diff_app::apply_app_theme(ctx, self.settings.app_theme);

        let typing = ctx.wants_keyboard_input();
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                self.close_requested = true;
            }
            if !typing && i.key_pressed(Key::Q) && !i.modifiers.command {
                self.close_requested = true;
            }
        });
        if self.close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Menubar (File / Help + theme + font; no edit/hunk/editor).
        let mut actions: Vec<MenuAction> = Vec::new();
        egui::TopBottomPanel::top("lgtm-folder-menubar").show(ctx, |ui| {
            let mctx = MenuContext {
                dirty: false,
                supports_edit_mode: false,
                in_edit_mode: false,
                supports_hunk_nav: false,
                supports_editor: false,
                read_only: true,
                app_theme: self.settings.app_theme,
                editor_theme: self.settings.editor_theme,
                git_graph_visible: false,
                word_wrap: self.settings.word_wrap,
                ignore_whitespace: self.settings.ignore_whitespace,
            };
            menubar::render_menubar(ui, mctx, &self.recents, &mut actions);
        });
        for action in actions {
            self.handle_menu_action(action);
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

        let mut clicked: Option<(std::path::PathBuf, std::path::PathBuf)> = None;
        let left_root = self.diff.left_root.clone();
        let right_root = self.diff.right_root.clone();
        let font_size = self.settings.font_size;
        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for e in self.visible_entries() {
                        let (badge, color) = badge_for(e.status);
                        let resp = ui
                            .horizontal(|ui| {
                                ui.style_mut().override_font_id =
                                    Some(FontId::monospace(font_size));
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
                            })
                            .response;
                        let r = resp.rect;
                        let interact = ui.interact(
                            r,
                            ui.id().with(("row", e.relative_path.clone())),
                            Sense::click(),
                        );
                        if interact.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if interact.clicked() {
                            let openable = matches!(
                                e.status,
                                FolderEntryStatus::Modified
                                    | FolderEntryStatus::LeftOnly
                                    | FolderEntryStatus::RightOnly
                            );
                            if openable {
                                let l = left_root.join(&e.relative_path);
                                let r = right_root.join(&e.relative_path);
                                clicked = Some((l, r));
                            }
                        }
                    }
                });
        });
        if let Some((l, r)) = clicked {
            // Spawn a fresh file-mode lgtm window for the picked entry.
            // Use /dev/null for the absent side of LeftOnly / RightOnly.
            let l_arg = if l.exists() {
                l
            } else {
                std::path::PathBuf::from("/dev/null")
            };
            let r_arg = if r.exists() {
                r
            } else {
                std::path::PathBuf::from("/dev/null")
            };
            if let Ok(me) = std::env::current_exe() {
                let _ = std::process::Command::new(me).arg(l_arg).arg(r_arg).spawn();
            }
        }

        if self.show_about {
            let mut open = true;
            egui::Window::new("About lgtm")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(RichText::new(menubar::about_body()).monospace());
                    if ui.button("OK").clicked() {
                        self.show_about = false;
                    }
                });
            if !open {
                self.show_about = false;
            }
        }
        if self.show_shortcuts {
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
    }
}

fn spawn_lgtm_files() {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
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
    let _ = std::process::Command::new(me).arg(left).arg(right).spawn();
}

fn spawn_lgtm_folders() {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
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
    let _ = std::process::Command::new(me)
        .arg("--dir")
        .arg(left)
        .arg(right)
        .spawn();
}

fn spawn_from_entry(entry: &lgtm_core::RecentEntry) {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(me);
    if entry.mode == lgtm_core::RecentMode::Folder {
        cmd.arg("--dir");
    }
    let _ = cmd.arg(&entry.left).arg(&entry.right).spawn();
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
