//! `git_*` tools — structured git operations (status, diff, log, commit) via
//! the system `git` binary.
//!
//! These give the model the same local-workflow git operations it could
//! already run through `run_terminal_command` (`git status`/`diff`/`log`/
//! `add`/`commit` are all in the terminal tool's routine-command allowlist —
//! see `xai-grok-workspace::permission::auto_mode::ROUTINE_PREFIXES`), but as
//! typed inputs/outputs instead of a raw shell string the model has to quote
//! correctly and a text blob it has to re-parse. Each call shells out fresh
//! via `tokio::process::Command` (`Command::args`, never a shell — no
//! quoting/injection surface); there is no persistent session state to hold,
//! unlike [`super::browser`]'s long-lived tab.
//!
//! Permission-wise these map to `AccessKind::Bash` (see
//! `xai-grok-workspace::permission::types`) with a reconstructed command
//! string, so they inherit the exact same auto-mode classification and
//! prompt copy that shelling out to the same git commands already gets —
//! no new permission category needed for operations this codebase already
//! treats as routine local-dev commands.

use std::path::Path;
use std::process::Output;

use crate::types::output::ToolOutput;
use crate::types::requirements::{Expr, ToolRequirement};
use crate::types::tool::{ToolKind, ToolNamespace};
use crate::types::tool_metadata::{ToolMetadata, resolve_cwd, shared_resources};

/// Same order of magnitude as `browser_get_text`'s truncation and
/// `web_fetch`'s prose limit -- long enough to be useful, short enough not
/// to blow the context on a large diff or log.
const MAX_OUTPUT_CHARS: usize = 20_000;

/// git_log's `limit` is model-supplied; cap it so a huge request can't dump
/// the entire history into context.
const MAX_LOG_LIMIT: u32 = 200;

fn truncate(mut s: String) -> String {
    if s.chars().count() > MAX_OUTPUT_CHARS {
        s = s.chars().take(MAX_OUTPUT_CHARS).collect();
        s.push_str("\n... [truncated]");
    }
    s
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<Output, xai_tool_runtime::ToolError> {
    tokio::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .await
        .map_err(|e| {
            xai_tool_runtime::ToolError::new(
                xai_tool_runtime::ToolErrorKind::Custom,
                format!("Could not run git — is it installed? ({e})"),
            )
        })
}

fn output_text(output: Output) -> Result<String, xai_tool_runtime::ToolError> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(xai_tool_runtime::ToolError::new(
            xai_tool_runtime::ToolErrorKind::Custom,
            format!("git failed: {}", stderr.trim()),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ── git_status ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GitStatusInput {
    #[serde(default)]
    #[schemars(description = "Optional paths to scope the status to. Omit for the whole repo.")]
    pub paths: Vec<String>,
}

#[derive(Debug, Default)]
pub struct GitStatusTool;

impl ToolMetadata for GitStatusTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Show the working tree status: current branch, ahead/behind counts, and changed/staged/\
         untracked files."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GitStatusTool {
    type Args = GitStatusInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("git_status").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("git_status", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.git_status", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GitStatusInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;
        let mut args = vec!["status", "--short", "--branch"];
        if !input.paths.is_empty() {
            args.push("--");
            args.extend(input.paths.iter().map(String::as_str));
        }
        let output = run_git(&cwd, &args).await?;
        let text = output_text(output)?;
        let text = if text.trim().lines().count() <= 1 {
            format!("{}\n(clean — no changes)", text.trim_end())
        } else {
            text
        };
        Ok(ToolOutput::Text(truncate(text).into()))
    }
}

// ── git_diff ──────────────────────────────────────────────────────────────

fn default_context_lines() -> u32 {
    3
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GitDiffInput {
    #[serde(default)]
    #[schemars(
        description = "Diff staged changes (git diff --staged) instead of the working tree."
    )]
    pub staged: bool,
    #[serde(default)]
    #[schemars(description = "Optional paths to scope the diff to. Omit for the whole repo.")]
    pub paths: Vec<String>,
    #[serde(default = "default_context_lines")]
    #[schemars(description = "Lines of context around each change. Defaults to 3.")]
    pub context_lines: u32,
}

impl Default for GitDiffInput {
    fn default() -> Self {
        Self {
            staged: false,
            paths: Vec::new(),
            context_lines: default_context_lines(),
        }
    }
}

#[derive(Debug, Default)]
pub struct GitDiffTool;

impl ToolMetadata for GitDiffTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Show a diff of changes: unstaged working-tree changes by default, or staged changes \
         with staged=true. Long diffs are truncated."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GitDiffTool {
    type Args = GitDiffInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("git_diff").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("git_diff", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.git_diff", skip_all, fields(staged = input.staged))]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GitDiffInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;
        let context_flag = format!("-U{}", input.context_lines.min(200));
        let mut args = vec!["diff"];
        if input.staged {
            args.push("--staged");
        }
        args.push(&context_flag);
        if !input.paths.is_empty() {
            args.push("--");
            args.extend(input.paths.iter().map(String::as_str));
        }
        let output = run_git(&cwd, &args).await?;
        let text = output_text(output)?;
        let text = if text.trim().is_empty() {
            "(no differences)".to_owned()
        } else {
            text
        };
        Ok(ToolOutput::Text(truncate(text).into()))
    }
}

