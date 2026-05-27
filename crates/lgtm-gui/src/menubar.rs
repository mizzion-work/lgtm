//! Menubar shared between window types.
//!
//! Renders File / Edit / View / Help menus and reports user intent via
//! a [`MenuAction`] enum. Side-effect-free: rendering this widget never
//! touches the filesystem or spawns processes — the caller handles each
//! returned action so the menubar stays testable and re-renderable.

use egui::{Key, KeyboardShortcut, Modifiers};
use lgtm_core::{AppTheme, EditorTheme, RecentEntry, RecentList, RecentMode};

/// User intent emitted by [`render_menubar`].
///
/// Multiple actions can fire in one frame in principle, but in practice
/// only one menu item is clicked per frame, so callers can simply drain
/// `Vec<MenuAction>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Open a new file pair via native picker.
    OpenFiles,
    /// Open a new folder pair via native picker.
    OpenFolders,
    /// Re-open a previously-opened entry.
    OpenRecent(RecentEntry),
    /// Clear the recent-files list.
    ClearRecent,
    /// Save the modified pane(s).
    Save,
    /// Quit the window.
    Quit,
    /// Toggle between view / edit mode.
    ToggleEditMode,
    /// Launch the configured editor on the focused pane.
    OpenInEditor,
    /// Launch the configured editor on both panes.
    OpenBothInEditor,
    /// Jump to the next hunk (mirrors the `n` key).
    NextHunk,
    /// Jump to the previous hunk (mirrors the `p` key).
    PrevHunk,
    /// Jump to the first hunk (mirrors `Ctrl+Home`).
    FirstHunk,
    /// Jump to the last hunk (mirrors `Ctrl+End`).
    LastHunk,
    /// Show the "Keyboard Shortcuts" modal.
    ShowShortcuts,
    /// Show the About modal.
    ShowAbout,
    /// Bump font size by one step.
    IncreaseFontSize,
    /// Reduce font size by one step.
    DecreaseFontSize,
    /// Reset font size to the default.
    ResetFontSize,
    /// Switch the egui chrome theme (window background, menus, buttons).
    SetAppTheme(AppTheme),
    /// Switch the syntect theme used to color source code.
    SetEditorTheme(EditorTheme),
    /// Toggle the git-graph drawer.
    ToggleGitGraph,
    /// Toggle word-wrap (long lines reflow at pane width).
    ToggleWordWrap,
    /// Toggle ignore-whitespace mode in the diff computation.
    ToggleIgnoreWhitespace,
}

/// What state the host window can do at the moment. Drives whether items
/// are enabled or greyed out.
#[derive(Debug, Clone, Copy)]
pub struct MenuContext {
    /// `true` if Save should be enabled.
    pub dirty: bool,
    /// `true` if the host supports edit mode (DiffApp does; MergeApp/FolderApp don't).
    pub supports_edit_mode: bool,
    /// `true` if the host is currently in edit mode (changes the toggle label).
    pub in_edit_mode: bool,
    /// `true` if hunk navigation is meaningful (DiffApp only).
    pub supports_hunk_nav: bool,
    /// `true` if the editor key actions apply.
    pub supports_editor: bool,
    /// `true` if `--read-only` was passed; toggle/save are hidden.
    pub read_only: bool,
    /// Currently-selected app theme (highlighted with a check in the menu).
    pub app_theme: AppTheme,
    /// Currently-selected editor theme (highlighted with a check in the menu).
    pub editor_theme: EditorTheme,
    /// `true` if the git-graph drawer is currently visible.
    pub git_graph_visible: bool,
    /// Current word-wrap setting (for ✔ marking).
    pub word_wrap: bool,
    /// Current ignore-whitespace setting (for ✔ marking).
    pub ignore_whitespace: bool,
}

