//! `gh_pr_*` tools — structured GitHub pull-request operations via the
//! system `gh` CLI.
//!
//! These give the model the same forge workflow it could already run through
//! `run_terminal_command` (`gh pr view`/`list` are read-only; `gh pr create`/
//! `comment` prompt), but as typed inputs/outputs instead of a raw shell
//! string. Each call shells out fresh via `tokio::process::Command`
//! (`Command::args`, never a shell — no quoting/injection surface).
//!
//! Permission-wise these map to `AccessKind::Bash` (see
//! `xai-grok-workspace::permission::types`) with a reconstructed command
//! string, so they inherit the exact same auto-mode classification that
//! shelling out to the same `gh` commands already gets.

use std::path::Path;
use std::process::Output;

use crate::types::output::ToolOutput;
use crate::types::requirements::{Expr, ToolRequirement};
use crate::types::tool::{ToolKind, ToolNamespace};
use crate::types::tool_metadata::{ToolMetadata, resolve_cwd, shared_resources};
use crate::util::{detach_command, pager_env};

/// Same order of magnitude as `git_*` / `web_fetch` truncation.
const MAX_OUTPUT_CHARS: usize = 20_000;

/// `gh_pr_list`'s `limit` is model-supplied; cap it so a huge request can't
/// dump the entire PR history into context.
const MAX_LIST_LIMIT: u32 = 100;

/// JSON fields for `gh pr view` — enough for the model to reason about a PR
/// without dumping the full review timeline.
const VIEW_JSON_FIELDS: &str =
    "number,title,state,url,headRefName,baseRefName,isDraft,body,author,mergeable";

/// JSON fields for `gh pr list`.
const LIST_JSON_FIELDS: &str =
    "number,title,state,url,headRefName,baseRefName,isDraft,author";

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_OUTPUT_CHARS {
        s = s.chars().take(MAX_OUTPUT_CHARS).collect();
        s.push_str("\n... [truncated]");
    }
    s
}

fn default_list_limit() -> u32 {
    20
}

async fn run_gh(cwd: &Path, args: &[&str]) -> Result<Output, xai_tool_runtime::ToolError> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.current_dir(cwd).args(args).stdin(std::process::Stdio::null());
    detach_command(&mut cmd);
    cmd.envs(pager_env());
    cmd.output().await.map_err(|e| {
        xai_tool_runtime::ToolError::new(
            xai_tool_runtime::ToolErrorKind::Custom,
            format!("Could not run gh — is the GitHub CLI installed and authenticated? ({e})"),
        )
    })
}

fn output_text(output: Output) -> Result<String, xai_tool_runtime::ToolError> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_owned()
        } else {
            stdout.trim().to_owned()
        };
        return Err(xai_tool_runtime::ToolError::new(
            xai_tool_runtime::ToolErrorKind::Custom,
            format!("gh failed: {detail}"),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Build a reconstructed `gh …` string for permission prompts / Bash rules.
/// Values that may contain spaces are single-quoted (display only — the real
/// invocation uses `Command::args`).
pub(crate) fn reconstruct_gh_command(parts: &[(&str, Option<&str>)]) -> String {
    let mut out = String::from("gh");
    for (flag_or_arg, value) in parts {
        out.push(' ');
        out.push_str(flag_or_arg);
        if let Some(v) = value {
            out.push(' ');
            out.push_str(&shell_single_quote(v));
        }
    }
    out
}

fn shell_single_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_owned();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@'))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── gh_pr_view ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GhPrViewInput {
    #[serde(default)]
    #[schemars(
        description = "PR number, URL, or head branch. Omit to view the PR for the current branch."
    )]
    pub pr: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional owner/repo override (gh --repo).")]
    pub repo: Option<String>,
}

#[derive(Debug, Default)]
pub struct GhPrViewTool;

impl ToolMetadata for GhPrViewTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Show details for a GitHub pull request (number, title, state, URL, branches, body). \
         Omit pr to view the PR for the current branch."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GhPrViewTool {
    type Args = GhPrViewInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("gh_pr_view").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("gh_pr_view", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.gh_pr_view", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GhPrViewInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;

        let mut owned: Vec<String> = Vec::new();
        owned.push("pr".into());
        owned.push("view".into());
        if let Some(pr) = input.pr.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push(pr.to_owned());
        }
        owned.push("--json".into());
        owned.push(VIEW_JSON_FIELDS.into());
        if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--repo".into());
            owned.push(repo.to_owned());
        }

        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        let text = truncate(output_text(run_gh(&cwd, &args).await?)?);
        Ok(ToolOutput::Text(text.into()))
    }
}

// ── gh_pr_list ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GhPrListInput {
    #[serde(default)]
    #[schemars(description = "Filter by state: open, closed, merged, or all. Defaults to open.")]
    pub state: Option<String>,
    #[serde(default = "default_list_limit")]
    #[schemars(description = "Maximum number of PRs to list. Defaults to 20, capped at 100.")]
    pub limit: u32,
    #[serde(default)]
    #[schemars(description = "Filter by base branch.")]
    pub base: Option<String>,
    #[serde(default)]
    #[schemars(description = "Filter by head branch (optionally owner:branch).")]
    pub head: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional owner/repo override (gh --repo).")]
    pub repo: Option<String>,
}

