//! Route descriptor table — the canonical enumeration of HTTP routes and the
//! WebSocket upgrade path the `darkrun-http` server handles.
//!
//! Built around `.darkrun` paths and the factory vocabulary. Dependency-light:
//! a static list of descriptors with path builders.

/// An HTTP method, plus the pseudo-method `Ws` for the WebSocket upgrade path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// GET.
    Get,
    /// HEAD.
    Head,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// DELETE.
    Delete,
    /// WebSocket upgrade.
    Ws,
}

/// A single route descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSpec {
    /// The HTTP method (or `Ws`).
    pub method: HttpMethod,
    /// RFC-6570-style path template, e.g. `/api/session/{id}`.
    pub path_template: &'static str,
    /// Unique operation id surfaced into any emitted OpenAPI document.
    pub operation_id: &'static str,
    /// Short human-readable summary.
    pub summary: &'static str,
    /// Tag grouping.
    pub tag: &'static str,
}

/// Path builders so callers don't hand-format templates.
pub mod paths {
    /// `GET /api/session/{id}`.
    pub fn session(id: &str) -> String {
        format!("/api/session/{id}")
    }
    /// `HEAD /api/session/{id}/heartbeat`.
    pub fn session_heartbeat(id: &str) -> String {
        format!("/api/session/{id}/heartbeat")
    }
    /// `POST /review/{id}/decide`.
    pub fn review_decide(id: &str) -> String {
        format!("/review/{id}/decide")
    }
    /// `GET /api/review/current`.
    pub fn review_current() -> String {
        "/api/review/current".to_string()
    }
    /// `GET /api/runs`.
    pub fn runs() -> String {
        "/api/runs".to_string()
    }
    /// `GET /api/runs/{slug}`.
    pub fn run_detail(slug: &str) -> String {
        format!("/api/runs/{slug}")
    }
    /// `POST /question/{id}/answer`.
    pub fn question_answer(id: &str) -> String {
        format!("/question/{id}/answer")
    }
    /// `POST /direction/{id}/select`.
    pub fn direction_select(id: &str) -> String {
        format!("/direction/{id}/select")
    }
    /// `POST /picker/{id}/select`.
    pub fn picker_select(id: &str) -> String {
        format!("/picker/{id}/select")
    }
    /// `POST /visual-review/{id}/annotate`.
    pub fn visual_review_annotate(id: &str) -> String {
        format!("/visual-review/{id}/annotate")
    }
    /// `GET`/`POST` `/api/proof/{run}`.
    pub fn proof(run: &str) -> String {
        format!("/api/proof/{run}")
    }
    /// `POST /api/advance/{id}`.
    pub fn advance(id: &str) -> String {
        format!("/api/advance/{id}")
    }
    /// `GET`/`POST` `/api/feedback/{run}/{station}`.
    pub fn feedback_list(run: &str, station: &str) -> String {
        format!("/api/feedback/{run}/{station}")
    }
    /// `PUT`/`DELETE` `/api/feedback/{run}/{station}/{id}`.
    pub fn feedback_item(run: &str, station: &str, id: &str) -> String {
        format!("/api/feedback/{run}/{station}/{id}")
    }
    /// `POST /api/feedback/{run}/{station}/{id}/replies`.
    pub fn feedback_replies(run: &str, station: &str, id: &str) -> String {
        format!("/api/feedback/{run}/{station}/{id}/replies")
    }
    /// `GET /health`.
    pub fn health() -> String {
        "/health".to_string()
    }
    /// `GET /ws/session/{id}`.
    pub fn ws_session(id: &str) -> String {
        format!("/ws/session/{id}")
    }
}

