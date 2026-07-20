//! First-launch / `/provider setup` wizard: pick a built-in provider, paste
//! an API key, finish. Reuses modal chrome; keeps keys out of `config.toml`
//! by going through the existing `AddProvider` → `add_byop_provider` path.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use xai_grok_shell::agent::config::{BuiltinProviderPreset, builtin_provider_presets};

use crate::theme::Theme;
use crate::views::modal_window::{
    ModalSizing, ModalWindowConfig, ModalWindowOutcome, ModalWindowState, Shortcut,
    handle_modal_key, handle_modal_mouse, render_modal_window,
};

const SHORTCUT_ID_HINT: usize = usize::MAX;
const SHORTCUT_ID_CANCEL: usize = 0;

/// Which step of the wizard is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByopSetupStep {
    PickProvider,
    EnterApiKey,
    Submitting,
}

/// Outcome of an input event for the caller to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByopSetupOutcome {
    /// User cancelled (Esc / Cancel).
    Cancelled,
    /// User confirmed provider + API key — caller should dispatch AddProvider.
    Submit {
        name: String,
        api_key: String,
        base_url: Option<String>,
    },
    /// State changed; re-render.
    Changed,
    /// Key/mouse not consumed.
    Unchanged,
}

/// Welcome / slash-command provider setup wizard state.
pub struct ByopSetupModalState {
    pub step: ByopSetupStep,
    /// Index into [`builtin_provider_presets`].
    pub focus: usize,
    /// Top of visible scroll window for the provider list.
    pub scroll_offset: usize,
    pub api_key_input: String,
    pub error: Option<String>,
    pub content_area: Option<Rect>,
    pub window: ModalWindowState,
}

impl ByopSetupModalState {
    pub fn new() -> Self {
        Self {
            step: ByopSetupStep::PickProvider,
            focus: 0,
            scroll_offset: 0,
            api_key_input: String::new(),
            error: None,
            content_area: None,
            window: ModalWindowState::default(),
        }
    }

    fn selected_preset(&self) -> &'static BuiltinProviderPreset {
        let presets = builtin_provider_presets();
        &presets[self.focus.min(presets.len().saturating_sub(1))]
    }

    fn clamp_focus(&mut self) {
        let len = builtin_provider_presets().len();
        if len == 0 {
            self.focus = 0;
            return;
        }
        if self.focus >= len {
            self.focus = len - 1;
        }
    }

    fn ensure_focus_visible(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        if self.focus < self.scroll_offset {
            self.scroll_offset = self.focus;
        } else if self.focus >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.focus + 1 - visible_rows;
        }
    }
}

impl Default for ByopSetupModalState {
    fn default() -> Self {
        Self::new()
    }
}

fn try_submit(state: &mut ByopSetupModalState) -> ByopSetupOutcome {
    let preset = state.selected_preset();
    let trimmed = state.api_key_input.trim();
    if preset.requires_api_key && trimmed.is_empty() {
        state.error = Some("Paste an API key to continue.".into());
        return ByopSetupOutcome::Changed;
    }
    let api_key = if trimmed.is_empty() {
        // Ollama and similar local servers accept a placeholder.
        "ollama".to_owned()
    } else {
        trimmed.to_owned()
    };
    state.step = ByopSetupStep::Submitting;
    state.error = None;
    ByopSetupOutcome::Submit {
        name: preset.id.to_owned(),
        api_key,
        base_url: preset.base_url.map(str::to_owned),
    }
}

/// Handle keyboard input for the setup wizard.
pub fn handle_byop_setup_key(
    state: &mut ByopSetupModalState,
    key: &KeyEvent,
) -> ByopSetupOutcome {
    if state.step == ByopSetupStep::Submitting {
        return ByopSetupOutcome::Unchanged;
    }

    let shortcuts = [
        Shortcut {
            label: "↑↓ navigate",
            clickable: false,
            id: SHORTCUT_ID_HINT,
        },
        Shortcut {
            label: "enter confirm",
            clickable: false,
            id: SHORTCUT_ID_HINT,
        },
        Shortcut {
            label: "Esc cancel",
            clickable: true,
            id: SHORTCUT_ID_CANCEL,
        },
    ];
    let config = ModalWindowConfig {
        title: "Set up a provider",
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing::medium(),
        fold_info: None,
    };

    match handle_modal_key(&mut state.window, key, &config) {
        ModalWindowOutcome::CloseRequested => {
            return if state.step == ByopSetupStep::EnterApiKey {
                state.step = ByopSetupStep::PickProvider;
                state.error = None;
                ByopSetupOutcome::Changed
            } else {
                ByopSetupOutcome::Cancelled
            };
        }
        ModalWindowOutcome::ShortcutActivated(SHORTCUT_ID_CANCEL) => {
            return ByopSetupOutcome::Cancelled;
        }
        ModalWindowOutcome::Handled => return ByopSetupOutcome::Changed,
        ModalWindowOutcome::Unhandled
        | ModalWindowOutcome::TabChanged(_)
        | ModalWindowOutcome::ShortcutActivated(_)
        | ModalWindowOutcome::CollapseGroup
        | ModalWindowOutcome::ExpandGroup
        | ModalWindowOutcome::CollapseDetails
        | ModalWindowOutcome::ExpandDetails
        | ModalWindowOutcome::JumpToParent(_) => {}
    }

    match state.step {
        ByopSetupStep::PickProvider => match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if state.focus > 0 {
                    state.focus -= 1;
                }
                ByopSetupOutcome::Changed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = builtin_provider_presets().len().saturating_sub(1);
                if state.focus < max {
                    state.focus += 1;
                }
                ByopSetupOutcome::Changed
            }
            KeyCode::Enter => {
                state.step = ByopSetupStep::EnterApiKey;
                state.error = None;
                ByopSetupOutcome::Changed
            }
            _ => ByopSetupOutcome::Unchanged,
        },
        ByopSetupStep::EnterApiKey => {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
            {
                return ByopSetupOutcome::Cancelled;
            }
            match key.code {
                KeyCode::Enter => try_submit(state),
                KeyCode::Backspace => {
                    state.api_key_input.pop();
                    ByopSetupOutcome::Changed
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    state.api_key_input.push(c);
                    state.error = None;
                    ByopSetupOutcome::Changed
                }
                _ => ByopSetupOutcome::Unchanged,
            }
        }
        ByopSetupStep::Submitting => ByopSetupOutcome::Unchanged,
    }
}