impl Default for GhPrListInput {
    fn default() -> Self {
        Self {
            state: None,
            limit: default_list_limit(),
            base: None,
            head: None,
            repo: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct GhPrListTool;

impl ToolMetadata for GhPrListTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "List GitHub pull requests for the current repository (or --repo). Returns JSON."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GhPrListTool {
    type Args = GhPrListInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("gh_pr_list").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("gh_pr_list", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.gh_pr_list", skip_all, fields(limit = input.limit))]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GhPrListInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;

        let limit = input.limit.clamp(1, MAX_LIST_LIMIT).to_string();
        let mut owned: Vec<String> = vec![
            "pr".into(),
            "list".into(),
            "--limit".into(),
            limit,
            "--json".into(),
            LIST_JSON_FIELDS.into(),
        ];
        if let Some(state) = input.state.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--state".into());
            owned.push(state.to_owned());
        }
        if let Some(base) = input.base.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--base".into());
            owned.push(base.to_owned());
        }
        if let Some(head) = input.head.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--head".into());
            owned.push(head.to_owned());
        }
        if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--repo".into());
            owned.push(repo.to_owned());
        }

        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        let text = truncate(output_text(run_gh(&cwd, &args).await?)?);
        let text = if text.trim().is_empty() || text.trim() == "[]" {
            "[]".to_owned()
        } else {
            text
        };
        Ok(ToolOutput::Text(text.into()))
    }
}

// ── gh_pr_create ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GhPrCreateInput {
    #[schemars(description = "Pull request title.")]
    pub title: String,
    #[serde(default)]
    #[schemars(description = "Pull request body / description.")]
    pub body: Option<String>,
    #[serde(default)]
    #[schemars(description = "Base branch (defaults to the repo default branch).")]
    pub base: Option<String>,
    #[serde(default)]
    #[schemars(description = "Head branch (defaults to the current branch).")]
    pub head: Option<String>,
    #[serde(default)]
    #[schemars(description = "Create as a draft pull request.")]
    pub draft: bool,
    #[serde(default)]
    #[schemars(description = "Optional owner/repo override (gh --repo).")]
    pub repo: Option<String>,
}

#[derive(Debug, Default)]
pub struct GhPrCreateTool;

impl ToolMetadata for GhPrCreateTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Create a GitHub pull request for the current branch (or --head). Requires the GitHub CLI \
         (`gh`) to be authenticated. Prefer this over shelling out to `gh pr create`."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GhPrCreateTool {
    type Args = GhPrCreateInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("gh_pr_create").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            "gh_pr_create",
            ToolMetadata::description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.gh_pr_create", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GhPrCreateInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        if input.title.trim().is_empty() {
            return Err(xai_tool_runtime::ToolError::invalid_arguments(
                "Pull request title cannot be empty.",
            ));
        }
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;

        let mut owned: Vec<String> = vec![
            "pr".into(),
            "create".into(),
            "--title".into(),
            input.title.clone(),
        ];
        // Always pass --body so gh never opens an editor in the agent process.
        owned.push("--body".into());
        owned.push(input.body.clone().unwrap_or_default());
        if let Some(base) = input.base.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--base".into());
            owned.push(base.to_owned());
        }
        if let Some(head) = input.head.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--head".into());
            owned.push(head.to_owned());
        }
        if input.draft {
            owned.push("--draft".into());
        }
        if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--repo".into());
            owned.push(repo.to_owned());
        }

        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        let text = truncate(output_text(run_gh(&cwd, &args).await?)?);
        Ok(ToolOutput::Text(text.into()))
    }
}

// ── gh_pr_comment ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GhPrCommentInput {
    #[schemars(description = "PR number, URL, or head branch to comment on.")]
    pub pr: String,
    #[schemars(description = "Comment body.")]
    pub body: String,
    #[serde(default)]
    #[schemars(description = "Optional owner/repo override (gh --repo).")]
    pub repo: Option<String>,
}

#[derive(Debug, Default)]
pub struct GhPrCommentTool;

impl ToolMetadata for GhPrCommentTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Add a comment on a GitHub pull request. Prefer this over shelling out to `gh pr comment`."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GhPrCommentTool {
    type Args = GhPrCommentInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("gh_pr_comment").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            "gh_pr_comment",
            ToolMetadata::description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.gh_pr_comment", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GhPrCommentInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        if input.pr.trim().is_empty() {
            return Err(xai_tool_runtime::ToolError::invalid_arguments(
                "Pull request reference (pr) cannot be empty.",
            ));
        }
        if input.body.trim().is_empty() {
            return Err(xai_tool_runtime::ToolError::invalid_arguments(
                "Comment body cannot be empty.",
            ));
        }
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;

        let mut owned: Vec<String> = vec![
            "pr".into(),
            "comment".into(),
            input.pr.clone(),
            "--body".into(),
            input.body.clone(),
        ];
        if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            owned.push("--repo".into());
            owned.push(repo.to_owned());
        }

        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        let text = truncate(output_text(run_gh(&cwd, &args).await?)?);
        Ok(ToolOutput::Text(text.into()))
    }
}

