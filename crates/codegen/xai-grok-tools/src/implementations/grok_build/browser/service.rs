//! Session-scoped headless-browser handle (CDP over WebSocket via
//! `chromiumoxide`), shared by every `browser_*` tool.
//!
//! Lazily launches Chrome/Chromium on the first call any `browser_*` tool
//! makes — most sessions never touch these tools, so nothing is spawned at
//! startup. Single tab per session: simplest correct model, matching how one
//! `run_terminal_command` session works. `Clone`-able (an `Arc<Mutex<_>>`
//! handle) so the same live tab is reused across every subsequent call
//! within the session; `Default` gives the "not yet launched" empty state
//! that `Resources::get_or_default` inserts on first access.

use chromiumoxide::Browser;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::Mutex;
use xai_tool_runtime::{ToolError, ToolErrorKind};

fn launch_failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::new(
        ToolErrorKind::Custom,
        format!(
            "Could not launch a browser — is Google Chrome or Chromium installed? ({e})"
        ),
    )
}

fn cdp_failed(action: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::new(ToolErrorKind::Custom, format!("Browser {action} failed: {e}"))
}

#[derive(Default)]
struct BrowserState {
    browser: Option<Browser>,
    page: Option<Page>,
    // Keeps the CDP event-stream pump alive; never polled directly again
    // after spawn. Dropped (and the task aborted) when `close()` tears the
    // browser down.
    handler_task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for BrowserState {
    fn drop(&mut self) {
        if let Some(task) = self.handler_task.take() {
            task.abort();
        }
    }
}

#[derive(Clone, Default)]
pub struct BrowserService {
    state: Arc<Mutex<BrowserState>>,
}

impl BrowserService {
    /// Launch Chrome if not already running, open a tab if none is open yet,
    /// and return a cheap clone of the current page. Reused by every action.
    async fn ensure_page(&self) -> Result<Page, ToolError> {
        let mut state = self.state.lock().await;
        if let Some(page) = &state.page {
            return Ok(page.clone());
        }
        if state.browser.is_none() {
            let config = chromiumoxide::BrowserConfig::builder()
                // Most hosts running this CLI (containers, CI, root shells)
                // can't use Chrome's setuid sandbox; no user data of value
                // is at risk since this is an ephemeral automation profile,
                // not the user's personal browser.
                .no_sandbox()
                .build()
                .map_err(launch_failed)?;
            let (browser, mut handler) = Browser::launch(config).await.map_err(launch_failed)?;
            let handler_task = tokio::spawn(async move {
                while handler.next().await.is_some() {}
            });
            state.browser = Some(browser);
            state.handler_task = Some(handler_task);
        }
        let browser = state.browser.as_ref().expect("just set above");
        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| cdp_failed("tab open", e))?;
        state.page = Some(page.clone());
        Ok(page)
    }

    /// Navigate the current tab to `url`. Returns the page's final URL
    /// (post-redirect) and title, if available.
    pub async fn navigate(&self, url: &str) -> Result<(String, Option<String>), ToolError> {
        let page = self.ensure_page().await?;
        page.goto(url).await.map_err(|e| cdp_failed("navigation", e))?;
        page.wait_for_navigation()
            .await
            .map_err(|e| cdp_failed("navigation", e))?;
        let final_url = page
            .url()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| url.to_string());
        let title = page.get_title().await.ok().flatten();
        Ok((final_url, title))
    }

    /// Click the first element matching `selector` (CSS selector).
    pub async fn click(&self, selector: &str) -> Result<(), ToolError> {
        let page = self.ensure_page().await?;
        let element = page.find_element(selector).await.map_err(|e| {
            ToolError::new(
                ToolErrorKind::Custom,
                format!("No element matches selector '{selector}': {e}"),
            )
        })?;
        element.click().await.map_err(|e| cdp_failed("click", e))?;
        Ok(())
    }

    /// Type `text` into the first element matching `selector`, optionally
    /// pressing Enter afterward.
    pub async fn type_text(&self, selector: &str, text: &str, submit: bool) -> Result<(), ToolError> {
        let page = self.ensure_page().await?;
        let element = page.find_element(selector).await.map_err(|e| {
            ToolError::new(
                ToolErrorKind::Custom,
                format!("No element matches selector '{selector}': {e}"),
            )
        })?;
        element
            .click()
            .await
            .map_err(|e| cdp_failed("type (focus)", e))?
            .type_str(text)
            .await
            .map_err(|e| cdp_failed("type", e))?;
        if submit {
            element
                .press_key("Enter")
                .await
                .map_err(|e| cdp_failed("submit", e))?;
        }
        Ok(())
    }

    /// Capture the current tab as a PNG. `full_page` captures the entire
    /// scrollable page rather than just the visible viewport.
    pub async fn screenshot(&self, full_page: bool) -> Result<Vec<u8>, ToolError> {
        let page = self.ensure_page().await?;
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(full_page)
            .build();
        page.screenshot(params)
            .await
            .map_err(|e| cdp_failed("screenshot", e))
    }

    /// Extract visible text. `selector: None` returns the whole page's
    /// rendered text content; `Some(selector)` scopes to the first matching
    /// element.
    pub async fn get_text(&self, selector: Option<&str>) -> Result<String, ToolError> {
        let page = self.ensure_page().await?;
        match selector {
            None => page
                .evaluate("document.body ? document.body.innerText : ''")
                .await
                .map_err(|e| cdp_failed("text extraction", e))?
                .into_value::<String>()
                .map_err(|e| cdp_failed("text extraction", e)),
            Some(sel) => {
                let element = page.find_element(sel).await.map_err(|e| {
                    ToolError::new(
                        ToolErrorKind::Custom,
                        format!("No element matches selector '{sel}': {e}"),
                    )
                })?;
                Ok(element.inner_text().await.map_err(|e| cdp_failed("text extraction", e))?
                    .unwrap_or_default())
            }
        }
    }

    /// Tear down the browser and tab. Idempotent — a no-op if nothing was
    /// ever launched.
    pub async fn close(&self) -> Result<(), ToolError> {
        let mut state = self.state.lock().await;
        state.page = None;
        if let Some(mut browser) = state.browser.take() {
            // Best-effort: the process may already be gone; either way the
            // handle is dropped and the pump task aborted right after.
            let _ = browser.close().await;
        }
        if let Some(task) = state.handler_task.take() {
            task.abort();
        }
        Ok(())
    }

    /// Whether a browser has ever been launched in this session — used only
    /// to give `browser_close` an accurate "already closed" confirmation.
    pub async fn is_running(&self) -> bool {
        self.state.lock().await.browser.is_some()
    }
}
