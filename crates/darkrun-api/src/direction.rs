//! Design-direction decision endpoint — `POST /direction/:id/select`.
//!
//! The operator gives a design DIRECTION by choosing one of the generated
//! archetypes and (optionally) annotating it — dropping pins on the preview,
//! attaching a captured screenshot reference, and leaving comments. The chosen
//! archetype id plus its annotations are the decision.
//!
//! Picker sessions reuse the same module for their (much simpler) select body.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::DirectionAnnotations;

/// Request body for `POST /direction/:id/select` — the operator's design
/// decision: which archetype they chose plus optional annotations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DirectionSelectRequest {
    /// The id of the chosen archetype (must match one of the session's
    /// `archetypes[].id`).
    pub archetype: String,
    /// Annotations on the chosen direction — pins, screenshot ref, comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<DirectionAnnotations>,
}

/// Response body for `POST /direction/:id/select`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DirectionSelectResponse {
    /// Always `true` on success.
    pub ok: bool,
    /// The chosen archetype id, echoed back.
    pub archetype: String,
}

/// Request body for `POST /picker/:id/select`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickerSelectRequest {
    /// The id of the option to select — must match one of the session's
    /// options.
    pub id: String,
}

/// Response body for `POST /picker/:id/select`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickerSelectResponse {
    /// Always `true` on success.
    pub ok: bool,
    /// The selected option id, echoed back.
    pub id: String,
}
