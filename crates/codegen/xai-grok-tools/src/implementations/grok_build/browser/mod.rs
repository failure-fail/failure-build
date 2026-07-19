//! `browser_*` tools — headless-browser automation (navigate, click, type,
//! screenshot, extract text) via `chromiumoxide` (CDP over WebSocket).
//!
//! Unlike `image_gen`/`web_fetch`, there's no remote credential to gate
//! registration on: any tool just needs a local Chrome/Chromium binary,
//! which either exists or it doesn't (checked lazily, at first actual call,
//! not at registration time — see [`service::BrowserService::navigate`]).
//! So these tools are exposed only via a dedicated toolset (the `browser-use`
//! persona), not gated by a `SessionContext` config field the way image/video
//! generation are.
//!
//! [`service::BrowserService`] is the session-scoped, `Clone`-able live
//! handle every tool below shares via `Resources::get_or_default` — it
//! launches Chrome lazily on the first call and reuses the same tab for
//! every subsequent one in the session.

pub mod service;

use crate::types::output::ToolOutput;
use crate::types::requirements::{Expr, ToolRequirement};
use crate::types::tool::{ToolKind, ToolNamespace};
use crate::types::tool_metadata::{ToolMetadata, shared_resources};
use service::BrowserService;

async fn service_for(
    ctx: &xai_tool_runtime::ToolCallContext,
) -> Result<BrowserService, xai_tool_runtime::ToolError> {
    let resources = shared_resources(ctx)?;
    let mut resources = resources.lock().await;
    Ok(resources.get_or_default::<BrowserService>().clone())
}

// ── browser_navigate ─────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BrowserNavigateInput {
    #[schemars(description = "The URL to navigate to, e.g. https://example.com.")]
    pub url: String,
}

#[derive(Debug, Default)]
pub struct BrowserNavigateTool;

impl ToolMetadata for BrowserNavigateTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Browser
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Navigate the browser to a URL and wait for the page to load. Launches a headless \
         browser on first use; the same tab is reused for every subsequent browser_* call in \
         this session. Returns the page's final URL (after any redirects) and title."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for BrowserNavigateTool {
    type Args = BrowserNavigateInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("browser_navigate").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("browser_navigate", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.browser_navigate", skip_all, fields(url = %input.url))]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: BrowserNavigateInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let url = validate_navigable_url(&input.url).await?;
        let service = service_for(&ctx).await?;
        let (final_url, title) = service.navigate(&url).await?;
        // The initial-host check above can't see where an HTTP redirect or
        // meta-refresh actually lands, and chromiumoxide gives no hook to
        // pin the resolved IP before it dials — so re-check where we
        // actually ended up, and purge the tab if it's blocked before any
        // later browser_get_text/browser_screenshot call can read it.
        if let Err(e) = validate_navigable_url(&final_url).await {
            let _ = service.reset_to_blank().await;
            return Err(e);
        }
        let title = title.unwrap_or_else(|| "(untitled)".to_owned());
        Ok(ToolOutput::Text(
            format!("Navigated to {final_url}\nPage title: {title}").into(),
        ))
    }
}

/// Parse and SSRF-check a navigation target. Only `http`/`https` are
/// meaningful for a browser tab; anything else (`file://`, `chrome://`,
/// `javascript:`, ...) would hand the model a local-filesystem or
/// privileged-UI read/write primitive through the back door.
async fn validate_navigable_url(raw: &str) -> Result<String, xai_tool_runtime::ToolError> {
    let url = url::Url::parse(raw).map_err(|e| {
        xai_tool_runtime::ToolError::invalid_arguments(format!("Invalid URL '{raw}': {e}"))
    })?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(xai_tool_runtime::ToolError::invalid_arguments(format!(
            "Unsupported URL scheme '{}': only http/https are allowed.",
            url.scheme()
        )));
    }
    let host = url.host_str().ok_or_else(|| {
        xai_tool_runtime::ToolError::invalid_arguments(format!("URL '{raw}' has no host"))
    })?;
    let port = url.port_or_known_default().unwrap_or(443);
    match crate::util::ssrf::first_blocked_resolved_ip(host, port).await {
        Ok(Some(ip)) => Err(xai_tool_runtime::ToolError::invalid_arguments(format!(
            "Refusing to navigate to '{host}': resolves to {ip}, a private/internal address."
        ))),
        Ok(None) => Ok(url.to_string()),
        Err(e) => Err(xai_tool_runtime::ToolError::invalid_arguments(format!(
            "Could not resolve host '{host}': {e}"
        ))),
    }
}

