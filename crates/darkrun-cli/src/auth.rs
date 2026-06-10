//! `darkrun auth` — the CLI half of the website-brokered OAuth flow.
//!
//! The website hosts the OAuth dance and holds the client secrets; the CLI only
//! ever talks to the website, never to the provider directly:
//!
//! 1. `login` generates a random nonce and opens the browser to
//!    `<web>/auth/<provider>/start?state=<nonce>`.
//! 2. The provider redirects to `<web>/auth/<provider>/callback`, the server
//!    exchanges the code for a token and parks it under the nonce.
//! 3. The CLI polls `<web>/auth/broker/<nonce>` until the token arrives, then
//!    persists it via [`CredentialStore`].
//!
//! Everything that touches the network goes through `darkrun-vcs`'s
//! [`HttpTransport`] seam, so the URL building, broker claim, and status/logout
//! paths are all unit-testable offline with `MockTransport`.

use std::time::Duration;

use darkrun_vcs::{
    Credential, CredentialStore, HttpRequest, HttpResponse, HttpTransport, Provider,
};

/// The default website base when `DARKRUN_WEB_BASE` is unset.
pub const DEFAULT_WEB_BASE: &str = "https://darkrun.ai";

/// Environment variable overriding the website base URL.
pub const WEB_BASE_ENV: &str = "DARKRUN_WEB_BASE";

/// How long `login` waits for the browser round-trip before giving up.
const POLL_TIMEOUT: Duration = Duration::from_secs(180);

/// How long between broker polls.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Resolve the website base, honoring `DARKRUN_WEB_BASE`, trimming any trailing
/// slash so callers can append `/auth/...` cleanly.
pub fn web_base() -> String {
    let raw = std::env::var(WEB_BASE_ENV).unwrap_or_else(|_| DEFAULT_WEB_BASE.to_string());
    raw.trim_end_matches('/').to_string()
}

/// Build the browser-facing OAuth start URL:
/// `<web>/auth/<provider>/start?state=<nonce>`.
pub fn start_url(web_base: &str, provider: Provider, nonce: &str) -> String {
    format!(
        "{base}/auth/{provider}/start?state={nonce}",
        base = web_base.trim_end_matches('/'),
        provider = provider.key(),
        nonce = darkrun_vcs::percent_encode(nonce),
    )
}

/// Build the broker poll URL: `<web>/auth/broker/<nonce>`.
pub fn broker_url(web_base: &str, nonce: &str) -> String {
    format!(
        "{base}/auth/broker/{nonce}",
        base = web_base.trim_end_matches('/'),
        nonce = darkrun_vcs::percent_encode(nonce),
    )
}

/// Generate a URL-safe random nonce. Uses process + time entropy mixed through
/// a small splitmix64 so we avoid pulling in an RNG crate for one value.
pub fn generate_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    // A monotonic per-call counter guarantees successive calls differ even when
    // the system clock hasn't advanced a nanosecond between them.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seed = {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let bump = COUNTER.fetch_add(1, Ordering::Relaxed);
        nanos
            ^ (std::process::id() as u64).rotate_left(17)
            ^ bump.rotate_left(40)
            ^ 0x9E37_79B9_7F4A_7C15
    };
    let mut state = seed;
    let mut out = String::with_capacity(32);
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    for _ in 0..32 {
        // splitmix64 step.
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.push(ALPHABET[(z % ALPHABET.len() as u64) as usize] as char);
    }
    out
}

/// The outcome of a single broker poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// The token is ready — here is the parked credential.
    Ready(Box<Credential>),
    /// The token is not parked yet; keep polling.
    Pending,
}

/// Poll the broker once. A `200` with a JSON [`Credential`] body means ready; a
/// `404` (or any other non-2xx) means the token has not been parked yet.
///
/// Pure over the transport seam — the timing loop lives in [`poll_until_ready`].
pub fn poll_broker(
    transport: &dyn HttpTransport,
    web_base: &str,
    nonce: &str,
) -> Result<PollOutcome, Box<dyn std::error::Error>> {
    let url = broker_url(web_base, nonce);
    let response = transport.execute(HttpRequest::get(url))?;
    if response.is_success() {
        let cred: Credential = response.json()?;
        Ok(PollOutcome::Ready(Box::new(cred)))
    } else {
        Ok(PollOutcome::Pending)
    }
}

/// A sleeper seam so the polling loop is testable without real time.
pub trait Sleeper {
    /// Sleep for `dur`.
    fn sleep(&self, dur: Duration);
    /// The elapsed time since the loop began, for timeout accounting.
    fn elapsed(&self) -> Duration;
}

/// The real, wall-clock sleeper used by the binary.
struct RealSleeper {
    start: std::time::Instant,
}

