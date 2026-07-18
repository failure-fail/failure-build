//! Dracula theme for the pager.
//!
//! Colors follow the official Dracula palette (draculatheme.com): a small
//! fixed set of background/foreground/accent hex values. A few background
//! shades and border tones outside that official set are derived (labeled
//! below) since the `Theme` struct has more background/border roles than
//! Dracula's spec defines.

use ratatui::style::{Color, Modifier};

use super::tokyonight::Theme;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[allow(dead_code)]
mod palette {
    use super::*;

    // Official Dracula palette.
    pub const BACKGROUND: Color = rgb(40, 42, 54); // #282a36
    pub const CURRENT_LINE: Color = rgb(68, 71, 90); // #44475a
    pub const FOREGROUND: Color = rgb(248, 248, 242); // #f8f8f2
    pub const COMMENT: Color = rgb(98, 114, 164); // #6272a4
    pub const CYAN: Color = rgb(139, 233, 253); // #8be9fd
    pub const GREEN: Color = rgb(80, 250, 123); // #50fa7b
    pub const ORANGE: Color = rgb(255, 184, 108); // #ffb86c
    pub const PINK: Color = rgb(255, 121, 198); // #ff79c6
    pub const PURPLE: Color = rgb(189, 147, 249); // #bd93f9
    pub const RED: Color = rgb(255, 85, 85); // #ff5555
    pub const YELLOW: Color = rgb(241, 250, 140); // #f1fa8c

    // Derived (not in the official 11-color spec): extra background depth
    // and a lighter-than-comment neutral for secondary text/borders.
    pub const BACKGROUND_DARK: Color = rgb(30, 31, 42);
    pub const BACKGROUND_DARKER: Color = rgb(23, 24, 33);
    pub const HOVER: Color = rgb(58, 61, 78);
    pub const SUBTLE: Color = rgb(158, 165, 194);
    pub const BORDER_ACTIVE: Color = rgb(90, 94, 120);
}
use palette::*;

impl Theme {
    /// Dracula theme (draculatheme.com).
    pub const fn dracula() -> Self {
        Self {
            bg_base: BACKGROUND,
            bg_light: CURRENT_LINE,
            bg_dark: BACKGROUND_DARK,
            bg_highlight: CURRENT_LINE,
            bg_hover: HOVER,
            bg_terminal: BACKGROUND_DARK,

            accent_user: PURPLE,
            accent_assistant: PINK,
            accent_thinking: COMMENT,
            accent_tool: SUBTLE,
            accent_system: PURPLE,
            accent_error: RED,
            accent_success: GREEN,
            accent_running: PINK,
            accent_skill: CYAN,

            text_primary: FOREGROUND,
            text_secondary: SUBTLE,

            gray_dim: CURRENT_LINE,
            gray: COMMENT,
            gray_bright: SUBTLE,

            command: YELLOW,
            path: ORANGE,
            running: CYAN,
            warning: YELLOW,

            fuzzy_accent: PURPLE,

            accent_plan: ORANGE,

            accent_verify: PURPLE,

            accent_feedback: GREEN,

            accent_remember: GREEN,

            selection_border: CURRENT_LINE,
            hover_border: HOVER,
            prompt_border: CURRENT_LINE,
            prompt_border_active: BORDER_ACTIVE,

            accent_model: CYAN,

            scrollbar_bg: BACKGROUND_DARKER,
            scrollbar_fg: CURRENT_LINE,

            diff_delete_bg: rgb(80, 25, 30),
            diff_delete_fg: RED,
            diff_insert_bg: rgb(20, 60, 30),
            diff_insert_fg: GREEN,
            diff_equal_fg: COMMENT,
            diff_gutter_fg: COMMENT,

            bg_visual: rgb(58, 61, 84),

            paste_bg: BACKGROUND_DARK,
            paste_fg: SUBTLE,
            paste_dim: COMMENT,

            md_heading_h1: PURPLE,
            md_heading_h1_mod: Modifier::BOLD,
            md_heading_h2: PINK,
            md_heading_h2_mod: Modifier::BOLD,
            md_heading_h3: ORANGE,
            md_heading_h3_mod: Modifier::BOLD,
            md_heading_h4: RED,
            md_heading_h4_mod: Modifier::BOLD,
            md_heading_h5: GREEN,
            md_heading_h5_mod: Modifier::BOLD,
            md_heading_h6: CYAN,
            md_heading_h6_mod: Modifier::BOLD,
            md_code: GREEN,
            md_task_checked: GREEN,
            md_task_unchecked: SUBTLE,
            md_muted: COMMENT,
            md_code_bg: BACKGROUND_DARK,
            md_text: FOREGROUND,
            link_fg: CYAN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dracula_theme() {
        let theme = Theme::dracula();
        assert!(matches!(theme.bg_base, Color::Rgb(40, 42, 54)));
        assert!(matches!(theme.accent_user, Color::Rgb(189, 147, 249)));
    }
}
