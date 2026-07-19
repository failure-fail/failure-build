//! Axum HTTP layer for the native MCP bridge: mounts rmcp's Streamable HTTP
//! service behind bearer/query-token auth, matching the npm bridge's
//! `authorized()` check in `npm/failure/bin/mcp-server.js`. `/health` stays
//! unauthenticated, same as the npm bridge.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use super::agent::BridgeAgent;
use super::server_handler::BridgeToolServer;

#[derive(Clone)]
struct AuthState {
    token: Arc<str>,
}

async fn auth_middleware(State(state): State<AuthState>, req: Request<Body>, next: Next) -> Response {
    let expected_bearer = format!("Bearer {}", state.token);
    let header_ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == expected_bearer);
    let query_ok = req
        .uri()
        .query()
        .is_some_and(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .any(|(k, v)| k == "token" && v.as_ref() == state.token.as_ref())
        });

    if !header_ok && !query_ok {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error": "Missing or invalid Failure MCP token"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// A running bridge HTTP server. Dropping this without calling `shutdown`
/// leaves the server running — call `shutdown` explicitly on `/mcp stop`.
pub struct ServerHandle {
    pub addr: SocketAddr,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl ServerHandle {
    pub fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
    }
}

pub async fn serve(
    port: u16,
    token: String,
    agent: Arc<BridgeAgent>,
    default_cwd: PathBuf,
) -> anyhow::Result<ServerHandle> {
    let service = StreamableHttpService::new(
        move || Ok(BridgeToolServer::new(agent.clone(), default_cwd.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let auth_state = AuthState { token: Arc::from(token) };
    let mcp_router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ))
        .with_state(auth_state);

    let router = axum::Router::new()
        .route(
            "/health",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
        )
        .merge(mcp_router);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    Ok(ServerHandle { addr, shutdown_tx })
}
