//! darkrun observability — one Sentry init shared by every binary surface.
//!
//! All four surfaces report to Sentry through [`init`]: the `darkrun` CLI/MCP
//! binary, the `darkrun-desktop` app, the `darkrun-web` server, and (separately,
//! via the browser JS SDK) the website. Each calls `init(<service>)` at the top
//! of `main` and holds the returned guard for the process lifetime.
//!
//! **DSN resolution** is the key design point. For the DISTRIBUTED binaries (the
//! CLI + desktop) the DSN is compiled in: the release build sets
//! `DARKRUN_SENTRY_DSN` so [`option_env!`] bakes it into the artifact and a
//! shipped binary reports without any runtime config. The SERVER reads the same
//! var from its environment (Cloud Run wires it from Secret Manager). A build /
//! run with no DSN is a clean no-op — local dev never phones home.
//!
//! C-free: the transport is reqwest + rustls (no native-tls / openssl).

#![deny(missing_docs)]

/// The DSN to report to, or `None` when unset (→ telemetry is a no-op).
///
/// Compile-time (`DARKRUN_SENTRY_DSN` at build, baked into distributed binaries)
/// takes precedence; otherwise the run-time environment (the server's case).
fn dsn() -> Option<String> {
    option_env!("DARKRUN_SENTRY_DSN")
        .map(str::to_string)
        .or_else(|| std::env::var("DARKRUN_SENTRY_DSN").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The release identifier reported to Sentry — `darkrun@<version>`, or the exact
/// build tag when the release pipeline injects `DARKRUN_RELEASE`.
fn release() -> std::borrow::Cow<'static, str> {
    match option_env!("DARKRUN_RELEASE") {
        Some(r) if !r.trim().is_empty() => r.into(),
        _ => format!("darkrun@{}", env!("CARGO_PKG_VERSION")).into(),
    }
}

/// The deployment environment tag — `DARKRUN_ENV` if set, else `development` for
/// debug builds and `production` for release builds.
fn environment() -> std::borrow::Cow<'static, str> {
    if let Ok(env) = std::env::var("DARKRUN_ENV") {
        let env = env.trim().to_string();
        if !env.is_empty() {
            return env.into();
        }
    }
    if cfg!(debug_assertions) {
        "development".into()
    } else {
        "production".into()
    }
}

/// Initialize Sentry for `service` (e.g. `"cli"`, `"desktop"`, `"web"`), tagging
/// every event with that surface. Returns the client guard — **hold it for the
/// program's lifetime** (dropping it flushes and shuts Sentry down). Returns
/// `None` when no DSN is configured, in which case telemetry is a no-op.
///
/// Panics are captured automatically (the `panic` integration). The PII default
/// is off — `send_default_pii` stays `false`.
#[must_use = "hold the guard for the process lifetime; dropping it disables Sentry"]
pub fn init(service: &'static str) -> Option<sentry::ClientInitGuard> {
    let dsn = dsn()?;
    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: Some(release()),
            environment: Some(environment()),
            // Conservative defaults: no PII, errors-only (no perf sampling by
            // default — surfaces can opt in via traces_sample_rate later).
            send_default_pii: false,
            ..Default::default()
        },
    ));
    sentry::configure_scope(|scope| {
        scope.set_tag("service", service);
    });
    Some(guard)
}