/// Handle mouse input for the setup wizard.
pub fn handle_byop_setup_mouse(
    state: &mut ByopSetupModalState,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> ByopSetupOutcome {
    if state.step == ByopSetupStep::Submitting {
        return ByopSetupOutcome::Unchanged;
    }
    match handle_modal_mouse(&mut state.window, kind, column, row) {
        ModalWindowOutcome::CloseRequested
        | ModalWindowOutcome::ShortcutActivated(SHORTCUT_ID_CANCEL) => {
            return ByopSetupOutcome::Cancelled;
        }
        ModalWindowOutcome::Handled => return ByopSetupOutcome::Changed,
        _ => {}
    }

    if state.step != ByopSetupStep::PickProvider {
        return ByopSetupOutcome::Unchanged;
    }
    let Some(area) = state.content_area else {
        return ByopSetupOutcome::Unchanged;
    };
    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if column >= area.x
                && column < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
            {
                let row_idx = (row - area.y) as usize + state.scroll_offset;
                if row_idx < builtin_provider_presets().len() {
                    state.focus = row_idx;
                    state.step = ByopSetupStep::EnterApiKey;
                    state.error = None;
                    return ByopSetupOutcome::Changed;
                }
            }
            ByopSetupOutcome::Unchanged
        }
        MouseEventKind::ScrollUp => {
            if state.focus > 0 {
                state.focus -= 1;
            }
            ByopSetupOutcome::Changed
        }
        MouseEventKind::ScrollDown => {
            let max = builtin_provider_presets().len().saturating_sub(1);
            if state.focus < max {
                state.focus += 1;
            }
            ByopSetupOutcome::Changed
        }
        _ => ByopSetupOutcome::Unchanged,
    }
}

/// Handle paste into the API-key step.
pub fn handle_byop_setup_paste(state: &mut ByopSetupModalState, text: &str) -> ByopSetupOutcome {
    if state.step != ByopSetupStep::EnterApiKey {
        return ByopSetupOutcome::Unchanged;
    }
    let cleaned: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    if cleaned.is_empty() {
        return ByopSetupOutcome::Unchanged;
    }
    state.api_key_input.push_str(&cleaned);
    state.error = None;
    ByopSetupOutcome::Changed
}

