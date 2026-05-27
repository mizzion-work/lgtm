//! Three-way merge window.

use std::path::PathBuf;

use egui::{Align, Color32, FontId, Key, Layout, RichText, ScrollArea};
use lgtm_core::{MergeRegion, RecentEntry, RecentList, RecentMode, Settings, Side, ThreeWayMerge};

use crate::menubar::{self, MenuAction, MenuContext};

/// Reason the merge window closed; the CLI translates this to an exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeExit {
    /// User saved a fully-resolved merge.
    Saved,
    /// User quit without saving.
    Aborted,
}

/// The egui app driving the three-way merge window.
pub struct MergeApp {
    /// The merge state. Conflict resolutions live inside `regions`.
    pub merge: ThreeWayMerge,
    /// Where to write the merge result on save.
    pub output_path: PathBuf,
    /// What happened when the window closed.
    pub exit: Option<MergeExit>,
    show_confirm_unresolved: bool,
    show_confirm_abort: bool,
    save_error: Option<String>,
    /// User preferences (font, theme); shared store with other windows.
    pub settings: Settings,
    /// Recent-files list (read-only here — merge windows don't push).
    pub recents: RecentList,
    /// About modal visibility.
    show_about: bool,
    /// Shortcuts modal visibility.
    show_shortcuts: bool,
}

impl MergeApp {
    /// Construct a new merge app.
    pub fn new(merge: ThreeWayMerge, output_path: PathBuf) -> Self {
        Self {
            merge,
            output_path,
            exit: None,
            show_confirm_unresolved: false,
            show_confirm_abort: false,
            save_error: None,
            settings: Settings::load(),
            recents: RecentList::load(),
            show_about: false,
            show_shortcuts: false,
        }
    }

    fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Save => self.try_save(),
            MenuAction::Quit => self.show_confirm_abort = true,
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
            // Edit / hunk-nav / editor / view-toggles / git-graph are
            // not applicable in merge mode — supports flags suppress the
            // menu items so users shouldn't see them. Ignore if fired.
            _ => {}
        }
    }

    fn title(&self) -> String {
        format!("lgtm — merging {}", self.output_path.display())
    }

    fn resolved_count(&self) -> usize {
        self.merge.total_conflicts() - self.merge.unresolved_conflicts()
    }

    fn try_save(&mut self) {
        if self.merge.unresolved_conflicts() > 0 {
            self.show_confirm_unresolved = true;
            return;
        }
        match self.merge.render() {
            Ok(text) => {
                if let Err(e) = std::fs::write(&self.output_path, text) {
                    self.save_error = Some(e.to_string());
                } else {
                    self.exit = Some(MergeExit::Saved);
                }
            }
            Err(e) => self.save_error = Some(e.to_string()),
        }
    }
}

