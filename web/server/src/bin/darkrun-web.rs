//! The `darkrun-web` binary — runs the website host (OAuth + static site).
//!
//! Listen address comes from `DARKRUN_WEB_ADDR` (default `0.0.0.0:8787`). All
//! other configuration (OAuth client credentials, web base, site dir) is read
//! from the environment by [`darkrun_web::serve`].

use std::net::SocketAddr;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Sentry for the hosted web surface — the DSN comes from the environment
    // (Cloud Run wires it from Secret Manager). Held for the process lifetime.
    let _sentry = darkrun_telemetry::init("web");

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let addr: SocketAddr = std::env::var("DARKRUN_WEB_ADDR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "0.0.0.0:8787".to_string())
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;

    darkrun_web::serve(addr).await
}