/// Render the setup wizard into `area`.
pub fn render_byop_setup_modal(
    buf: &mut Buffer,
    area: Rect,
    state: &mut ByopSetupModalState,
    theme: &Theme,
    compact: bool,
) {
    let title = match state.step {
        ByopSetupStep::PickProvider => "Set up a provider",
        ByopSetupStep::EnterApiKey => "Enter API key",
        ByopSetupStep::Submitting => "Saving provider…",
    };
    let shortcuts = match state.step {
        ByopSetupStep::PickProvider => [
            Shortcut {
                label: "↑↓ navigate",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
            Shortcut {
                label: "enter select",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
            Shortcut {
                label: "Esc cancel",
                clickable: true,
                id: SHORTCUT_ID_CANCEL,
            },
        ],
        ByopSetupStep::EnterApiKey => [
            Shortcut {
                label: "enter save",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
            Shortcut {
                label: "Esc back",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
            Shortcut {
                label: "ctrl-c cancel",
                clickable: true,
                id: SHORTCUT_ID_CANCEL,
            },
        ],
        ByopSetupStep::Submitting => [
            Shortcut {
                label: "saving…",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
            Shortcut {
                label: "",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
            Shortcut {
                label: "",
                clickable: false,
                id: SHORTCUT_ID_HINT,
            },
        ],
    };
    // Filter empty labels for the submitting step.
    let shortcuts: Vec<Shortcut<'_>> = shortcuts
        .into_iter()
        .filter(|s| !s.label.is_empty())
        .collect();

    let config = ModalWindowConfig {
        title,
        tabs: None,
        shortcuts: &shortcuts,
        sizing: ModalSizing::medium().with_compact(compact),
        fold_info: None,
    };

    let Some(areas) = render_modal_window(buf, area, &mut state.window, &config, theme) else {
        return;
    };
    let content_area = areas.content;
    state.content_area = Some(content_area);

    match state.step {
        ByopSetupStep::PickProvider => {
            let visible = content_area.height as usize;
            state.clamp_focus();
            state.ensure_focus_visible(visible);
            render_provider_list(content_area, buf, state, theme);
        }
        ByopSetupStep::EnterApiKey => render_api_key_step(content_area, buf, state, theme),
        ByopSetupStep::Submitting => {
            let line = Line::from(Span::styled(
                "Saving credentials and refreshing models…",
                Style::default().fg(theme.gray_bright),
            ));
            Paragraph::new(line).render(content_area, buf);
        }
    }
}

fn render_provider_list(
    area: Rect,
    buf: &mut Buffer,
    state: &ByopSetupModalState,
    theme: &Theme,
) {
    let presets = builtin_provider_presets();
    let visible = area.height as usize;
    let start = state
        .scroll_offset
        .min(presets.len().saturating_sub(visible.max(1)));
    for (row_i, preset) in presets.iter().skip(start).take(visible).enumerate() {
        let y = area.y + row_i as u16;
        if y >= area.y + area.height {
            break;
        }
        let idx = start + row_i;
        let selected = idx == state.focus;
        let style = if selected {
            Style::default()
                .fg(theme.accent_assistant)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_primary)
        };
        let marker = if selected { "› " } else { "  " };
        let label = format!(
            "{marker}{name}  ({id})",
            name = preset.display_name,
            id = preset.id
        );
        let line = Line::from(Span::styled(label, style));
        buf.set_line(area.x, y, &line, area.width);
    }
}

fn render_api_key_step(
    area: Rect,
    buf: &mut Buffer,
    state: &ByopSetupModalState,
    theme: &Theme,
) {
    let preset = state.selected_preset();
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Provider: {} ({})", preset.display_name, preset.id),
        Style::default().fg(theme.text_primary),
    )));
    if let Some(url) = preset.base_url {
        lines.push(Line::from(Span::styled(
            format!("Endpoint: {url}"),
            Style::default().fg(theme.gray_bright),
        )));
    }
    lines.push(Line::from(""));
    let hint = if preset.requires_api_key {
        "Paste your API key, then press Enter."
    } else {
        "API key optional for local Ollama — press Enter to continue."
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(theme.gray_bright),
    )));
    let masked: String = if state.api_key_input.is_empty() {
        String::new()
    } else {
        "•".repeat(state.api_key_input.chars().count().min(64))
    };
    lines.push(Line::from(vec![
        Span::styled("Key: ", Style::default().fg(theme.text_primary)),
        Span::styled(masked, Style::default().fg(theme.accent_assistant)),
    ]));
    if let Some(err) = &state.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(theme.accent_error),
        )));
    }
    Paragraph::new(lines).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_on_provider_advances_to_key_step() {
        let mut state = ByopSetupModalState::new();
        state.focus = 1;
        let outcome = handle_byop_setup_key(
            &mut state,
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(outcome, ByopSetupOutcome::Changed);
        assert_eq!(state.step, ByopSetupStep::EnterApiKey);
    }

    #[test]
    fn submit_requires_key_for_openai() {
        let mut state = ByopSetupModalState::new();
        state.focus = 1; // openai
        state.step = ByopSetupStep::EnterApiKey;
        let outcome = handle_byop_setup_key(
            &mut state,
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(outcome, ByopSetupOutcome::Changed);
        assert!(state.error.is_some());
        assert_eq!(state.step, ByopSetupStep::EnterApiKey);
    }

    #[test]
    fn submit_with_key_emits_action() {
        let mut state = ByopSetupModalState::new();
        state.focus = 1; // openai
        state.step = ByopSetupStep::EnterApiKey;
        state.api_key_input = "sk-test".into();
        let outcome = handle_byop_setup_key(
            &mut state,
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        match outcome {
            ByopSetupOutcome::Submit {
                name,
                api_key,
                base_url,
            } => {
                assert_eq!(name, "openai");
                assert_eq!(api_key, "sk-test");
                assert_eq!(base_url.as_deref(), Some("https://api.openai.com/v1"));
            }
            other => panic!("expected Submit, got {other:?}"),
        }
        assert_eq!(state.step, ByopSetupStep::Submitting);
    }

    #[test]
    fn ollama_allows_empty_key() {
        let mut state = ByopSetupModalState::new();
        let ollama_idx = builtin_provider_presets()
            .iter()
            .position(|p| p.id == "ollama")
            .expect("ollama preset");
        state.focus = ollama_idx;
        state.step = ByopSetupStep::EnterApiKey;
        let outcome = handle_byop_setup_key(
            &mut state,
            &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        match outcome {
            ByopSetupOutcome::Submit { name, api_key, .. } => {
                assert_eq!(name, "ollama");
                assert_eq!(api_key, "ollama");
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }
}