impl eframe::App for MergeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::diff_app::apply_app_theme(&ctx, self.settings.app_theme);

        let mut want_save = false;
        let mut want_quit = false;
        ctx.input(|i| {
            if i.modifiers.command_only() && i.key_pressed(Key::S) {
                want_save = true;
            }
            if i.key_pressed(Key::Escape) {
                want_quit = true;
            }
        });
        if want_save {
            self.try_save();
        }
        if want_quit {
            self.show_confirm_abort = true;
        }

        if let Some(exit) = self.exit {
            tracing::info!("merge exit: {exit:?}");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Menubar (File / Help — Edit/View suppressed via supports flags).
        let mut actions: Vec<MenuAction> = Vec::new();
        egui::Panel::top("lgtm-merge-menubar").show_inside(ui, |ui| {
            let mctx = MenuContext {
                dirty: self.merge.unresolved_conflicts() == 0,
                supports_edit_mode: false,
                in_edit_mode: false,
                supports_hunk_nav: false,
                supports_editor: false,
                read_only: false,
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

        egui::Panel::top("lgtm-merge-title").show_inside(ui, |ui| {
            ui.heading(self.title());
            ui.horizontal(|ui| {
                let total = self.merge.total_conflicts();
                let resolved = self.resolved_count();
                ui.label(format!("{resolved} of {total} conflicts resolved"));
                ui.separator();
                if ui.button("Save (Ctrl+S)").clicked() {
                    self.try_save();
                }
                if ui.button("Abort merge").clicked() {
                    self.show_confirm_abort = true;
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for idx in 0..self.merge.regions.len() {
                        render_region(ui, &mut self.merge.regions[idx], idx);
                    }
                });
        });

        if self.show_confirm_unresolved {
            self.render_confirm_unresolved(&ctx);
        }
        if self.show_confirm_abort {
            self.render_confirm_abort(&ctx);
        }
        if self.save_error.is_some() {
            self.render_save_error(&ctx);
        }
        if self.show_about {
            let mut open = true;
            egui::Window::new("About lgtm")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(&ctx, |ui| {
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
                .show(&ctx, |ui| {
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

/// Spawn a new file-mode `lgtm` window via the current binary so the
/// user can pick fresh files without quitting the merge.
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

fn spawn_from_entry(entry: &RecentEntry) {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(me);
    if entry.mode == RecentMode::Folder {
        cmd.arg("--dir");
    }
    let _ = cmd.arg(&entry.left).arg(&entry.right).spawn();
}

impl MergeApp {
    fn render_confirm_unresolved(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Unresolved conflicts")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} conflict(s) are still unresolved. Save with conflict markers?",
                    self.merge.unresolved_conflicts()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Save with markers").clicked() {
                        let text = self.merge.render_with_markers();
                        if let Err(e) = std::fs::write(&self.output_path, text) {
                            self.save_error = Some(e.to_string());
                        } else {
                            self.exit = Some(MergeExit::Saved);
                        }
                        self.show_confirm_unresolved = false;
                    }
                    if ui.button("Keep editing").clicked() {
                        self.show_confirm_unresolved = false;
                    }
                });
            });
        if !open {
            self.show_confirm_unresolved = false;
        }
    }

    fn render_confirm_abort(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Abort merge?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Discard the merge and exit?");
                ui.horizontal(|ui| {
                    if ui.button("Abort").clicked() {
                        self.exit = Some(MergeExit::Aborted);
                        self.show_confirm_abort = false;
                    }
                    if ui.button("Keep editing").clicked() {
                        self.show_confirm_abort = false;
                    }
                });
            });
        if !open {
            self.show_confirm_abort = false;
        }
    }

    fn render_save_error(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let msg = self.save_error.clone().unwrap_or_default();
        egui::Window::new("Save failed")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(msg);
                if ui.button("OK").clicked() {
                    self.save_error = None;
                }
            });
        if !open {
            self.save_error = None;
        }
    }
}

fn render_region(ui: &mut egui::Ui, region: &mut MergeRegion, idx: usize) {
    match region {
        MergeRegion::Stable { text } => {
            ui.scope(|ui| {
                ui.style_mut().override_font_id = Some(FontId::monospace(13.0));
                ui.label(strip_trailing_nl(text));
            });
        }
        MergeRegion::Resolvable { take, text } => {
            let badge = match take {
                Side::Left => RichText::new(" [auto: LOCAL] ")
                    .background_color(Color32::from_rgb(0x10, 0x40, 0x10)),
                Side::Right => RichText::new(" [auto: REMOTE] ")
                    .background_color(Color32::from_rgb(0x10, 0x40, 0x10)),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(badge);
                ui.scope(|ui| {
                    ui.style_mut().override_font_id = Some(FontId::monospace(13.0));
                    ui.label(strip_trailing_nl(text));
                });
            });
        }
        MergeRegion::Conflict {
            local,
            base,
            remote,
            resolution,
        } => render_conflict(ui, idx, local, base, remote, resolution),
    }
}

fn render_conflict(
    ui: &mut egui::Ui,
    idx: usize,
    local: &str,
    base: &str,
    remote: &str,
    resolution: &mut Option<String>,
) {
    let frame = egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0,
            if resolution.is_some() {
                Color32::from_rgb(0x40, 0x80, 0x40)
            } else {
                Color32::from_rgb(0xc0, 0x40, 0x40)
            },
        ))
        .inner_margin(8.0);
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(format!(
                "Conflict #{}  {}",
                idx,
                if resolution.is_some() {
                    "(resolved)"
                } else {
                    "(unresolved)"
                }
            ));
        });

        ui.columns(3, |cols| {
            for (col, (label, text)) in
                cols.iter_mut()
                    .zip([("LOCAL", local), ("BASE", base), ("REMOTE", remote)])
            {
                col.label(RichText::new(label).strong());
                col.scope(|ui| {
                    ui.style_mut().override_font_id = Some(FontId::monospace(13.0));
                    ui.label(strip_trailing_nl(text));
                });
            }
        });

        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            if ui.button("Take LOCAL").clicked() {
                *resolution = Some(local.to_string());
            }
            if ui.button("Take REMOTE").clicked() {
                *resolution = Some(remote.to_string());
            }
            if ui.button("Take BOTH (local first)").clicked() {
                let mut s = String::new();
                s.push_str(local);
                if !local.ends_with('\n') {
                    s.push('\n');
                }
                s.push_str(remote);
                *resolution = Some(s);
            }
            if ui.button("Take BOTH (remote first)").clicked() {
                let mut s = String::new();
                s.push_str(remote);
                if !remote.ends_with('\n') {
                    s.push('\n');
                }
                s.push_str(local);
                *resolution = Some(s);
            }
            if ui.button("Take BASE").clicked() {
                *resolution = Some(base.to_string());
            }
            if resolution.is_some() && ui.button("Reset").clicked() {
                *resolution = None;
            }
        });

        if let Some(r) = resolution.as_mut() {
            ui.label(RichText::new("Resolution (editable):").strong());
            ui.add(
                egui::TextEdit::multiline(r)
                    .font(FontId::monospace(13.0))
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
        }
    });
}

fn strip_trailing_nl(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}
