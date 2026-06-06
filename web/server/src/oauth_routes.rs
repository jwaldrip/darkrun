//! The three OAuth endpoints the website hosts.
//!
//! ```text
//! GET /auth/:provider/start?state=NONCE     -> 302 to the provider authorize URL
//! GET /auth/:provider/callback?code&state   -> exchange code, park under nonce, HTML
//! GET /auth/broker/:nonce                    -> one-time JSON { provider, access_token }
//! ```
//!
//! The browser is the only client of `start`/`callback`; the CLI is the only
//! client of `broker`. The client secret is used solely inside `callback`'s
//! server-side exchange and never crosses to the browser or the CLI.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use darkrun_vcs::{authorize_url, exchange_code, Provider};
use serde::{Deserialize, Serialize};

use crate::state::WebState;

/// Query for `/auth/:provider/start` — the CLI-generated nonce.
#[derive(Debug, Deserialize)]
pub struct StartQuery {
    /// The opaque nonce tying this login to the waiting terminal.
    pub state: String,
}

/// Query for `/auth/:provider/callback` — what the provider returns.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    /// The authorization code to exchange for a token.
    pub code: Option<String>,
    /// The nonce echoed back, used to park the resulting credential.
    pub state: Option<String>,
    /// An OAuth error code, when the provider denies the request.
    pub error: Option<String>,
    /// The human-readable OAuth error description, when present.
    pub error_description: Option<String>,
}

/// The one-time payload the CLI claims from `/auth/broker/:nonce`.
#[derive(Debug, Serialize, Deserialize)]
pub struct BrokerPayload {
    /// The provider this token authenticates against.
    pub provider: Provider,
    /// The OAuth access token.
    pub access_token: String,
}

/// Resolve a `:provider` path segment, or `400` if unknown.
///
/// The `Err` branch carries a ready-to-return [`Response`] (an axum type that is
/// intentionally large); this is local control flow, not an error propagated up
/// a deep call stack, so the size is fine here.
#[allow(clippy::result_large_err)]
fn parse_provider(raw: &str) -> Result<Provider, Response> {
    Provider::from_key(raw).ok_or_else(|| {
        error_page(
            StatusCode::BAD_REQUEST,
            "Unknown provider",
            &format!("`{raw}` is not a supported provider."),
        )
    })
}

/// `GET /auth/:provider/start?state=NONCE`
///
/// Redirects the browser to the provider authorize URL with the configured
/// client id, the server's `redirect_uri`, the provider-default scope, and the
/// caller's nonce as `state`.
pub async fn start(
    State(state): State<WebState>,
    Path(provider_key): Path<String>,
    Query(query): Query<StartQuery>,
) -> Response {
    let provider = match parse_provider(&provider_key) {
        Ok(p) => p,
        Err(resp) => return resp,
    };

    if query.state.trim().is_empty() {
        return error_page(
            StatusCode::BAD_REQUEST,
            "Missing state",
            "A login nonce is required to start authorization.",
        );
    }

    let creds = match state.config.credentials(provider) {
        Some(c) => c,
        None => {
            return error_page(
                StatusCode::SERVICE_UNAVAILABLE,
                "Provider not configured",
                &format!(
                    "{} sign-in is not available on this server.",
                    provider.display_name()
                ),
            )
        }
    };

    let redirect_uri = state.config.redirect_uri(provider);
    let url = authorize_url(provider, &creds.client_id, &redirect_uri, &query.state);
    Redirect::temporary(&url).into_response()
}