/// The canonical route table for the darkrun engine's HTTP/WS surface.
pub const ROUTES: &[RouteSpec] = &[
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/api/session/{sessionId}",
        operation_id: "getSession",
        summary: "Return session JSON for the desktop app to render.",
        tag: "session",
    },
    RouteSpec {
        method: HttpMethod::Head,
        path_template: "/api/session/{sessionId}/heartbeat",
        operation_id: "sessionHeartbeat",
        summary: "Client presence ping. 200 if the session exists, 404 otherwise.",
        tag: "session",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/review/{sessionId}/decide",
        operation_id: "postReviewDecide",
        summary: "Submit a review decision (approved | changes_requested).",
        tag: "review",
    },
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/api/review/current",
        operation_id: "getReviewCurrent",
        summary: "Compact run-state summary: active station, units, feedback counts.",
        tag: "review",
    },
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/api/runs",
        operation_id: "listRuns",
        summary: "List the project's runs as compact summaries, sorted by slug.",
        tag: "runs",
    },
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/api/runs/{slug}",
        operation_id: "getRun",
        summary: "Return a run's detail: stations, units on the active station, and phase.",
        tag: "runs",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/question/{sessionId}/answer",
        operation_id: "postQuestionAnswer",
        summary: "Submit the answer (selected option ids + optional text) to a visual question.",
        tag: "question",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/direction/{sessionId}/select",
        operation_id: "postDirectionSelect",
        summary: "Choose a design archetype and annotate it (the design direction decision).",
        tag: "direction",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/picker/{sessionId}/select",
        operation_id: "postPickerSelect",
        summary: "Choose an option in a blocking picker session.",
        tag: "picker",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/visual-review/{sessionId}/annotate",
        operation_id: "postVisualReviewAnnotate",
        summary: "Annotate an output screenshot (pins + comments) -> feedback.",
        tag: "visual-review",
    },
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/api/proof/{run}",
        operation_id: "getProof",
        summary: "Return a run's attached objective-evidence proof.",
        tag: "proof",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/api/proof/{run}",
        operation_id: "attachProof",
        summary: "Attach an objective-evidence proof to a run.",
        tag: "proof",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/api/advance/{sessionId}",
        operation_id: "postAdvance",
        summary: "Wake signal: walk the run past a user checkpoint on the next tick.",
        tag: "review",
    },
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/api/feedback/{run}/{station}",
        operation_id: "listFeedback",
        summary: "List feedback items for a run's station.",
        tag: "feedback",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/api/feedback/{run}/{station}",
        operation_id: "createFeedback",
        summary: "Create a new feedback item in a run's station.",
        tag: "feedback",
    },
    RouteSpec {
        method: HttpMethod::Put,
        path_template: "/api/feedback/{run}/{station}/{id}",
        operation_id: "updateFeedback",
        summary: "Update status or closed_by on a feedback item.",
        tag: "feedback",
    },
    RouteSpec {
        method: HttpMethod::Delete,
        path_template: "/api/feedback/{run}/{station}/{id}",
        operation_id: "deleteFeedback",
        summary: "Delete a feedback item (blocks open items via 409).",
        tag: "feedback",
    },
    RouteSpec {
        method: HttpMethod::Post,
        path_template: "/api/feedback/{run}/{station}/{id}/replies",
        operation_id: "createFeedbackReply",
        summary: "Append a reply to a feedback thread; optionally close as answered.",
        tag: "feedback",
    },
    RouteSpec {
        method: HttpMethod::Get,
        path_template: "/health",
        operation_id: "getHealth",
        summary: "Readiness probe. 200 once listening, 503 while starting.",
        tag: "health",
    },
    RouteSpec {
        method: HttpMethod::Ws,
        path_template: "/ws/session/{sessionId}",
        operation_id: "upgradeSessionWebSocket",
        summary: "WebSocket upgrade for live session updates.",
        tag: "websocket",
    },
];

/// Look up a route descriptor by method + path template.
pub fn find(method: HttpMethod, path_template: &str) -> Option<&'static RouteSpec> {
    ROUTES
        .iter()
        .find(|r| r.method == method && r.path_template == path_template)
}

#[cfg(test)]
mod path_tests {
    use super::paths;

    #[test]
    fn every_path_builder_renders_its_template() {
        assert_eq!(paths::session("s"), "/api/session/s");
        assert_eq!(paths::session_heartbeat("s"), "/api/session/s/heartbeat");
        assert_eq!(paths::review_decide("s"), "/review/s/decide");
        assert_eq!(paths::review_current(), "/api/review/current");
        assert_eq!(paths::runs(), "/api/runs");
        assert_eq!(paths::run_detail("r"), "/api/runs/r");
        assert_eq!(paths::question_answer("s"), "/question/s/answer");
        assert_eq!(paths::direction_select("s"), "/direction/s/select");
        assert_eq!(paths::picker_select("s"), "/picker/s/select");
        assert_eq!(paths::visual_review_annotate("s"), "/visual-review/s/annotate");
        assert_eq!(paths::proof("r"), "/api/proof/r");
        assert_eq!(paths::advance("s"), "/api/advance/s");
        assert_eq!(paths::feedback_list("r", "frame"), "/api/feedback/r/frame");
        assert_eq!(paths::feedback_item("r", "frame", "fb-1"), "/api/feedback/r/frame/fb-1");
        assert_eq!(paths::feedback_replies("r", "frame", "fb-1"), "/api/feedback/r/frame/fb-1/replies");
        assert_eq!(paths::health(), "/health");
        assert_eq!(paths::ws_session("s"), "/ws/session/s");
    }
}