// ── git_log ───────────────────────────────────────────────────────────────

fn default_log_limit() -> u32 {
    20
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GitLogInput {
    #[serde(default = "default_log_limit")]
    #[schemars(description = "Maximum number of commits to show. Defaults to 20, capped at 200.")]
    pub limit: u32,
    #[serde(default)]
    #[schemars(description = "Optional paths to scope the log to. Omit for the whole repo.")]
    pub paths: Vec<String>,
}

impl Default for GitLogInput {
    fn default() -> Self {
        Self {
            limit: default_log_limit(),
            paths: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct GitLogTool;

impl ToolMetadata for GitLogTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Show recent commit history (one line per commit: short hash + subject)."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GitLogTool {
    type Args = GitLogInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("git_log").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("git_log", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.git_log", skip_all, fields(limit = input.limit))]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GitLogInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;
        let limit = input.limit.clamp(1, MAX_LOG_LIMIT);
        let n_flag = format!("-{limit}");
        let mut args = vec!["log", "--oneline", &n_flag];
        if !input.paths.is_empty() {
            args.push("--");
            args.extend(input.paths.iter().map(String::as_str));
        }
        let output = run_git(&cwd, &args).await?;
        let text = output_text(output)?;
        let text = if text.trim().is_empty() {
            "(no commits)".to_owned()
        } else {
            text
        };
        Ok(ToolOutput::Text(truncate(text).into()))
    }
}

// ── git_commit ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GitCommitInput {
    #[schemars(description = "The commit message.")]
    pub message: String,
    #[serde(default)]
    #[schemars(
        description = "Specific paths to stage before committing. Omit (or leave empty) to \
         stage all tracked changes (git add -A)."
    )]
    pub paths: Vec<String>,
}

#[derive(Debug, Default)]
pub struct GitCommitTool;

impl ToolMetadata for GitCommitTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Git
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Stage changes and create a commit. Stages the given paths, or every tracked change if \
         none are given, then commits with the provided message."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for GitCommitTool {
    type Args = GitCommitInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("git_commit").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("git_commit", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.git_commit", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: GitCommitInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        if input.message.trim().is_empty() {
            return Err(xai_tool_runtime::ToolError::invalid_arguments(
                "Commit message cannot be empty.",
            ));
        }
        let resources = shared_resources(&ctx)?;
        let cwd = resolve_cwd(&ctx, &resources).await?;

        let mut add_args = vec!["add"];
        if input.paths.is_empty() {
            add_args.push("-A");
        } else {
            add_args.extend(input.paths.iter().map(String::as_str));
        }
        output_text(run_git(&cwd, &add_args).await?)?;

        let commit_output = run_git(&cwd, &["commit", "-m", &input.message]).await?;
        let text = output_text(commit_output)?;
        Ok(ToolOutput::Text(truncate(text).into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids() {
        assert_eq!(
            xai_tool_runtime::Tool::id(&GitStatusTool).as_str(),
            "git_status"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&GitDiffTool).as_str(),
            "git_diff"
        );
        assert_eq!(xai_tool_runtime::Tool::id(&GitLogTool).as_str(), "git_log");
        assert_eq!(
            xai_tool_runtime::Tool::id(&GitCommitTool).as_str(),
            "git_commit"
        );
    }

    #[test]
    fn diff_input_defaults() {
        let input: GitDiffInput = serde_json::from_str("{}").unwrap();
        assert!(!input.staged);
        assert!(input.paths.is_empty());
        assert_eq!(input.context_lines, 3);
    }

    #[test]
    fn log_input_defaults_to_20() {
        let input: GitLogInput = serde_json::from_str("{}").unwrap();
        assert_eq!(input.limit, 20);
    }

    #[test]
    fn commit_input_requires_message_field() {
        let err = serde_json::from_str::<GitCommitInput>("{}").unwrap_err();
        assert!(err.to_string().contains("message"));
    }

    #[tokio::test]
    async fn status_on_clean_repo_reports_clean() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        run_git(cwd, &["init", "-q"]).await.unwrap();
        run_git(cwd, &["config", "user.email", "test@example.com"])
            .await
            .unwrap();
        run_git(cwd, &["config", "user.name", "Test"])
            .await
            .unwrap();
        // An empty repo has no branch/commit yet; just check the command runs
        // and reports no changes rather than erroring.
        let output = run_git(cwd, &["status", "--short", "--branch"])
            .await
            .unwrap();
        let text = output_text(output).unwrap();
        assert!(
            !text.contains("M "),
            "expected no modified files, got: {text}"
        );
    }

    #[tokio::test]
    async fn commit_then_log_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        run_git(cwd, &["init", "-q"]).await.unwrap();
        run_git(cwd, &["config", "user.email", "test@example.com"])
            .await
            .unwrap();
        run_git(cwd, &["config", "user.name", "Test"])
            .await
            .unwrap();
        std::fs::write(cwd.join("file.txt"), "hello").unwrap();
        output_text(run_git(cwd, &["add", "-A"]).await.unwrap()).unwrap();
        output_text(
            run_git(cwd, &["commit", "-m", "initial commit"])
                .await
                .unwrap(),
        )
        .unwrap();
        let log = output_text(run_git(cwd, &["log", "--oneline", "-1"]).await.unwrap()).unwrap();
        assert!(log.contains("initial commit"), "got: {log}");
    }
}
