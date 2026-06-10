//! Production [`HttpTransport`] for the server-side token exchange.
//!
//! darkrun-vcs keeps every network path behind the [`HttpTransport`] seam and
//! ships only a [`MockTransport`](darkrun_vcs::MockTransport) for tests. The
//! live server needs a real client to reach the provider token endpoints, so we
//! adapt `reqwest`'s blocking client to the seam here. The exchange itself is
//! synchronous (the seam is), and axum handlers run it on the blocking pool.
//!
//! Keeping this adapter in the binary crate is deliberate: the secret-handling
//! provider code stays transport-agnostic and offline-testable.

use darkrun_vcs::{HttpRequest, HttpResponse, HttpTransport, Method, Result, VcsError};

/// A `reqwest`-backed [`HttpTransport`].
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    /// Build a transport with a sensible default client.
    ///
    /// Returns a transport error if the client cannot be constructed (e.g. the
    /// TLS backend fails to initialize).
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("darkrun-web")
            .build()
            .map_err(|e| VcsError::Transport(format!("building http client: {e}")))?;
        Ok(Self { client })
    }
}

impl HttpTransport for ReqwestTransport {
    #[cfg(not(tarpaulin_include))] // real blocking HTTP via reqwest — irreducible network I/O
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
        };

        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let response = builder
            .send()
            .map_err(|e| VcsError::Transport(format!("request to {}: {e}", request.url)))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .map_err(|e| VcsError::Transport(format!("reading response body: {e}")))?
            .to_vec();

        Ok(HttpResponse::new(status, body))
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn execute_builds_the_request_and_surfaces_a_connection_error() {
        let t = ReqwestTransport::new().expect("client builds");
        // Port 1 on loopback refuses fast — exercises method/header/body wiring
        // and the send-error path without a live server.
        let req = HttpRequest::post("http://127.0.0.1:1/x")
            .header("authorization", "Bearer t")
            .raw_body(vec![1, 2, 3]);
        assert!(t.execute(req).is_err());
        let get = HttpRequest::get("http://127.0.0.1:1/y");
        assert!(t.execute(get).is_err());
    }
}