impl RealSleeper {
    fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Sleeper for RealSleeper {
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Poll the broker until the token is ready or `timeout` elapses.
pub fn poll_until_ready(
    transport: &dyn HttpTransport,
    sleeper: &dyn Sleeper,
    web_base: &str,
    nonce: &str,
    timeout: Duration,
    interval: Duration,
) -> Result<Credential, Box<dyn std::error::Error>> {
    loop {
        match poll_broker(transport, web_base, nonce)? {
            PollOutcome::Ready(cred) => return Ok(*cred),
            PollOutcome::Pending => {
                if sleeper.elapsed() >= timeout {
                    return Err(format!(
                        "timed out after {}s waiting for the browser to finish authorizing",
                        timeout.as_secs()
                    )
                    .into());
                }
                sleeper.sleep(interval);
            }
        }
    }
}

/// A real [`HttpTransport`] backed by a blocking `reqwest` client. Lives in the
/// binary so `darkrun-vcs` stays HTTP-client-free.
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    /// Build a transport with a sensible default timeout.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { client })
    }
}

impl HttpTransport for ReqwestTransport {
    #[cfg(not(tarpaulin_include))] // real blocking HTTP — irreducible network I/O
    fn execute(&self, request: HttpRequest) -> darkrun_vcs::Result<HttpResponse> {
        let method = match request.method {
            darkrun_vcs::Method::Get => reqwest::Method::GET,
            darkrun_vcs::Method::Post => reqwest::Method::POST,
            darkrun_vcs::Method::Put => reqwest::Method::PUT,
        };
        let mut builder = self.client.request(method, &request.url);
        for (k, v) in &request.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let resp = builder
            .send()
            .map_err(|e| darkrun_vcs::VcsError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .map_err(|e| darkrun_vcs::VcsError::Transport(e.to_string()))?;
        Ok(HttpResponse::new(status, bytes.to_vec()))
    }
}

/// Open `url` in the operator's default browser, best-effort. A failure is not
/// fatal — the URL is always printed so the operator can open it by hand.
#[cfg(not(tarpaulin_include))] // spawns the OS browser opener — irreducible process I/O
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let prog = ("open", vec![url]);
    #[cfg(target_os = "linux")]
    let prog = ("xdg-open", vec![url]);
    #[cfg(target_os = "windows")]
    let prog = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let prog: (&str, Vec<&str>) = ("true", vec![]);

    let _ = std::process::Command::new(prog.0)
        .args(prog.1)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `darkrun auth login --provider …`.
///
/// Generates a nonce, opens the browser to the website's start URL (printing it
/// too), polls the broker for the token, and persists it to the store.
#[cfg(not(tarpaulin_include))] // opens a browser + polls a live broker
pub fn login(
    provider: Provider,
    store: &CredentialStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let base = web_base();
    let nonce = generate_nonce();
    let url = start_url(&base, provider, &nonce);

    println!("Opening your browser to authorize with {}…", provider.display_name());
    println!("  {url}");
    println!("If it doesn't open automatically, paste the URL above into your browser.");
    open_browser(&url);

    let transport = ReqwestTransport::new()?;
    let sleeper = RealSleeper::new();
    let cred = poll_until_ready(
        &transport,
        &sleeper,
        &base,
        &nonce,
        POLL_TIMEOUT,
        POLL_INTERVAL,
    )?;

    store.save(&cred)?;
    println!(
        "Authorized with {} — credential saved to {}",
        provider.display_name(),
        store.path().display()
    );
    Ok(())
}

/// `darkrun auth status` — print which providers currently have a stored
/// credential. Returns the lines printed (for testing).
pub fn status_lines(store: &CredentialStore) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let authed = store.list()?;
    let mut lines = Vec::new();
    for provider in [Provider::GitHub, Provider::GitLab] {
        let mark = if authed.contains(&provider) {
            "authorized"
        } else {
            "not authorized"
        };
        lines.push(format!("{:<8} {}", provider.display_name(), mark));
    }
    Ok(lines)
}

/// `darkrun auth status`.
pub fn status(store: &CredentialStore) -> Result<(), Box<dyn std::error::Error>> {
    for line in status_lines(store)? {
        println!("{line}");
    }
    Ok(())
}

/// `darkrun auth logout --provider …` — remove a stored credential. Returns
/// whether one was removed.
pub fn logout(
    provider: Provider,
    store: &CredentialStore,
) -> Result<bool, Box<dyn std::error::Error>> {
    let removed = store.remove(provider)?;
    if removed {
        println!("Removed {} credential.", provider.display_name());
    } else {
        println!("No {} credential to remove.", provider.display_name());
    }
    Ok(removed)
}

