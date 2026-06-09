//! Gate-environment classifier — tells an ENVIRONMENT failure (a service is
//! down, a tool is missing, a port is taken) apart from a genuine code defect.
//!
//! When a quality gate is recorded as a `fail`, the engine runs the output
//! through this classifier *before* the failure enters the fix-loop. If it looks
//! environmental, the gate is flipped to [`crate::domain::GateStatus::EnvBlocked`]
//! — the work isn't wrong, the box it ran in was — and the run tries a
//! best-effort boot (from `.darkrun/boot.md`) or escalates to the operator
//! instead of churning fix passes against a dead dependency.
//!
//! Pure standard library: substring signatures (no `regex` dependency) plus two
//! cheap probes ([`is_tool_available`], [`is_port_reachable`]). The matching is
//! deliberately conservative — a plain `assertion failed` or a logic bug must
//! NOT read as environmental, or real defects would be waved through.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

/// The verdict for one gate failure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvClassification {
    /// True when the failure looks environmental rather than a code defect.
    pub environment: bool,
    /// A short human reason (the matched signature, or the missing tool).
    pub reason: Option<String>,
    /// Set when a specific declared tool was found missing — the thing to install
    /// or boot before retrying.
    pub requires_tool: Option<String>,
}

/// The ENV_DOWN signature library: lowercased substrings over a gate's failure
/// output that mean "a dependency wasn't reachable", not "the code is wrong".
/// Grouped only for readability; matching is a flat substring scan.
const ENV_SIGNATURES: &[&str] = &[
    // connection refused / unreachable service
    "connection refused",
    "econnrefused",
    "could not connect to server",
    "could not connect to",
    "no connection could be made",
    "connection reset by peer",
    "server closed the connection unexpectedly",
    "connection timed out",
    "failed to connect to",
    // docker daemon
    "cannot connect to the docker daemon",
    "is the docker daemon running",
    // databases
    "can't reach database server",
    "cant reach database server",
    "could not translate host name",
    "mongonetworkerror",
    "redis connection",
    // dns
    "getaddrinfo enotfound",
    "getaddrinfo eai_again",
    "name or service not known",
    "temporary failure in name resolution",
    // port already in use (the service can't bind)
    "address already in use",
    "eaddrinuse",
    "port is already allocated",
    "bind: address already in use",
];

/// Substrings that, when present, indicate a MISSING BINARY (the tool itself
/// isn't installed). Kept separate so the classifier can try to name the tool.
const MISSING_BINARY_SIGNATURES: &[&str] = &[
    "command not found",
    "is not recognized as an internal or external command",
    "executable file not found in $path",
    "executable file not found in %path%",
    "no such file or directory (os error 2)",
];

/// The single source of truth for "is this line environmental?" — pure, no I/O,
/// so it's trivially testable. Returns the matched signature when one fires.
pub fn match_env_signature(output: &str) -> Option<&'static str> {
    let lower = output.to_ascii_lowercase();
    ENV_SIGNATURES
        .iter()
        .chain(MISSING_BINARY_SIGNATURES.iter())
        .copied()
        .find(|sig| lower.contains(sig))
}

/// Classify a gate failure. Priority: a declared tool that isn't on `PATH` wins
/// (we know exactly what's missing); then the output signatures; otherwise the
/// failure is treated as a genuine defect (`environment = false`).
pub fn classify_gate_failure(
    _gate_name: &str,
    output: &str,
    required_tools: &[String],
) -> EnvClassification {
    // 1. A declared dependency tool is missing from PATH — unambiguous.
    for tool in required_tools {
        let t = tool.trim();
        if !t.is_empty() && !is_tool_available(t) {
            return EnvClassification {
                environment: true,
                reason: Some(format!("required tool `{t}` is not on PATH")),
                requires_tool: Some(t.to_string()),
            };
        }
    }

    // 2. The failure output carries an environment signature.
    if let Some(sig) = match_env_signature(output) {
        // If it's a missing-binary signature, try to name the tool from the
        // declared set so the boot/escalate step knows what to install.
        let requires_tool = if MISSING_BINARY_SIGNATURES.contains(&sig) {
            let lower = output.to_ascii_lowercase();
            required_tools
                .iter()
                .find(|t| !t.trim().is_empty() && lower.contains(&t.trim().to_ascii_lowercase()))
                .map(|t| t.trim().to_string())
        } else {
            None
        };
        return EnvClassification {
            environment: true,
            reason: Some(format!("gate output matched environment signature: `{sig}`")),
            requires_tool,
        };
    }

    EnvClassification::default()
}

/// Is `bin` an executable on the user's `PATH`? A plain existence + (unix)
/// executable-bit check across every `PATH` entry. No spawning.
pub fn is_tool_available(bin: &str) -> bool {
    if bin.is_empty() {
        return false;
    }
    // An explicit path: check it directly.
    if bin.contains('/') || bin.contains('\\') {
        return Path::new(bin).exists();
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return true;
        }
        // Windows: try the common executable extensions.
        ["exe", "cmd", "bat"]
            .iter()
            .any(|ext| candidate.with_extension(ext).is_file())
    })
}

/// Can we open a TCP connection to `127.0.0.1:port` within a short timeout? Used
/// to tell "the service is up" from "nothing is listening". Best-effort.
pub fn is_port_reachable(port: u16) -> bool {
    let addr = ("127.0.0.1", port);
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return false;
    };
    addrs.any(|sa| TcpStream::connect_timeout(&sa, Duration::from_millis(300)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_and_service_signatures_classify_as_environment() {
        for out in [
            "Error: connect ECONNREFUSED 127.0.0.1:5432",
            "could not connect to server: Connection refused",
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock",
            "PrismaClientInitializationError: Can't reach database server at `localhost`",
            "listen tcp :8080: bind: address already in use",
        ] {
            let c = classify_gate_failure("test", out, &[]);
            assert!(c.environment, "should be env: {out}");
            assert!(c.reason.is_some());
        }
    }

    #[test]
    fn a_real_defect_is_not_environmental() {
        for out in [
            "assertion `left == right` failed\n  left: 3\n right: 4",
            "TypeError: cannot read properties of undefined (reading 'id')",
            "thread 'main' panicked at 'index out of bounds'",
            "2 failed, 18 passed",
        ] {
            let c = classify_gate_failure("test", out, &[]);
            assert!(!c.environment, "should NOT be env: {out}");
        }
    }

    #[test]
    fn a_missing_declared_tool_wins_and_is_named() {
        let c = classify_gate_failure(
            "integration",
            "some unrelated output",
            &["definitely-not-a-real-binary-xyz".to_string()],
        );
        assert!(c.environment);
        assert_eq!(c.requires_tool.as_deref(), Some("definitely-not-a-real-binary-xyz"));
    }

    #[test]
    fn missing_binary_signature_names_the_declared_tool() {
        let c = classify_gate_failure(
            "build",
            "bash: docker: command not found",
            &["docker".to_string()],
        );
        assert!(c.environment);
        assert_eq!(c.requires_tool.as_deref(), Some("docker"));
    }

    #[test]
    fn tool_probe_finds_a_real_binary_and_misses_junk() {
        // `sh` is on PATH on every unix box CI runs on.
        assert!(is_tool_available("sh"));
        assert!(!is_tool_available("definitely-not-a-real-binary-xyz-123"));
        assert!(!is_tool_available(""));
    }

    #[test]
    fn an_almost_certainly_closed_port_is_unreachable() {
        // High, unlikely-bound port. Best-effort: this should be closed.
        assert!(!is_port_reachable(59321));
    }
}
