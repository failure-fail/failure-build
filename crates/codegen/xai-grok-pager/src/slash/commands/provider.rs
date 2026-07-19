//! `/provider add <name> <api-key> [base-url]` — configure a custom
//! (BYOP) provider without hand-editing `config.toml`.
//!
//! Persists `[provider.<name>]`/`[model.<name>]` and stores the API key via
//! the same provider-scoped secret storage `failure login --provider` uses.
//! The existing config-file watcher picks up the change and refreshes the
//! model catalog automatically — no restart needed.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

/// Built-in provider presets with a known default base URL (see
/// `xai_grok_shell::agent::config::default_provider_entries`) — `base_url`
/// is optional for these.
const KNOWN_PRESETS: &[&str] = &["xai", "openai", "anthropic", "ollama"];

pub struct ProviderCommand;

impl SlashCommand for ProviderCommand {
    fn name(&self) -> &str {
        "provider"
    }

    fn description(&self) -> &str {
        "Configure a custom AI provider (OpenAI, Anthropic, Ollama, or any OpenAI-compatible endpoint)"
    }

    fn usage(&self) -> &str {
        "/provider add <name> <api-key> [base-url]"
    }

    fn takes_args(&self) -> bool {
        true
    }

    fn args_required(&self) -> bool {
        true
    }

    fn arg_placeholder(&self) -> Option<&str> {
        Some("add <name> <api-key> [base-url]")
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let mut tokens = args.split_whitespace();
        let Some(sub) = tokens.next() else {
            return CommandResult::Error(format!("Usage: {}", self.usage()));
        };
        if !sub.eq_ignore_ascii_case("add") {
            return CommandResult::Error(format!(
                "Unknown /provider subcommand '{sub}'. Usage: {}",
                self.usage()
            ));
        }

        let Some(name) = tokens.next() else {
            return CommandResult::Error(format!(
                "Missing provider name. Usage: {}",
                self.usage()
            ));
        };
        let Some(api_key) = tokens.next() else {
            return CommandResult::Error(format!(
                "Missing API key. Usage: {}",
                self.usage()
            ));
        };
        let base_url = tokens.next();

        if base_url.is_none() && !KNOWN_PRESETS.contains(&name.to_ascii_lowercase().as_str()) {
            return CommandResult::Error(format!(
                "'{name}' isn't a built-in provider ({}), so a base URL is required. \
                 Usage: {}",
                KNOWN_PRESETS.join("/"),
                self.usage()
            ));
        }

        CommandResult::Action(Action::AddProvider {
            name: name.to_owned(),
            api_key: api_key.to_owned(),
            base_url: base_url.map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::bundle::BundleState;
    use crate::app::ScreenMode;
    use crate::settings::PagerLocalSnapshot;

    static EMPTY_BUNDLE: BundleState = BundleState {
        has_cache: false,
        version: String::new(),
        personas: Vec::new(),
        roles: Vec::new(),
        agents: Vec::new(),
        skills: Vec::new(),
        persona_details: Vec::new(),
        role_details: Vec::new(),
    };

    fn dummy_ctx() -> CommandExecCtx<'static> {
        CommandExecCtx {
            models: Box::leak(Box::new(crate::acp::model_state::ModelState::default())),
            session_id: None,
            bundle_state: &EMPTY_BUNDLE,
            screen_mode: ScreenMode::Inline,
            pager_state: PagerLocalSnapshot {
                multiline_mode: false,
                yolo_mode: false,
                ..PagerLocalSnapshot::default()
            },
        }
    }

    #[test]
    fn missing_subcommand_errors() {
        let mut ctx = dummy_ctx();
        assert!(matches!(
            ProviderCommand.run(&mut ctx, ""),
            CommandResult::Error(_)
        ));
    }

    #[test]
    fn unknown_subcommand_errors() {
        let mut ctx = dummy_ctx();
        assert!(matches!(
            ProviderCommand.run(&mut ctx, "remove openai"),
            CommandResult::Error(_)
        ));
    }

    #[test]
    fn known_preset_without_base_url_is_ok() {
        let mut ctx = dummy_ctx();
        let result = ProviderCommand.run(&mut ctx, "add openai sk-test123");
        match result {
            CommandResult::Action(Action::AddProvider { name, api_key, base_url }) => {
                assert_eq!(name, "openai");
                assert_eq!(api_key, "sk-test123");
                assert_eq!(base_url, None);
            }
            other => panic!("expected AddProvider action, got {other:?}"),
        }
    }

    #[test]
    fn custom_name_without_base_url_errors() {
        let mut ctx = dummy_ctx();
        let result = ProviderCommand.run(&mut ctx, "add my-custom-thing sk-test123");
        assert!(matches!(result, CommandResult::Error(_)));
    }

    #[test]
    fn custom_name_with_base_url_is_ok() {
        let mut ctx = dummy_ctx();
        let result = ProviderCommand.run(
            &mut ctx,
            "add zandy-worker orch_test_abc https://ai-orchestrator-worker.example.dev/v1",
        );
        match result {
            CommandResult::Action(Action::AddProvider { name, api_key, base_url }) => {
                assert_eq!(name, "zandy-worker");
                assert_eq!(api_key, "orch_test_abc");
                assert_eq!(
                    base_url.as_deref(),
                    Some("https://ai-orchestrator-worker.example.dev/v1")
                );
            }
            other => panic!("expected AddProvider action, got {other:?}"),
        }
    }
}