impl Default for MenuContext {
    fn default() -> Self {
        Self {
            dirty: false,
            supports_edit_mode: true,
            in_edit_mode: false,
            supports_hunk_nav: true,
            supports_editor: true,
            read_only: false,
            app_theme: AppTheme::Auto,
            editor_theme: EditorTheme::default(),
            git_graph_visible: false,
            word_wrap: false,
            ignore_whitespace: false,
        }
    }
}

/// Keyboard shortcut: Ctrl/Cmd+S.
pub const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
/// Keyboard shortcut: Ctrl/Cmd+O.
pub const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
/// Keyboard shortcut: Ctrl/Cmd+Shift+O.
pub const SC_OPEN_FOLDER: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
/// Keyboard shortcut: Ctrl/Cmd+Q.
pub const SC_QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
/// Keyboard shortcut: Ctrl/Cmd++ (increase font).
pub const SC_FONT_UP: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals);
/// Keyboard shortcut: Ctrl/Cmd+- (decrease font).
pub const SC_FONT_DOWN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
/// Keyboard shortcut: Ctrl/Cmd+0 (reset font).
pub const SC_FONT_RESET: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0);

/// Render the menubar. Appends emitted [`MenuAction`]s into `out`.
pub fn render_menubar(
    ui: &mut egui::Ui,
    ctx: MenuContext,
    recents: &RecentList,
    out: &mut Vec<MenuAction>,
) {
    egui::menu::bar(ui, |ui| {
        // ---- File ---------------------------------------------------
        ui.menu_button("File", |ui| {
            if shortcut_button(ui, "Open Files…", Some(&SC_OPEN)).clicked() {
                out.push(MenuAction::OpenFiles);
                ui.close_menu();
            }
            if shortcut_button(ui, "Open Folders…", Some(&SC_OPEN_FOLDER)).clicked() {
                out.push(MenuAction::OpenFolders);
                ui.close_menu();
            }
            ui.menu_button("Open Recent", |ui| {
                if recents.is_empty() {
                    ui.label(egui::RichText::new("(empty)").weak());
                } else {
                    for entry in recents.entries() {
                        let label = recent_label(entry);
                        if ui.button(label).clicked() {
                            out.push(MenuAction::OpenRecent(entry.clone()));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Clear Recent").clicked() {
                        out.push(MenuAction::ClearRecent);
                        ui.close_menu();
                    }
                }
            });
            ui.separator();
            if !ctx.read_only {
                let save = ui.add_enabled(
                    ctx.dirty,
                    egui::Button::new("Save").shortcut_text(ui.ctx().format_shortcut(&SC_SAVE)),
                );
                if save.clicked() {
                    out.push(MenuAction::Save);
                    ui.close_menu();
                }
            }
            ui.separator();
            if shortcut_button(ui, "Quit", Some(&SC_QUIT)).clicked() {
                out.push(MenuAction::Quit);
                ui.close_menu();
            }
        });

        // ---- Edit ---------------------------------------------------
        ui.menu_button("Edit", |ui| {
            if ctx.supports_edit_mode && !ctx.read_only {
                let label = if ctx.in_edit_mode {
                    "Switch to View mode"
                } else {
                    "Switch to Edit mode"
                };
                if ui.button(label).clicked() {
                    out.push(MenuAction::ToggleEditMode);
                    ui.close_menu();
                }
                ui.separator();
            }
            if ctx.supports_editor {
                if shortcut_button(
                    ui,
                    "Open in Editor",
                    Some(&KeyboardShortcut::new(Modifiers::NONE, Key::E)),
                )
                .clicked()
                {
                    out.push(MenuAction::OpenInEditor);
                    ui.close_menu();
                }
                if shortcut_button(
                    ui,
                    "Open Both in Editor",
                    Some(&KeyboardShortcut::new(Modifiers::SHIFT, Key::E)),
                )
                .clicked()
                {
                    out.push(MenuAction::OpenBothInEditor);
                    ui.close_menu();
                }
            }
        });

        // ---- View ---------------------------------------------------
        if ctx.supports_hunk_nav {
            ui.menu_button("View", |ui| {
                if shortcut_button(
                    ui,
                    "Next Hunk",
                    Some(&KeyboardShortcut::new(Modifiers::NONE, Key::N)),
                )
                .clicked()
                {
                    out.push(MenuAction::NextHunk);
                    ui.close_menu();
                }
                if shortcut_button(
                    ui,
                    "Previous Hunk",
                    Some(&KeyboardShortcut::new(Modifiers::NONE, Key::P)),
                )
                .clicked()
                {
                    out.push(MenuAction::PrevHunk);
                    ui.close_menu();
                }
                ui.separator();
                if shortcut_button(
                    ui,
                    "First Hunk",
                    Some(&KeyboardShortcut::new(Modifiers::CTRL, Key::Home)),
                )
                .clicked()
                {
                    out.push(MenuAction::FirstHunk);
                    ui.close_menu();
                }
                if shortcut_button(
                    ui,
                    "Last Hunk",
                    Some(&KeyboardShortcut::new(Modifiers::CTRL, Key::End)),
                )
                .clicked()
                {
                    out.push(MenuAction::LastHunk);
                    ui.close_menu();
                }
                ui.separator();
                ui.menu_button("Font Size", |ui| {
                    if shortcut_button(ui, "Increase", Some(&SC_FONT_UP)).clicked() {
                        out.push(MenuAction::IncreaseFontSize);
                        ui.close_menu();
                    }
                    if shortcut_button(ui, "Decrease", Some(&SC_FONT_DOWN)).clicked() {
                        out.push(MenuAction::DecreaseFontSize);
                        ui.close_menu();
                    }
                    if shortcut_button(ui, "Reset", Some(&SC_FONT_RESET)).clicked() {
                        out.push(MenuAction::ResetFontSize);
                        ui.close_menu();
                    }
                });
                ui.menu_button("App Theme", |ui| {
                    for (label, theme) in [
                        ("Auto (follow OS)", AppTheme::Auto),
                        ("Light", AppTheme::Light),
                        ("Dark", AppTheme::Dark),
                    ] {
                        let mark = if ctx.app_theme == theme {
                            "✔ "
                        } else {
                            "   "
                        };
                        if ui.button(format!("{mark}{label}")).clicked() {
                            out.push(MenuAction::SetAppTheme(theme));
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("Editor Theme", |ui| {
                    for theme in EditorTheme::all() {
                        let mark = if ctx.editor_theme == *theme {
                            "✔ "
                        } else {
                            "   "
                        };
                        if ui.button(format!("{mark}{}", theme.label())).clicked() {
                            out.push(MenuAction::SetEditorTheme(*theme));
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                let wrap_label = if ctx.word_wrap {
                    "✔ Word Wrap"
                } else {
                    "   Word Wrap"
                };
                if ui.button(wrap_label).clicked() {
                    out.push(MenuAction::ToggleWordWrap);
                    ui.close_menu();
                }
                let ws_label = if ctx.ignore_whitespace {
                    "✔ Ignore Whitespace"
                } else {
                    "   Ignore Whitespace"
                };
                if ui.button(ws_label).clicked() {
                    out.push(MenuAction::ToggleIgnoreWhitespace);
                    ui.close_menu();
                }
                ui.separator();
                let graph_label = if ctx.git_graph_visible {
                    "✔ Show Git Graph"
                } else {
                    "   Show Git Graph"
                };
                if ui.button(graph_label).clicked() {
                    out.push(MenuAction::ToggleGitGraph);
                    ui.close_menu();
                }
            });
        }

        // ---- Help ---------------------------------------------------
        ui.menu_button("Help", |ui| {
            if ui.button("Keyboard Shortcuts").clicked() {
                out.push(MenuAction::ShowShortcuts);
                ui.close_menu();
            }
            ui.separator();
            if ui
                .button(format!("About lgtm v{}", env!("CARGO_PKG_VERSION")))
                .clicked()
            {
                out.push(MenuAction::ShowAbout);
                ui.close_menu();
            }
        });
    });
}

fn shortcut_button(ui: &mut egui::Ui, text: &str, sc: Option<&KeyboardShortcut>) -> egui::Response {
    let mut button = egui::Button::new(text.to_string());
    if let Some(sc) = sc {
        button = button.shortcut_text(ui.ctx().format_shortcut(sc));
    }
    ui.add(button)
}

/// Trim a path down to its last component(s) for display in the menu.
/// Falls back to the full path for unusual inputs.
fn recent_label(entry: &RecentEntry) -> String {
    let prefix = match entry.mode {
        RecentMode::File => "📄",
        RecentMode::Folder => "📁",
    };
    let l = short_path(&entry.left);
    let r = short_path(&entry.right);
    format!("{prefix}  {l} ↔ {r}")
}

fn short_path(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Body of the "About lgtm" modal — separate so the host window can
/// render it inside its preferred chrome.
pub fn about_body() -> String {
    format!(
        "  ╦  ╔═╗╔╦╗╔╦╗\n  ║  ║ ╦ ║ ║║║\n  ╩═╝╚═╝ ╩ ╩ ╩\n  from wtf to lgtm — v{}\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// Body of the "Keyboard Shortcuts" modal.
pub fn shortcuts_body() -> &'static str {
    "FILE\n  \
     Ctrl+O             open files…\n  \
     Ctrl+Shift+O       open folders…\n  \
     Ctrl+S             save modified panes\n  \
     Ctrl+Q             quit\n  \
     Esc                close find bar, then close window (confirms if dirty)\n\
     \n\
     FIND\n  \
     Ctrl+F             open the find bar\n  \
     Enter              next match (in find input)\n  \
     Shift+Enter        previous match\n  \
     Aa toggle          case-sensitive\n  \
     .* toggle          regex mode\n\
     \n\
     HUNKS  (view mode only — disabled while typing)\n  \
     n / p              next / previous hunk\n  \
     Ctrl+Home / End    first / last hunk\n\
     \n\
     EDITOR INTEGRATION  (view mode only)\n  \
     e                  open hovered file at hovered line in $EDITOR\n  \
     Shift+E            open both panes in $EDITOR\n\
     \n\
     FONT\n  \
     Ctrl+=             increase\n  \
     Ctrl+-             decrease\n  \
     Ctrl+0             reset to default\n\
     \n\
     MOUSE\n  \
     hunk → / ←         copy this hunk left→right or right→left\n  \
     hover any line     show git blame (silent if unavailable)\n  \
     minimap click      jump to nearest hunk\n  \
     drag splitter      (edit mode) resize the two panes\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn recent_label_uses_basenames() {
        let e = RecentEntry::new(
            RecentMode::File,
            PathBuf::from("/long/path/to/a.txt"),
            PathBuf::from("/elsewhere/b.txt"),
        );
        let label = recent_label(&e);
        assert!(label.contains("a.txt"));
        assert!(label.contains("b.txt"));
        assert!(label.contains("↔"));
    }

    #[test]
    fn recent_label_distinguishes_folder_from_file() {
        let f = RecentEntry::new(RecentMode::File, "/a".into(), "/b".into());
        let d = RecentEntry::new(RecentMode::Folder, "/x".into(), "/y".into());
        assert_ne!(recent_label(&f), recent_label(&d));
    }

    #[test]
    fn about_body_contains_version_and_banner() {
        let body = about_body();
        assert!(body.contains("lgtm"));
        assert!(body.contains(env!("CARGO_PKG_VERSION")));
        assert!(body.contains("╦"));
    }

    #[test]
    fn shortcuts_body_lists_main_keys() {
        let body = shortcuts_body();
        for k in ["Ctrl+S", "Ctrl+O", "Esc", "Shift+E"] {
            assert!(body.contains(k), "missing {k}");
        }
    }
}
