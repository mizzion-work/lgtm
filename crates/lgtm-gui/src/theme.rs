//! Color palette and shared style values for `lgtm`'s GUI.
//!
//! v1 ships dark-mode-friendly tints; light mode just renders against
//! egui's default light background and the same hues look fine.

use egui::Color32;

/// Background tint for a `Delete` row.
pub const DELETE_BG: Color32 = Color32::from_rgb(0x4a, 0x18, 0x1f);
/// Background tint for an `Insert` row.
pub const INSERT_BG: Color32 = Color32::from_rgb(0x18, 0x3f, 0x1d);
/// Background tint for a `Replace` row.
pub const REPLACE_BG: Color32 = Color32::from_rgb(0x4a, 0x40, 0x12);
/// Background tint for the inline-highlighted span inside a `Replace`.
pub const INLINE_BG: Color32 = Color32::from_rgb(0x88, 0x68, 0x0a);
/// Foreground color for the gutter line numbers.
pub const GUTTER_FG: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);

/// Width (in monospace characters) reserved for the line-number gutter.
pub const GUTTER_WIDTH_CHARS: usize = 5;
