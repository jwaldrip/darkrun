//! The injectable HTTP transport.
//!
//! Every network-touching path in this crate goes through [`HttpTransport`].
//! Production code wires a real client (e.g. a `reqwest`/`ureq` adapter living
//! in the binary) into the providers; tests wire [`MockTransport`] so the suite
//! is fully offline and deterministic.

use std::collections::BTreeMap;

use crate::error::{Result, VcsError};

/// An HTTP method, kept deliberately tiny — providers only ever issue
/// GET/POST/PUT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// HTTP `GET`.
    Get,
    /// HTTP `POST`.
    Post,
    /// HTTP `PUT`.
    Put,
}

impl Method {
    /// The canonical uppercase wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
        }
    }
}

/// An outbound HTTP request. Header keys are compared case-insensitively by the
/// mock; production adapters should forward them verbatim.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// The request method.
    pub method: Method,
    /// The fully-qualified request URL.
    pub url: String,
    /// Request headers as ordered key/value pairs.
    pub headers: Vec<(String, String)>,
    /// The request body, if any (already serialized to bytes).
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    /// Start a `GET` request to `url`.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// Start a `POST` request to `url`.
    pub fn post(url: impl Into<String>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// Start a `PUT` request to `url`.
    pub fn put(url: impl Into<String>) -> Self {
        Self {
            method: Method::Put,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// Append a header, returning `self` for chaining.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Attach a JSON body, setting `Content-Type: application/json`.
    pub fn json_body(mut self, value: &serde_json::Value) -> Result<Self> {
        let bytes = serde_json::to_vec(value)?;
        self.body = Some(bytes);
        self.headers
            .push(("Content-Type".into(), "application/json".into()));
        Ok(self)
    }

    /// Attach a raw body.
    pub fn raw_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }
}

/// An inbound HTTP response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response body bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Build a response from a status and string body.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    /// Whether the status is in the `2xx` success range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body decoded as UTF-8 (lossy-free; errors on invalid UTF-8).
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| VcsError::Decode(format!("response body was not valid UTF-8: {e}")))
    }

    /// Parse the body as JSON into `T`.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(VcsError::from)
    }
}

/// The injectable transport seam. Implementors perform a single round-trip.
///
/// Kept synchronous on purpose: it keeps the providers free of an async runtime
/// dependency and makes the mock trivial. The CLI's async polling loop wraps a
/// blocking adapter on a worker thread.
pub trait HttpTransport {
    /// Execute `request` and return the response, or a transport-level error.
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse>;
}

/// A recorded exchange for [`MockTransport`].
#[derive(Debug, Clone)]
struct Exchange {
    response: HttpResponse,
}

/// An offline transport for tests.
///
/// Responses are queued per `(METHOD, url)`. Each call pops the next queued
/// response for the matched key and records the request for later assertions.
#[derive(Default)]
pub struct MockTransport {
    queued: std::cell::RefCell<BTreeMap<String, Vec<Exchange>>>,
    recorded: std::cell::RefCell<Vec<HttpRequest>>,
}

impl MockTransport {
    /// Create an empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    fn key(method: Method, url: &str) -> String {
        format!("{} {}", method.as_str(), url)
    }

    /// Queue `response` to be returned for the next `method url` call.
    ///
    /// Calls to the same key are served FIFO, so queueing twice models two
    /// sequential calls (e.g. a token exchange followed by a PR creation).
    pub fn expect(&self, method: Method, url: impl AsRef<str>, response: HttpResponse) -> &Self {
        let key = Self::key(method, url.as_ref());
        self.queued
            .borrow_mut()
            .entry(key)
            .or_default()
            .push(Exchange { response });
        self
    }

    /// The requests this mock has served, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.recorded.borrow().clone()
    }

    /// The single request served so far, panicking if there was not exactly one.
    pub fn single_request(&self) -> HttpRequest {
        let reqs = self.requests();
        assert_eq!(reqs.len(), 1, "expected exactly one request, got {}", reqs.len());
        reqs.into_iter().next().expect("len checked above")
    }
}

impl HttpTransport for MockTransport {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        let key = Self::key(request.method, &request.url);
        self.recorded.borrow_mut().push(request);
        let mut queued = self.queued.borrow_mut();
        match queued.get_mut(&key) {
            Some(slot) if !slot.is_empty() => Ok(slot.remove(0).response),
            _ => Err(VcsError::Transport(format!("no mock response queued for `{key}`"))),
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn request_builders_set_method_headers_and_body() {
        assert_eq!(Method::Get.as_str(), "GET");
        let req = HttpRequest::post("https://x/y")
            .header("authorization", "Bearer t")
            .raw_body(vec![1, 2, 3]);
        assert_eq!(req.body.as_deref(), Some(&[1u8, 2, 3][..]));
        let _ = HttpRequest::get("https://x/z");
    }
}