/// Parse a `--provider` CLI value into a [`Provider`].
pub fn parse_provider(value: &str) -> Result<Provider, Box<dyn std::error::Error>> {
    Provider::from_key(value)
        .ok_or_else(|| format!("unknown provider '{value}' (expected github or gitlab)").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_vcs::{Method, MockTransport};
    use std::cell::Cell;

    fn temp_store() -> (tempfile::TempDir, CredentialStore) {
        let dir = tempfile::tempdir().expect("tmp");
        let store = CredentialStore::at(dir.path().join("credentials"));
        (dir, store)
    }

    #[test]
    fn start_url_includes_provider_and_state() {
        let url = start_url("https://darkrun.ai", Provider::GitHub, "abc123");
        assert_eq!(
            url,
            "https://darkrun.ai/auth/github/start?state=abc123"
        );
        let gl = start_url("https://darkrun.ai", Provider::GitLab, "n");
        assert_eq!(gl, "https://darkrun.ai/auth/gitlab/start?state=n");
    }

    #[test]
    fn start_url_trims_trailing_slash() {
        let url = start_url("https://darkrun.ai/", Provider::GitHub, "x");
        assert_eq!(url, "https://darkrun.ai/auth/github/start?state=x");
    }

    #[test]
    fn broker_url_is_built_under_auth_broker() {
        assert_eq!(
            broker_url("https://darkrun.ai", "nonce-1"),
            "https://darkrun.ai/auth/broker/nonce-1"
        );
    }

    #[test]
    fn web_base_honors_env_and_default() {
        // Default path (env unset).
        std::env::remove_var(WEB_BASE_ENV);
        assert_eq!(web_base(), "https://darkrun.ai");
        std::env::set_var(WEB_BASE_ENV, "http://localhost:8080/");
        assert_eq!(web_base(), "http://localhost:8080");
        std::env::remove_var(WEB_BASE_ENV);
    }

    #[test]
    fn nonce_is_long_and_url_safe() {
        let n = generate_nonce();
        assert_eq!(n.len(), 32);
        assert!(n.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn nonces_differ_across_calls() {
        // Process+time entropy: two calls should not collide in practice.
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn poll_broker_pending_on_404() {
        let mock = MockTransport::new();
        mock.expect(
            Method::Get,
            "https://darkrun.ai/auth/broker/n",
            HttpResponse::new(404, b"not found".to_vec()),
        );
        let out = poll_broker(&mock, "https://darkrun.ai", "n").unwrap();
        assert_eq!(out, PollOutcome::Pending);
    }

    #[test]
    fn poll_broker_ready_parses_credential() {
        let mock = MockTransport::new();
        let body = serde_json::to_vec(&Credential::new(Provider::GitHub, "tok-xyz")).unwrap();
        mock.expect(
            Method::Get,
            "https://darkrun.ai/auth/broker/n",
            HttpResponse::new(200, body),
        );
        let out = poll_broker(&mock, "https://darkrun.ai", "n").unwrap();
        match out {
            PollOutcome::Ready(c) => {
                assert_eq!(c.access_token, "tok-xyz");
                assert_eq!(c.provider, Provider::GitHub);
            }
            PollOutcome::Pending => panic!("expected ready"),
        }
    }

    /// A test sleeper that counts ticks and reports a caller-controlled elapsed.
    struct FakeSleeper {
        ticks: Cell<u32>,
        elapsed_after: Duration,
        // Returns `elapsed_after` once `ticks` reaches `time_out_at`.
        time_out_at: u32,
    }

    impl Sleeper for FakeSleeper {
        fn sleep(&self, _dur: Duration) {
            self.ticks.set(self.ticks.get() + 1);
        }
        fn elapsed(&self) -> Duration {
            if self.ticks.get() >= self.time_out_at {
                self.elapsed_after
            } else {
                Duration::ZERO
            }
        }
    }

    #[test]
    fn poll_until_ready_returns_on_first_ready() {
        let mock = MockTransport::new();
        let body = serde_json::to_vec(&Credential::new(Provider::GitLab, "gl-tok")).unwrap();
        mock.expect(
            Method::Get,
            "https://darkrun.ai/auth/broker/n",
            HttpResponse::new(200, body),
        );
        let sleeper = FakeSleeper {
            ticks: Cell::new(0),
            elapsed_after: Duration::ZERO,
            time_out_at: u32::MAX,
        };
        let cred = poll_until_ready(
            &mock,
            &sleeper,
            "https://darkrun.ai",
            "n",
            Duration::from_secs(10),
            Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(cred.access_token, "gl-tok");
        assert_eq!(sleeper.ticks.get(), 0, "ready on first poll → no sleeps");
    }

    #[test]
    fn poll_until_ready_polls_then_succeeds() {
        let mock = MockTransport::new();
        // First poll pending (404), second ready (200).
        mock.expect(
            Method::Get,
            "https://darkrun.ai/auth/broker/n",
            HttpResponse::new(404, b"".to_vec()),
        );
        let body = serde_json::to_vec(&Credential::new(Provider::GitHub, "late")).unwrap();
        mock.expect(
            Method::Get,
            "https://darkrun.ai/auth/broker/n",
            HttpResponse::new(200, body),
        );
        let sleeper = FakeSleeper {
            ticks: Cell::new(0),
            elapsed_after: Duration::ZERO,
            time_out_at: u32::MAX,
        };
        let cred = poll_until_ready(
            &mock,
            &sleeper,
            "https://darkrun.ai",
            "n",
            Duration::from_secs(10),
            Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(cred.access_token, "late");
        assert_eq!(sleeper.ticks.get(), 1, "one sleep between the two polls");
    }

    #[test]
    fn poll_until_ready_times_out() {
        let mock = MockTransport::new();
        // Always pending.
        for _ in 0..5 {
            mock.expect(
                Method::Get,
                "https://darkrun.ai/auth/broker/n",
                HttpResponse::new(404, b"".to_vec()),
            );
        }
        let sleeper = FakeSleeper {
            ticks: Cell::new(0),
            elapsed_after: Duration::from_secs(999),
            time_out_at: 1, // report timed-out after the first sleep
        };
        let err = poll_until_ready(
            &mock,
            &sleeper,
            "https://darkrun.ai",
            "n",
            Duration::from_secs(10),
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn status_lines_reflect_stored_credentials() {
        let (_d, store) = temp_store();
        // Nothing stored.
        let lines = status_lines(&store).unwrap();
        assert!(lines.iter().all(|l| l.contains("not authorized")));

        // Save a GitHub credential.
        store
            .save(&Credential::new(Provider::GitHub, "tok"))
            .unwrap();
        let lines = status_lines(&store).unwrap();
        let gh = lines.iter().find(|l| l.contains("GitHub")).unwrap();
        assert!(gh.contains("authorized") && !gh.contains("not authorized"));
        let gl = lines.iter().find(|l| l.contains("GitLab")).unwrap();
        assert!(gl.contains("not authorized"));
    }

    #[test]
    fn logout_removes_existing_credential() {
        let (_d, store) = temp_store();
        store
            .save(&Credential::new(Provider::GitLab, "tok"))
            .unwrap();
        assert!(logout(Provider::GitLab, &store).unwrap());
        // Second logout is a no-op.
        assert!(!logout(Provider::GitLab, &store).unwrap());
        assert!(store.get(Provider::GitLab).unwrap().is_none());
    }

    #[test]
    fn parse_provider_accepts_keys_and_aliases() {
        assert_eq!(parse_provider("github").unwrap(), Provider::GitHub);
        assert_eq!(parse_provider("gh").unwrap(), Provider::GitHub);
        assert_eq!(parse_provider("gitlab").unwrap(), Provider::GitLab);
        assert_eq!(parse_provider("gl").unwrap(), Provider::GitLab);
        assert!(parse_provider("bitbucket").is_err());
    }

    #[test]
    fn login_round_trip_saves_credential() {
        // Exercise the full login path minus the browser by reusing the broker
        // poll + save against a temp store. (The browser open is best-effort and
        // not asserted here.)
        let (_d, store) = temp_store();
        let mock = MockTransport::new();
        let body = serde_json::to_vec(&Credential::new(Provider::GitHub, "logged-in")).unwrap();
        mock.expect(
            Method::Get,
            "https://darkrun.ai/auth/broker/abc",
            HttpResponse::new(200, body),
        );
        let sleeper = FakeSleeper {
            ticks: Cell::new(0),
            elapsed_after: Duration::ZERO,
            time_out_at: u32::MAX,
        };
        let cred = poll_until_ready(
            &mock,
            &sleeper,
            "https://darkrun.ai",
            "abc",
            Duration::from_secs(5),
            Duration::from_millis(1),
        )
        .unwrap();
        store.save(&cred).unwrap();
        assert_eq!(
            store.get(Provider::GitHub).unwrap().unwrap().access_token,
            "logged-in"
        );
    }

    #[test]
    fn real_sleeper_and_reqwest_transport_smoke() {
        use std::time::Duration;
        let s = RealSleeper::new();
        s.sleep(Duration::from_millis(0));
        let _ = s.elapsed();
        // A real transport builds; a request to a dead port fails fast.
        let t = ReqwestTransport::new().expect("client builds");
        assert!(t.execute(HttpRequest::get("http://127.0.0.1:1/x")).is_err());
    }
}
