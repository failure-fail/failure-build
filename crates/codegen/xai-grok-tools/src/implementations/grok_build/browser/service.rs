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
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use xai_tool_runtime::{ToolError, ToolErrorKind};

/// Navigation waits on network + redirect chains, so it gets more slack than
/// same-page actions.
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
/// Click/type/screenshot/text-extraction only touch the already-loaded page;
/// a hung/adversarial page (busy JS, `beforeunload` trap) shouldn't be able
/// to wedge the tool call indefinitely.
const ACTION_TIMEOUT: Duration = Duration::from_secs(15);

async fn with_timeout<T>(
    dur: Duration,
    action: &str,
    fut: impl Future<Output = Result<T, ToolError>>,
) -> Result<T, ToolError> {
    match tokio::time::timeout(dur, fut).await {
        Ok(result) => result,
        Err(_) => Err(ToolError::new(
            ToolErrorKind::Custom,
            format!("Browser {action} timed out after {}s", dur.as_secs()),
        )),
    }
}

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
                // can't use Chrome's setuid sandbox. This is a real
                // trade-off, not a free one: a renderer/V8 bug in an
                // attacker-controlled page becomes host code execution as
                // the CLI's own user, not just an info leak. Accepted for
                // now because a setuid sandbox is frequently unavailable in
                // exactly the environments this CLI runs in; the ephemeral
                // automation profile (no cookies/history of value) only
                // bounds the *browser-data* blast radius, not this one.
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
        with_timeout(NAVIGATE_TIMEOUT, "navigation", async {
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
        })
        .await
    }

    /// Click the first element matching `selector` (CSS selector).
    pub async fn click(&self, selector: &str) -> Result<(), ToolError> {
        with_timeout(ACTION_TIMEOUT, "click", async {
            let page = self.ensure_page().await?;
            let element = page.find_element(selector).await.map_err(|e| {
                ToolError::new(
                    ToolErrorKind::Custom,
                    format!("No element matches selector '{selector}': {e}"),
                )
            })?;
            element.click().await.map_err(|e| cdp_failed("click", e))?;
            Ok(())
        })
        .await
    }

    /// Type `text` into the first element matching `selector`, optionally
    /// pressing Enter afterward.
    pub async fn type_text(&self, selector: &str, text: &str, submit: bool) -> Result<(), ToolError> {
        with_timeout(ACTION_TIMEOUT, "type", async {
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
        })
        .await
    }

    /// Capture the current tab as a PNG. `full_page` captures the entire
    /// scrollable page rather than just the visible viewport.
    pub async fn screenshot(&self, full_page: bool) -> Result<Vec<u8>, ToolError> {
        with_timeout(ACTION_TIMEOUT, "screenshot", async {
            let page = self.ensure_page().await?;
            let params = ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .full_page(full_page)
                .build();
            page.screenshot(params)
                .await
                .map_err(|e| cdp_failed("screenshot", e))
        })
        .await
    }

    /// Extract visible text. `selector: None` returns the whole page's
    /// rendered text content; `Some(selector)` scopes to the first matching
    /// element.
    pub async fn get_text(&self, selector: Option<&str>) -> Result<String, ToolError> {
        with_timeout(ACTION_TIMEOUT, "text extraction", async {
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
                    Ok(element
                        .inner_text()
                        .await
                        .map_err(|e| cdp_failed("text extraction", e))?
                        .unwrap_or_default())
                }
            }
        })
        .await
    }

    /// Current tab's URL, if a page is open. Used to re-check the landing
    /// site after an action (e.g. a click) that might have navigated it.
    pub async fn current_url(&self) -> Option<String> {
        let state = self.state.lock().await;
        let page = state.page.as_ref()?;
        page.url().await.ok().flatten()
    }

    /// Reset the current tab to a blank page, discarding whatever was
    /// loaded. Used to purge content from a page that turned out to have
    /// navigated (via redirect or in-page action) to a blocked destination,
    /// so a later browser_get_text/browser_screenshot can't still read it.
    pub async fn reset_to_blank(&self) -> Result<(), ToolError> {
        let page = self.ensure_page().await?;
        page.goto("about:blank")
            .await
            .map_err(|e| cdp_failed("reset", e))?;
        Ok(())
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
