//! Optional Cloudflare Quick Tunnel, spawned via the `cloudflared` binary on
//! `$PATH` (or `$CLOUDFLARED_BIN`) — mirrors the npm bridge's `startTunnel()`
//! in `npm/failure/bin/mcp-server.js`. Best-effort: if `cloudflared` isn't
//! installed, the bridge still runs, just without a public URL.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

const URL_PATTERN: &str = r"https://[a-z0-9-]+\.trycloudflare\.com";

/// A running tunnel process. Killed when dropped.
pub struct Tunnel {
    child: Child,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Spawn `cloudflared tunnel --url http://127.0.0.1:<port>` and wait (up to
/// `timeout`) for it to announce a public URL on stdout/stderr. Returns
/// `Ok(None)` (not an error) when `cloudflared` isn't installed — a missing
/// tunnel binary is a normal, expected case and must not fail bridge startup.
pub async fn start(port: u16, timeout: Duration) -> anyhow::Result<Option<(Tunnel, String)>> {
    let bin = std::env::var("CLOUDFLARED_BIN").unwrap_or_else(|_| "cloudflared".to_string());

    let mut child = match Command::new(&bin)
        .args([
            "tunnel",
            "--no-autoupdate",
            "--url",
            &format!("http://127.0.0.1:{port}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let re = regex::Regex::new(URL_PATTERN).expect("valid regex");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);
    spawn_url_watcher(stdout, re.clone(), tx.clone());
    spawn_url_watcher(stderr, re, tx.clone());
    drop(tx);

    match tokio::time::timeout(timeout, rx.recv()).await {
        Ok(Some(url)) => Ok(Some((Tunnel { child }, url))),
        _ => {
            let _ = child.start_kill();
            Ok(None)
        }
    }
}

fn spawn_url_watcher(
    reader: impl AsyncRead + Unpin + Send + 'static,
    re: regex::Regex,
    tx: tokio::sync::mpsc::Sender<String>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(m) = re.find(&line) {
                let _ = tx.send(m.as_str().to_string()).await;
                return;
            }
        }
    });
}
