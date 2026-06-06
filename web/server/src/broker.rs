//! The short-lived, single-use credential broker.
//!
//! The OAuth dance is brokered: the browser lands on the server's callback, the
//! server exchanges the code for a token, and parks the resulting [`Credential`]
//! under the CLI-supplied nonce. The waiting terminal then polls
//! `/auth/broker/:nonce` exactly once to claim it.
//!
//! Two guarantees keep a parked token from leaking:
//!
//! * **single-use** — a successful claim evicts the entry, so a second poll (or
//!   a replayed request) gets a `404`.
//! * **TTL** — entries expire after [`BrokerEntry`]'s lifetime even if never
//!   claimed, so an abandoned login does not leave a token resident forever.
//!
//! The store is an in-memory map behind a `Mutex`; nothing touches disk. The
//! clock is injectable so tests can advance time without sleeping.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use darkrun_vcs::Credential;

/// Default lifetime of a parked credential: long enough for the browser
/// round-trip and the CLI's single poll, short enough that an abandoned login
/// evaporates quickly.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// A pluggable monotonic clock, so tests can advance time deterministically.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The real, wall-clock-backed [`Clock`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A parked credential and the instant it stops being claimable.
struct BrokerEntry {
    credential: Credential,
    expires_at: Instant,
}

/// The in-memory credential broker.
///
/// Cheap to [`Clone`] — clones share the same backing store, so the axum router
/// state can hold a copy per handler without duplicating parked tokens.
#[derive(Clone)]
pub struct Broker {
    inner: Arc<Mutex<HashMap<String, BrokerEntry>>>,
    ttl: Duration,
    clock: Arc<dyn Clock>,
}

impl Broker {
    /// A broker with the [`DEFAULT_TTL`] and a real clock.
    pub fn new() -> Self {
        Self::with_clock(DEFAULT_TTL, Arc::new(SystemClock))
    }

    /// A broker with an explicit TTL and clock — the seam tests use.
    pub fn with_clock(ttl: Duration, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
            clock,
        }
    }

    /// Park `credential` under `nonce`, replacing any prior entry for that nonce.
    ///
    /// The entry expires `ttl` from now even if never claimed.
    pub fn park(&self, nonce: impl Into<String>, credential: Credential) {
        let expires_at = self.clock.now() + self.ttl;
        let entry = BrokerEntry {
            credential,
            expires_at,
        };
        if let Ok(mut map) = self.inner.lock() {
            map.insert(nonce.into(), entry);
        }
    }

    /// Claim the credential parked under `nonce`, evicting it.
    ///
    /// Returns `None` if the nonce is unknown, already claimed, or expired. A
    /// claim is single-use: the entry is removed whether or not it had expired,
    /// so a replay can never resurrect it.
    pub fn claim(&self, nonce: &str) -> Option<Credential> {
        let now = self.clock.now();
        let mut map = self.inner.lock().ok()?;
        let entry = map.remove(nonce)?;
        if entry.expires_at <= now {
            // Expired between park and claim — evicted above, report miss.
            return None;
        }
        Some(entry.credential)
    }

    /// Drop every entry whose TTL has elapsed.
    ///
    /// Claims already evict lazily; this sweeps abandoned (never-claimed)
    /// entries so the map cannot grow without bound under churn.
    pub fn sweep_expired(&self) {
        let now = self.clock.now();
        if let Ok(mut map) = self.inner.lock() {
            map.retain(|_, e| e.expires_at > now);
        }
    }

    /// The number of live (not yet swept) entries — for tests/metrics.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Whether the broker currently holds no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_vcs::Provider;
    use std::sync::Mutex as StdMutex;

    /// A manually-advanced clock for deterministic TTL tests.
    struct FakeClock {
        now: StdMutex<Instant>,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                now: StdMutex::new(Instant::now()),
            })
        }
        fn advance(&self, by: Duration) {
            let mut g = self.now.lock().unwrap();
            *g += by;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
    }

    fn cred() -> Credential {
        Credential::new(Provider::GitHub, "tok-abc")
    }

    #[test]
    fn park_then_claim_returns_credential() {
        let broker = Broker::new();
        broker.park("nonce-1", cred());
        let got = broker.claim("nonce-1").expect("claim should succeed");
        assert_eq!(got.access_token, "tok-abc");
        assert_eq!(got.provider, Provider::GitHub);
    }

    #[test]
    fn claim_is_single_use() {
        let broker = Broker::new();
        broker.park("nonce-1", cred());
        assert!(broker.claim("nonce-1").is_some());
        assert!(
            broker.claim("nonce-1").is_none(),
            "second claim must miss after eviction"
        );
    }

    #[test]
    fn claim_evicts_entry() {
        let broker = Broker::new();
        broker.park("nonce-1", cred());
        assert_eq!(broker.len(), 1);
        let _ = broker.claim("nonce-1");
        assert!(broker.is_empty(), "successful claim must evict");
    }

    #[test]
    fn unknown_nonce_misses() {
        let broker = Broker::new();
        assert!(broker.claim("never-parked").is_none());
    }

    #[test]
    fn expired_entry_cannot_be_claimed() {
        let clock = FakeClock::new();
        let broker = Broker::with_clock(Duration::from_secs(60), clock.clone());
        broker.park("nonce-1", cred());
        clock.advance(Duration::from_secs(61));
        assert!(
            broker.claim("nonce-1").is_none(),
            "expired entry must not be claimable"
        );
    }

    #[test]
    fn claim_just_before_expiry_succeeds() {
        let clock = FakeClock::new();
        let broker = Broker::with_clock(Duration::from_secs(60), clock.clone());
        broker.park("nonce-1", cred());
        clock.advance(Duration::from_secs(59));
        assert!(broker.claim("nonce-1").is_some());
    }

    #[test]
    fn sweep_removes_only_expired() {
        let clock = FakeClock::new();
        let broker = Broker::with_clock(Duration::from_secs(60), clock.clone());
        broker.park("old", cred());
        clock.advance(Duration::from_secs(30));
        broker.park("fresh", cred());
        clock.advance(Duration::from_secs(40)); // old at 70s (expired), fresh at 40s
        broker.sweep_expired();
        assert_eq!(broker.len(), 1, "only the fresh entry should remain");
        assert!(broker.claim("fresh").is_some());
        assert!(broker.claim("old").is_none());
    }

    #[test]
    fn park_overwrites_same_nonce() {
        let broker = Broker::new();
        broker.park("n", Credential::new(Provider::GitHub, "first"));
        broker.park("n", Credential::new(Provider::GitLab, "second"));
        assert_eq!(broker.len(), 1);
        let got = broker.claim("n").unwrap();
        assert_eq!(got.access_token, "second");
        assert_eq!(got.provider, Provider::GitLab);
    }

    #[test]
    fn distinct_nonces_are_independent() {
        let broker = Broker::new();
        broker.park("a", Credential::new(Provider::GitHub, "ta"));
        broker.park("b", Credential::new(Provider::GitLab, "tb"));
        assert_eq!(broker.claim("a").unwrap().access_token, "ta");
        assert_eq!(broker.claim("b").unwrap().access_token, "tb");
    }

    #[test]
    fn default_broker_starts_empty() {
        let broker = Broker::default();
        assert!(broker.is_empty());
        assert_eq!(broker.len(), 0);
    }
}
