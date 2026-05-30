//! Question-answer endpoint — `POST /question/:id/answer`.
//!
//! The wire schema for submitting an answer to a VISUAL QUESTION session: the
//! ids of the selected options plus an optional free-text note, plus the
//! success envelope echoing the recorded answer back.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::QuestionAnnotations;
use crate::session::QuestionAnswer;

/// Request body for `POST /question/:id/answer`.
///
/// A visual question is a single prompt with a list of options; the answer is
/// the set of selected option ids (one for single-select, many for
/// multi-select) plus an optional free-text elaboration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct QuestionAnswerRequest {
    /// The option ids the operator selected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    /// Optional free-text elaboration / "other" input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Optional annotation bundle (pins / screenshots over the reference
    /// images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<QuestionAnnotations>,
}

impl QuestionAnswerRequest {
    /// Project the request into the stored [`QuestionAnswer`] shape carried on
    /// the session payload.
    pub fn to_answer(&self) -> QuestionAnswer {
        QuestionAnswer {
            selected: self.selected.clone(),
            text: self.text.clone(),
        }
    }
}

/// Response body for `POST /question/:id/answer`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QuestionAnswerResponse {
    /// Always `true` on success.
    pub ok: bool,
    /// The recorded answer, echoed back.
    pub answer: QuestionAnswer,
}
