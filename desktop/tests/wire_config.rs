//! `ConnConfig` construction, URL/path building, and `from_env` defaulting.

use darkrun_desktop::wire::ConnConfig;

fn cfg(host: &str, port: u16, session: &str) -> ConnConfig {
    ConnConfig {
        host: host.to_string(),
        port,
        session_id: session.to_string(),
    }
}

// ---- ws_url ----

#[test]
fn ws_url_basic() {
    let c = cfg("127.0.0.1", 7878, "abc");
    assert_eq!(c.ws_url(), "ws://127.0.0.1:7878/ws/session/abc");
}

#[test]
fn ws_url_uses_host_and_port() {
    let c = cfg("localhost", 9000, "s1");
    assert_eq!(c.ws_url(), "ws://localhost:9000/ws/session/s1");
}

#[test]
fn ws_url_session_id_verbatim() {
    let c = cfg("127.0.0.1", 1, "current");
    assert_eq!(c.ws_url(), "ws://127.0.0.1:1/ws/session/current");
}

#[test]
fn ws_url_with_dashes_and_uuid_like_id() {
    let c = cfg("127.0.0.1", 7878, "9f3a-bb12-0001");
    assert_eq!(c.ws_url(), "ws://127.0.0.1:7878/ws/session/9f3a-bb12-0001");
}

#[test]
fn ws_url_port_max() {
    let c = cfg("127.0.0.1", u16::MAX, "x");
    assert_eq!(c.ws_url(), "ws://127.0.0.1:65535/ws/session/x");
}

#[test]
fn ws_url_port_zero() {
    let c = cfg("127.0.0.1", 0, "x");
    assert_eq!(c.ws_url(), "ws://127.0.0.1:0/ws/session/x");
}

// ---- decide_path ----

#[test]
fn decide_path_basic() {
    assert_eq!(cfg("127.0.0.1", 7878, "abc").decide_path(), "/review/abc/decide");
}

#[test]
fn decide_path_uses_only_session_id() {
    // Host/port don't appear in the path.
    let a = cfg("127.0.0.1", 7878, "z").decide_path();
    let b = cfg("example.com", 1, "z").decide_path();
    assert_eq!(a, b);
    assert_eq!(a, "/review/z/decide");
}

#[test]
fn decide_path_with_complex_id() {
    assert_eq!(
        cfg("127.0.0.1", 7878, "run-2026-05-30").decide_path(),
        "/review/run-2026-05-30/decide"
    );
}

#[test]
fn unit_reset_path_is_run_and_unit_scoped() {
    assert_eq!(
        cfg("127.0.0.1", 7878, "sess").unit_reset_path("my-run", "u1"),
        "/api/unit/my-run/u1/reset"
    );
}

// ---- authority ----

#[test]
fn authority_joins_host_and_port() {
    assert_eq!(cfg("127.0.0.1", 7878, "x").authority(), "127.0.0.1:7878");
}

#[test]
fn authority_other_host() {
    assert_eq!(cfg("10.0.0.5", 8080, "x").authority(), "10.0.0.5:8080");
}

#[test]
fn authority_independent_of_session() {
    let a = cfg("127.0.0.1", 7878, "one").authority();
    let b = cfg("127.0.0.1", 7878, "two").authority();
    assert_eq!(a, b);
}

// ---- equality / clone ----

#[test]
fn config_eq_and_clone() {
    let a = cfg("127.0.0.1", 7878, "abc");
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn config_ne_on_any_field() {
    let base = cfg("127.0.0.1", 7878, "abc");
    assert_ne!(base, cfg("127.0.0.2", 7878, "abc"));
    assert_ne!(base, cfg("127.0.0.1", 7879, "abc"));
    assert_ne!(base, cfg("127.0.0.1", 7878, "abd"));
}

// ---- from_env: defaults + parsing ----
//
// `from_env` reads global process env, and `#[test]`s in one integration file
// share a process and run concurrently. All env cases are exercised inside a
// single serialized test so they can't race each other.

#[test]
fn from_env_cases() {
    fn clear() {
        std::env::remove_var("DARKRUN_PORT");
        std::env::remove_var("DARKRUN_SESSION_ID");
    }

    // Defaults when unset.
    clear();
    let c = ConnConfig::from_env();
    assert_eq!(c.host, "127.0.0.1");
    assert_eq!(c.port, 7878);
    assert_eq!(c.session_id, "current");

    // Reads both vars.
    std::env::set_var("DARKRUN_PORT", "9123");
    std::env::set_var("DARKRUN_SESSION_ID", "sess-42");
    let c = ConnConfig::from_env();
    assert_eq!(c.port, 9123);
    assert_eq!(c.session_id, "sess-42");
    assert_eq!(c.host, "127.0.0.1");

    // Invalid port -> default.
    clear();
    std::env::set_var("DARKRUN_PORT", "not-a-number");
    assert_eq!(ConnConfig::from_env().port, 7878);

    // Out-of-range port (> u16::MAX) -> default.
    std::env::set_var("DARKRUN_PORT", "70000");
    assert_eq!(ConnConfig::from_env().port, 7878);

    // Port is trimmed before parse.
    std::env::set_var("DARKRUN_PORT", "  8080  ");
    assert_eq!(ConnConfig::from_env().port, 8080);

    // Port zero is valid.
    std::env::set_var("DARKRUN_PORT", "0");
    assert_eq!(ConnConfig::from_env().port, 0);

    // Empty / whitespace session -> default.
    clear();
    std::env::set_var("DARKRUN_SESSION_ID", "");
    assert_eq!(ConnConfig::from_env().session_id, "current");
    std::env::set_var("DARKRUN_SESSION_ID", "   ");
    assert_eq!(ConnConfig::from_env().session_id, "current");

    // Non-blank session preserved verbatim (surrounding whitespace kept).
    std::env::set_var("DARKRUN_SESSION_ID", " keep-me ");
    assert_eq!(ConnConfig::from_env().session_id, " keep-me ");

    clear();
}