/// `GET /auth/:provider/callback?code&state`
///
/// Exchanges the code for a token server-side, parks it under the nonce, and
/// returns the dark-branded "return to your terminal" page. Provider-reported
/// errors and missing parameters render a branded error page instead.
#[cfg(not(tarpaulin_include))] // OAuth callback: spawns a blocking token exchange over the network — irreducible I/O
pub async fn callback(
    State(state): State<WebState>,
    Path(provider_key): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let provider = match parse_provider(&provider_key) {
        Ok(p) => p,
        Err(resp) => return resp,
    };

    if let Some(err) = query.error {
        let detail = query
            .error_description
            .unwrap_or_else(|| "The provider denied the authorization request.".to_string());
        return error_page(
            StatusCode::BAD_REQUEST,
            "Authorization failed",
            &format!("{err}: {detail}"),
        );
    }

    let (code, nonce) = match (query.code, query.state) {
        (Some(c), Some(s)) if !c.is_empty() && !s.is_empty() => (c, s),
        _ => {
            return error_page(
                StatusCode::BAD_REQUEST,
                "Incomplete callback",
                "The provider callback was missing the code or state parameter.",
            )
        }
    };

    let creds = match state.config.credentials(provider) {
        Some(c) => c.clone(),
        None => {
            return error_page(
                StatusCode::SERVICE_UNAVAILABLE,
                "Provider not configured",
                &format!(
                    "{} sign-in is not available on this server.",
                    provider.display_name()
                ),
            )
        }
    };

    let redirect_uri = state.config.redirect_uri(provider);
    let transport = state.transport.clone();

    // The exchange is synchronous (the transport seam is) and may block on I/O;
    // run it off the async reactor.
    let exchanged = tokio::task::spawn_blocking(move || {
        exchange_code(
            transport.as_ref(),
            provider,
            &creds.client_id,
            &creds.client_secret,
            &code,
            &redirect_uri,
        )
    })
    .await;

    let credential = match exchanged {
        Ok(Ok(cred)) => cred,
        Ok(Err(e)) => {
            tracing::warn!(provider = provider.key(), error = %e, "token exchange failed");
            return error_page(
                StatusCode::BAD_GATEWAY,
                "Token exchange failed",
                "darkrun could not complete sign-in with the provider. Try again.",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "exchange task panicked");
            return error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error",
                "Something went wrong completing sign-in.",
            );
        }
    };

    state.broker.park(nonce, credential);
    success_page(provider)
}

/// `GET /auth/broker/:nonce`
///
/// Returns the parked credential as JSON exactly once, then evicts it. A second
/// poll, an unknown nonce, or an expired entry all return `404`.
pub async fn broker_claim(
    State(state): State<WebState>,
    Path(nonce): Path<String>,
) -> Response {
    match state.broker.claim(&nonce) {
        Some(cred) => Json(BrokerPayload {
            provider: cred.provider,
            access_token: cred.access_token,
        })
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response(),
    }
}

/// The minimal dark-branded "return to your terminal" page.
fn success_page(provider: Provider) -> Response {
    let body = page_shell(
        "Signed in",
        &format!(
            r#"<div class="badge">darkrun</div>
      <h1>You're signed in to {}.</h1>
      <p>Authorization is complete. Return to your terminal — darkrun is
      finishing the handshake. You can close this tab.</p>"#,
            provider.display_name()
        ),
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// A branded error page with the given status and message.
fn error_page(status: StatusCode, heading: &str, detail: &str) -> Response {
    let body = page_shell(
        heading,
        &format!(
            r#"<div class="badge">darkrun</div>
      <h1>{heading}</h1>
      <p>{detail}</p>"#
        ),
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// The shared dark-only HTML shell. No external assets; inline styles keep the
/// page self-contained for the brief moment it is shown.
fn page_shell(title: &str, inner: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="robots" content="noindex" />
    <title>{title} — darkrun</title>
    <style>
      :root {{ color-scheme: dark; }}
      html, body {{ margin: 0; height: 100%; background: #07090c; color: #e6e8ec;
        font: 16px/1.6 ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif; }}
      body {{ display: grid; place-items: center; padding: 2rem; }}
      main {{ max-width: 32rem; text-align: center; }}
      .badge {{ display: inline-block; letter-spacing: .12em; text-transform: uppercase;
        font-size: .72rem; color: #8a93a3; border: 1px solid #1c222b; border-radius: 999px;
        padding: .3rem .8rem; margin-bottom: 1.5rem; }}
      h1 {{ font-size: 1.5rem; font-weight: 700; margin: 0 0 .75rem; }}
      h1 b {{ font-weight: 800; }}
      p {{ color: #aab2c0; margin: 0; }}
    </style>
  </head>
  <body>
    <main>
      {inner}
    </main>
  </body>
</html>"#
    )
}