/// Reconstruct permission-display commands for each typed PR tool input.
pub fn permission_command_for_view(input: &GhPrViewInput) -> String {
    let mut parts: Vec<(&str, Option<&str>)> = vec![("pr", None), ("view", None)];
    if let Some(pr) = input.pr.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push((pr, None));
    }
    parts.push(("--json", Some(VIEW_JSON_FIELDS)));
    if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(("--repo", Some(repo)));
    }
    reconstruct_gh_command(&parts)
}

pub fn permission_command_for_list(input: &GhPrListInput) -> String {
    let limit = input.limit.clamp(1, MAX_LIST_LIMIT).to_string();
    // Keep limit owned for the reconstructed string lifetime via format below.
    let mut cmd = format!("gh pr list --limit {limit} --json {LIST_JSON_FIELDS}");
    if let Some(state) = input.state.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --state ");
        cmd.push_str(&shell_single_quote(state));
    }
    if let Some(base) = input.base.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --base ");
        cmd.push_str(&shell_single_quote(base));
    }
    if let Some(head) = input.head.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --head ");
        cmd.push_str(&shell_single_quote(head));
    }
    if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --repo ");
        cmd.push_str(&shell_single_quote(repo));
    }
    cmd
}

pub fn permission_command_for_create(input: &GhPrCreateInput) -> String {
    let body = input.body.as_deref().unwrap_or("");
    let mut cmd = format!(
        "gh pr create --title {} --body {}",
        shell_single_quote(&input.title),
        shell_single_quote(body)
    );
    if let Some(base) = input.base.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --base ");
        cmd.push_str(&shell_single_quote(base));
    }
    if let Some(head) = input.head.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --head ");
        cmd.push_str(&shell_single_quote(head));
    }
    if input.draft {
        cmd.push_str(" --draft");
    }
    if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --repo ");
        cmd.push_str(&shell_single_quote(repo));
    }
    cmd
}

pub fn permission_command_for_comment(input: &GhPrCommentInput) -> String {
    let mut cmd = format!(
        "gh pr comment {} --body {}",
        shell_single_quote(&input.pr),
        shell_single_quote(&input.body)
    );
    if let Some(repo) = input.repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        cmd.push_str(" --repo ");
        cmd.push_str(&shell_single_quote(repo));
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids() {
        assert_eq!(
            xai_tool_runtime::Tool::id(&GhPrViewTool).as_str(),
            "gh_pr_view"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&GhPrListTool).as_str(),
            "gh_pr_list"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&GhPrCreateTool).as_str(),
            "gh_pr_create"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&GhPrCommentTool).as_str(),
            "gh_pr_comment"
        );
    }

    #[test]
    fn list_input_defaults_to_20() {
        let input: GhPrListInput = serde_json::from_str("{}").unwrap();
        assert_eq!(input.limit, 20);
        assert!(input.state.is_none());
    }

    #[test]
    fn create_input_requires_title() {
        let err = serde_json::from_str::<GhPrCreateInput>("{}").unwrap_err();
        assert!(err.to_string().contains("title"));
    }

    #[test]
    fn comment_input_requires_pr_and_body() {
        let err = serde_json::from_str::<GhPrCommentInput>(r#"{"pr":"1"}"#).unwrap_err();
        assert!(err.to_string().contains("body"));
        let err = serde_json::from_str::<GhPrCommentInput>(r#"{"body":"hi"}"#).unwrap_err();
        assert!(err.to_string().contains("pr"));
    }

    #[test]
    fn permission_commands_look_like_gh() {
        let view = permission_command_for_view(&GhPrViewInput {
            pr: Some("42".into()),
            repo: None,
        });
        assert!(view.starts_with("gh pr view 42 --json "));

        let list = permission_command_for_list(&GhPrListInput {
            state: Some("open".into()),
            limit: 5,
            ..Default::default()
        });
        assert_eq!(
            list,
            format!("gh pr list --limit 5 --json {LIST_JSON_FIELDS} --state open")
        );

        let create = permission_command_for_create(&GhPrCreateInput {
            title: "Add feature".into(),
            body: Some("details".into()),
            base: Some("main".into()),
            head: None,
            draft: true,
            repo: None,
        });
        assert!(create.contains("gh pr create --title "));
        assert!(create.contains("--draft"));
        assert!(create.contains("--base main"));

        let comment = permission_command_for_comment(&GhPrCommentInput {
            pr: "42".into(),
            body: "LGTM".into(),
            repo: Some("o/r".into()),
        });
        assert_eq!(comment, "gh pr comment 42 --body LGTM --repo o/r");
    }

    #[test]
    fn shell_single_quote_escapes_spaces_and_quotes() {
        assert_eq!(shell_single_quote("plain"), "plain");
        assert_eq!(shell_single_quote("has space"), "'has space'");
        assert_eq!(shell_single_quote("it's"), "'it'\\''s'");
    }
}