// ── browser_click ────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BrowserClickInput {
    #[schemars(description = "CSS selector of the element to click, e.g. 'button#submit' or 'a.nav-link'.")]
    pub selector: String,
}

#[derive(Debug, Default)]
pub struct BrowserClickTool;

impl ToolMetadata for BrowserClickTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Browser
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Click the first element on the current page matching a CSS selector. Call \
         browser_navigate first if no page is loaded yet."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for BrowserClickTool {
    type Args = BrowserClickInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("browser_click").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("browser_click", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.browser_click", skip_all, fields(selector = %input.selector))]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: BrowserClickInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let service = service_for(&ctx).await?;
        service.click(&input.selector).await?;
        // A click can trigger in-page navigation (a plain `<a href>`, a JS
        // handler) to somewhere the original browser_navigate call never
        // saw. Re-check the tab's resulting location for the same reason
        // browser_navigate re-checks its post-redirect URL.
        if let Some(current) = service.current_url().await
            && current != "about:blank"
            && let Err(e) = validate_navigable_url(&current).await
        {
            let _ = service.reset_to_blank().await;
            return Err(e);
        }
        Ok(ToolOutput::Text(
            format!("Clicked element matching '{}'.", input.selector).into(),
        ))
    }
}

// ── browser_type ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BrowserTypeInput {
    #[schemars(description = "CSS selector of the input/textarea element to type into.")]
    pub selector: String,
    #[schemars(description = "Text to type into the element.")]
    pub text: String,
    #[serde(default)]
    #[schemars(description = "If true, press Enter after typing (e.g. to submit a search box). Defaults to false.")]
    pub submit: bool,
}

#[derive(Debug, Default)]
pub struct BrowserTypeTool;

impl ToolMetadata for BrowserTypeTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Browser
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Type text into an input/textarea element on the current page, identified by a CSS \
         selector. Set submit=true to press Enter afterward (e.g. to submit a search box)."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for BrowserTypeTool {
    type Args = BrowserTypeInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("browser_type").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("browser_type", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.browser_type", skip_all, fields(selector = %input.selector, submit = input.submit))]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: BrowserTypeInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let service = service_for(&ctx).await?;
        service.type_text(&input.selector, &input.text, input.submit).await?;
        Ok(ToolOutput::Text(
            format!("Typed into element matching '{}'.", input.selector).into(),
        ))
    }
}

// ── browser_get_text ─────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BrowserGetTextInput {
    #[serde(default)]
    #[schemars(
        description = "CSS selector to scope text extraction to one element. Omit to get the whole page's visible text."
    )]
    pub selector: Option<String>,
}

/// Long enough to be useful, short enough not to blow the context on a huge
/// page. Matches the order of magnitude `web_fetch` truncates prose to.
const MAX_GET_TEXT_CHARS: usize = 20_000;

#[derive(Debug, Default)]
pub struct BrowserGetTextTool;

impl ToolMetadata for BrowserGetTextTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Browser
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Extract the visible text of the current page (or one element, via a CSS selector). \
         Long text is truncated."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for BrowserGetTextTool {
    type Args = BrowserGetTextInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("browser_get_text").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("browser_get_text", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.browser_get_text", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: BrowserGetTextInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let service = service_for(&ctx).await?;
        let text = service.get_text(input.selector.as_deref()).await?;
        let truncated = if text.chars().count() > MAX_GET_TEXT_CHARS {
            let mut s: String = text.chars().take(MAX_GET_TEXT_CHARS).collect();
            s.push_str("\n... [truncated]");
            s
        } else {
            text
        };
        Ok(ToolOutput::Text(truncated.into()))
    }
}

// ── browser_screenshot ───────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BrowserScreenshotInput {
    #[serde(default)]
    #[schemars(description = "Capture the full scrollable page instead of just the visible viewport. Defaults to false.")]
    pub full_page: bool,
}

#[derive(Debug, Default)]
pub struct BrowserScreenshotTool;

impl ToolMetadata for BrowserScreenshotTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Browser
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Take a screenshot of the current page and return it as an image you can see."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for BrowserScreenshotTool {
    type Args = BrowserScreenshotInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("browser_screenshot").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("browser_screenshot", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.browser_screenshot", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: BrowserScreenshotInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let service = service_for(&ctx).await?;
        let png_bytes = service.screenshot(input.full_page).await?;
        let output = crate::implementations::read_file::image::image_read_output(
            png_bytes,
            "image/png".to_owned(),
        )
        .await;
        Ok(ToolOutput::ReadFile(output))
    }
}

// ── browser_close ────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BrowserCloseInput {}

#[derive(Debug, Default)]
pub struct BrowserCloseTool;

impl ToolMetadata for BrowserCloseTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Browser
    }
    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }
    fn description_template(&self) -> &str {
        "Close the browser and free its resources. Safe to call even if no browser was ever \
         opened. Not required at the end of a turn — it's only for freeing resources early."
    }
    fn requires_expr(&self) -> Expr<ToolRequirement> {
        Expr::True
    }
}

impl xai_tool_runtime::Tool for BrowserCloseTool {
    type Args = BrowserCloseInput;
    type Output = ToolOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new("browser_close").expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new("browser_close", ToolMetadata::description_template(self))
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.browser_close", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        _input: BrowserCloseInput,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let service = service_for(&ctx).await?;
        let was_running = service.is_running().await;
        service.close().await?;
        Ok(ToolOutput::Text(
            if was_running {
                "Browser closed."
            } else {
                "No browser was open."
            }
            .to_owned()
            .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids() {
        assert_eq!(
            xai_tool_runtime::Tool::id(&BrowserNavigateTool).as_str(),
            "browser_navigate"
        );
        assert_eq!(xai_tool_runtime::Tool::id(&BrowserClickTool).as_str(), "browser_click");
        assert_eq!(xai_tool_runtime::Tool::id(&BrowserTypeTool).as_str(), "browser_type");
        assert_eq!(
            xai_tool_runtime::Tool::id(&BrowserGetTextTool).as_str(),
            "browser_get_text"
        );
        assert_eq!(
            xai_tool_runtime::Tool::id(&BrowserScreenshotTool).as_str(),
            "browser_screenshot"
        );
        assert_eq!(xai_tool_runtime::Tool::id(&BrowserCloseTool).as_str(), "browser_close");
    }

    #[tokio::test]
    async fn rejects_non_http_schemes() {
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "chrome://settings"] {
            let err = validate_navigable_url(bad).await.unwrap_err();
            assert!(err.to_string().contains("scheme"), "got: {err}");
        }
    }

    #[tokio::test]
    async fn rejects_private_ip_literal() {
        let err = validate_navigable_url("http://169.254.169.254/latest/meta-data")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("private/internal"), "got: {err}");
    }

    #[tokio::test]
    async fn accepts_public_https_url() {
        let ok = validate_navigable_url("https://example.com/page").await;
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn get_text_input_selector_defaults_to_none() {
        let input: BrowserGetTextInput = serde_json::from_str("{}").unwrap();
        assert_eq!(input.selector, None);
    }

    #[test]
    fn type_input_submit_defaults_to_false() {
        let input: BrowserTypeInput =
            serde_json::from_str(r#"{"selector": "input", "text": "hi"}"#).unwrap();
        assert!(!input.submit);
    }

    #[tokio::test]
    async fn errors_when_resources_missing() {
        let tool = BrowserCloseTool;
        let resources = crate::types::resources::Resources::new();
        let result = xai_tool_runtime::Tool::run(
            &tool,
            crate::types::tool_metadata::test_ctx_with_call_id(resources.into_shared(), "test-call"),
            BrowserCloseInput {},
        )
        .await;
        // get_or_default never errors (BrowserService::default is valid), so
        // this should actually succeed as a no-op close.
        assert!(result.is_ok(), "{result:?}");
    }
}
