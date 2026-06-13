//! Session payloads — the discriminated union the desktop app reads on the
//! wire from `GET /api/session/:id`.
//!
//! Expressed in the factory vocabulary and tagged on `session_type`
//! (`review | question | direction | picker | view`). Opaque parser output
//! (parsed run/unit/criteria structures built from markdown) is carried as raw
//! [`serde_json::Value`]s rather than schematized here.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{GateType, ReviewAnnotations, SessionStatus};
use crate::proof::Proof;

/// The fixed phase taxonomy a Station walks (mirrors
/// `darkrun_core::domain::StationPhase`, kept local to stay dependency-light).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// Specify: Explore + Decompose into Units.
    Spec,
    /// Review the spec before manufacturing.
    Review,
    /// Manufacture: the Pass-loop over Units.
    Manufacture,
    /// Audit the output against the spec AND run the quality checks / tests
    /// (the old `Tests` phase folded in here).
    Audit,
    /// Reflect: autonomous retrospective feeding the run-level reflections.
    Reflect,
    /// The Checkpoint gate fires.
    Checkpoint,
}

/// Status of one milestone in a station's granular progress track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneStatus {
    /// Finished.
    Done,
    /// In flight.
    Active,
    /// Not started.
    Pending,
}

/// One ordered milestone in a station's granular progress track.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProgressMilestone {
    /// Stable milestone key (e.g. `review:spec`, `manufacture`).
    pub key: String,
    /// Display label.
    pub label: String,
    /// Whether this milestone is done, active, or pending.
    pub status: MilestoneStatus,
}

/// Per-station status snapshot surfaced in a review payload.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StationStateInfo {
    /// The station name.
    pub station: String,
    /// Whether the station's work has merged into the run's main line — the
    /// only authoritative predicate; the rest are display shims.
    pub merged_into_main: bool,
    /// Lifecycle status (display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Current phase (display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// When the station started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// When the station completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// When the checkpoint gate was entered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_entered_at: Option<String>,
    /// The checkpoint outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_outcome: Option<String>,
}

/// A named knowledge file surfaced in a review.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeFile {
    /// File name.
    pub name: String,
    /// File content.
    pub content: String,
}

/// A per-station artifact (name + content).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StationArtifact {
    /// The station the artifact belongs to.
    pub station: String,
    /// File name.
    pub name: String,
    /// File content.
    pub content: String,
}

/// The render kind of an output artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputArtifactType {
    /// Markdown source.
    Markdown,
    /// Raw HTML.
    Html,
    /// An image.
    Image,
    /// A video.
    Video,
    /// Source code (highlighted by `language`).
    Code,
    /// An opaque file.
    File,
}

/// One declared output deliverable surfaced in a review.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutputArtifact {
    /// The station the artifact belongs to (empty string = run scope).
    pub station: String,
    /// File name.
    pub name: String,
    /// How to render the artifact.
    #[serde(rename = "type")]
    pub artifact_type: OutputArtifactType,
    /// Highlight language id for `code`-type artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Subdirectory grouping under the station dir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    /// Inline content, when small enough to embed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Fetch URL (already prefixed + session-scoped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    /// Original run-dir-relative path, for "declared by" lookups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_relative_path: Option<String>,
}

/// The render kind of a per-unit output preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitOutputType {
    /// Markdown source.
    Markdown,
    /// Raw HTML.
    Html,
    /// An image.
    Image,
    /// An opaque file.
    File,
}

/// One per-unit output preview entry — one per path a Unit declared.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnitOutputPreview {
    /// Declared output path.
    pub path: String,
    /// Display name.
    pub name: String,
    /// How to render the preview.
    #[serde(rename = "type")]
    pub output_type: UnitOutputType,
    /// Fetch URL.
    pub url: String,
    /// Inline preview body (markdown source or raw HTML).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_body: Option<String>,
    /// Size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Whether the declared output actually exists on disk.
    pub exists: bool,
}

/// A snapshot of a prior review, attached when the current review follows a
/// changes-requested decision.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviousReviewSnapshot {
    /// The prior feedback text.
    pub feedback: String,
    /// When the prior review happened.
    pub reviewed_at: String,
    /// Raw run-document content at the time.
    pub run_raw_content: String,
    /// Raw unit-document content keyed by unit slug.
    pub unit_raw_contents: BTreeMap<String, String>,
}

/// The consequence the Approve button will trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApproveActionKind {
    /// Mark an ad-hoc review done.
    AdHocDone,
    /// Open a pull request.
    OpenPr,
    /// Submit for external review.
    SubmitExternal,
    /// Start the run.
    StartRun,
    /// Start execution.
    StartExecution,
    /// Complete the current station.
    CompleteStation,
    /// Submit the run for review.
    SubmitRunReview,
    /// Complete the run.
    CompleteRun,
    /// Plain approve.
    Approve,
}

/// The server-computed Approve button label + consequence.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApproveAction {
    /// The button label rendered verbatim.
    pub label: String,
    /// The consequence the button triggers.
    pub kind: ApproveActionKind,
}

/// A delivery PR/MR auto-discovered from a published head ref.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveredReviewUrl {
    /// The PR/MR URL.
    pub url: String,
    /// Where the URL was discovered.
    pub source: DiscoveredReviewSource,
    /// The PR/MR number.
    pub pr_number: u64,
    /// The matched HEAD SHA.
    pub matched_sha: String,
}

/// The provenance of an auto-discovered review URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoveredReviewSource {
    /// A GitHub `refs/pull/<n>/head` ref.
    GithubPrRef,
    /// A GitLab `refs/merge-requests/<n>/head` ref.
    GitlabMrRef,
}

/// A decision the desktop app submitted while no await was open. Drained on the
/// next `darkrun_await_gate` entry.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PendingDecision {
    /// The submitted decision.
    pub decision: String,
    /// The accompanying feedback.
    pub feedback: String,
    /// When it was submitted.
    pub submitted_at: String,
}

/// Unified current-state snapshot — "where is this run right now?".
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RunCurrentState {
    /// The factory driving the run.
    pub factory: String,
    /// The active station.
    pub station: String,
    /// The active phase, if resolvable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<RunPhase>,
    /// The active sub-step, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// Elaborate-phase signals not yet satisfied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_signals: Vec<String>,
    /// Granular milestone track for the active station.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<ProgressMilestone>,
    /// Index of the active (first not-done) milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_index: Option<u32>,
    /// Total milestone count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_total: Option<u32>,
    /// Terminal seal state, present once every station has merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal_status: Option<SealStatus>,
    /// The default branch the work is waiting to land on, when pending seal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awaiting_merge_into: Option<String>,
}

/// Terminal seal state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SealStatus {
    /// The terminal write-lock is stamped.
    Sealed,
    /// Built and signed, but the branch hasn't landed yet.
    PendingSeal,
}

/// The review-session payload (`session_type = "review"`) — the load-bearing
/// variant for phase 1.
///
/// Opaque parsed artifacts (run/unit/criteria structures built from markdown)
/// are carried as raw JSON [`Value`]s — a loose-by-design approach for internal
/// parser output.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ReviewSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Session lifecycle status.
    pub status: SessionStatus,
    /// The run slug under review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_slug: Option<String>,
    /// The run directory on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_dir: Option<String>,
    /// The checkpoint gate kind that opened this review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_type: Option<GateType>,
    /// The artifact this review targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The recorded decision, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// Reviewer free-text feedback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// Annotations attached to the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ReviewAnnotations>,
    /// Opaque parsed run document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Value>,
    /// Opaque parsed unit documents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<Value>,
    /// Opaque parsed completion criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<Value>,
    /// Optional mermaid diagram source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mermaid: Option<String>,
    /// Per-station status snapshots, in FACTORY ORDER (not alphabetical) — the
    /// desktop renders the station strip in this order, so it's a Vec, not a map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub station_states: Vec<StationStateInfo>,
    /// Per-station definition summaries, keyed by station name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub station_summaries: BTreeMap<String, String>,
    /// Per-station user-facing PRE-execution briefs ("what I'm going to do"),
    /// keyed by station name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub station_briefs: BTreeMap<String, String>,
    /// Per-station closing OUTCOMES ("what the station actually produced"),
    /// keyed by station name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub station_outcomes: BTreeMap<String, String>,
    /// Per-station worker observations, keyed by station name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub station_observations: BTreeMap<String, String>,
    /// Per-station elaboration narratives, keyed by station name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub station_elaborations: BTreeMap<String, String>,
    /// Per-station milestone tracks, keyed by station name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub station_milestones: BTreeMap<String, Vec<ProgressMilestone>>,
    /// The run-scope synthesized reflection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<String>,
    /// The unified current-state snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_state: Option<RunCurrentState>,
    /// Knowledge files surfaced in the review.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_files: Vec<KnowledgeFile>,
    /// Per-station artifacts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub station_artifacts: Vec<StationArtifact>,
    /// Declared output deliverables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_artifacts: Vec<OutputArtifact>,
    /// Stray station-scope files not declared by any unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_files: Vec<OutputArtifact>,
    /// Stray run-root files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_other_files: Vec<OutputArtifact>,
    /// Per-unit output previews, keyed by unit slug.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unit_outputs: BTreeMap<String, Vec<UnitOutputPreview>>,
    /// Inverse of `unit_outputs`: output path -> declaring unit slugs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output_declared_by: BTreeMap<String, Vec<String>>,
    /// The prior review snapshot, on a changes-requested follow-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_review: Option<PreviousReviewSnapshot>,
    /// An auto-discovered delivery PR/MR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_review_url: Option<DiscoveredReviewUrl>,
    /// Whether this is an ad-hoc (non-gate) review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ad_hoc: Option<bool>,
    /// The station the review was opened against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// Where in the lifecycle this gate fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_context: Option<String>,
    /// The station that begins after approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_station: Option<String>,
    /// The phase that begins after approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_phase: Option<String>,
    /// The Approve button's computed label + consequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve_action: Option<ApproveAction>,
    /// True when a `darkrun_await_gate` call is currently blocked on this
    /// session (drives the Approve button's enabled state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub await_active: Option<bool>,
    /// Cumulative number of awaits that have run on this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub await_count: Option<u32>,
    /// A decision queued while no await was open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_decision: Option<PendingDecision>,
    /// When the last await started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_await_started_at: Option<String>,
    /// When the last await ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_await_ended_at: Option<String>,
}

/// One selectable option in a visual question — a single choice the operator
/// can pick, optionally backed by a generated image and a description.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QuestionOption {
    /// Canonical id echoed back in the answer's `selected[]`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional generated-image URL (a mockup / design option to pick among).
    /// When `image_url_light` is also set, this is the **dark-theme** variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Optional **light-theme** variant of `image_url`. When present, the option
    /// is multi-theme: the desktop/web view shows whichever matches the active
    /// theme. Absent → `image_url` is theme-neutral and always shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url_light: Option<String>,
    /// Optional longer description rendered under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The operator's answer to a visual question — the chosen option ids plus an
/// optional free-text note.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct QuestionAnswer {
    /// The option ids the operator selected (one for single-select, many for
    /// multi-select).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    /// Optional free-text elaboration / "other" input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// The question-session payload (`session_type = "question"`) — a VISUAL
/// QUESTION the agent poses mid-run: a prompt plus a list of options (each an
/// optionally-image-backed choice) the operator picks among.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct QuestionSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Session lifecycle status.
    pub status: SessionStatus,
    /// The run this question belongs to — lets the engine persist it under the
    /// run and surface it on the run's desktop channel (mirrors Direction /
    /// Picker, which already carry this).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_slug: Option<String>,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The question prompt rendered above the options.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    /// Optional markdown context preamble.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The selectable options (image-backed design choices, or plain choices).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<QuestionOption>,
    /// Whether more than one option may be selected.
    #[serde(default)]
    pub multi_select: bool,
    /// The recorded answer, once submitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<QuestionAnswer>,
    /// Reference image URLs the question annotates (e.g. the surface under
    /// discussion), distinct from per-option images.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_urls: Vec<String>,
}

/// One design ARCHETYPE card in a direction session — a named design direction,
/// always backed by a generated preview image.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DirectionArchetype {
    /// Canonical id echoed back as `chosen_archetype`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Generated preview-image URL for this archetype. When `image_url_light` is
    /// also set, this is the **dark-theme** variant.
    pub image_url: String,
    /// Optional **light-theme** variant of `image_url`. When present, the
    /// archetype is multi-theme: the view shows whichever matches the active
    /// theme. Absent → `image_url` is theme-neutral and always shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url_light: Option<String>,
    /// Description of the design direction this archetype represents.
    pub description: String,
}

/// A pin dropped on a direction preview at a relative coordinate.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DirectionPin {
    /// X coordinate (0..1 relative to the preview width).
    pub x: f64,
    /// Y coordinate (0..1 relative to the preview height).
    pub y: f64,
    /// The note attached to the pin.
    pub note: String,
}

/// Annotations the operator attaches when giving a design direction — pins on
/// the chosen archetype, an optional captured-screenshot reference, and a list
/// of free-text comments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DirectionAnnotations {
    /// Pin annotations on the preview.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<DirectionPin>,
    /// Optional reference to a captured screenshot — a `data:image/...;base64`
    /// URL or a server-relative artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Free-text comments on the direction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<String>,
}

/// The direction-session payload (`session_type = "direction"`) — a DESIGN
/// DIRECTION the agent asks for: a prompt plus design archetypes (each an
/// image-backed direction), the chosen archetype id, and annotations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DirectionSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Session lifecycle status.
    pub status: SessionStatus,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The run slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_slug: Option<String>,
    /// The prompt rendered above the archetype cards.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    /// Optional markdown preamble above the archetype cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The design archetypes to choose between.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archetypes: Vec<DirectionArchetype>,
    /// The id of the archetype the operator chose, once decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_archetype: Option<String>,
    /// Annotations attached to the chosen direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<DirectionAnnotations>,
}

/// The kind of selection a picker session blocks on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PickerKind {
    /// Pick a factory.
    Factory,
    /// Pick a sizing mode.
    Mode,
    /// Pick a station.
    Station,
    /// Confirm a destructive action.
    Confirm,
    /// Enter a URL.
    UrlInput,
}

/// One option in a picker session.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickerOption {
    /// Canonical id echoed back on selection.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the option is hidden behind a "show all" expansion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<bool>,
}

/// A saved picker selection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickerSelection {
    /// The selected option id.
    pub id: String,
}

/// The picker-session payload (`session_type = "picker"`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickerSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Session lifecycle status.
    pub status: SessionStatus,
    /// The run slug, if scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_slug: Option<String>,
    /// The kind of selection.
    pub kind: PickerKind,
    /// Title.
    pub title: String,
    /// Prompt text.
    pub prompt: String,
    /// Selectable options.
    pub options: Vec<PickerOption>,
    /// The saved selection, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<PickerSelection>,
}

/// The mode a view session opened in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// A read-only artifact browser.
    Viewer,
    /// A spawned project dev server.
    Boot,
}

/// The render kind of one browsable output artifact in a view session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewArtifactKind {
    /// An opaque file.
    File,
    /// An image.
    Image,
    /// A captured screenshot (the visual-review target).
    Screenshot,
    /// Markdown source.
    Markdown,
    /// A JSON document.
    Json,
}

/// One browsable output artifact in a view session's artifact browser.
///
/// The view session is an ARTIFACT BROWSER: it lists the run's output
/// deliverables so the operator can inspect them. Each entry carries a stable
/// id, its run-relative path, a render kind, a display label, and optional
/// fetch / thumbnail URLs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ViewArtifact {
    /// Stable id (referenced by an output-annotation review's `artifact_id`).
    pub id: String,
    /// Run-relative path of the artifact on disk.
    pub path: String,
    /// How to render the artifact.
    pub kind: ViewArtifactKind,
    /// Display label.
    pub label: String,
    /// Optional thumbnail URL (a small preview for the browser grid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    /// Optional full fetch URL (session-scoped, already prefixed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The scope a view session browses — which slice of the run's outputs it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewScope {
    /// Every output the run produced.
    #[default]
    Run,
    /// Only the outputs of one station.
    Station,
    /// A single artifact deep-link.
    Artifact,
}

/// The view-session payload (`session_type = "view"`) — a non-blocking ARTIFACT
/// BROWSER opened by `darkrun_view`.
///
/// The centerpiece is [`artifacts`](ViewSessionPayload::artifacts): the list of
/// output deliverables the operator browses, narrowed by
/// [`scope`](ViewSessionPayload::scope). The viewer/boot mode + boot fields are
/// retained for the dev-server boot path.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ViewSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Whether the view is open or closed.
    pub status: ViewStatus,
    /// The run slug being viewed.
    pub run_slug: String,
    /// What slice of the run's outputs this view browses.
    #[serde(default)]
    pub scope: ViewScope,
    /// The output artifacts available to browse.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ViewArtifact>,
    /// Optional factory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factory: Option<String>,
    /// Optional station narrowing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// Optional artifact deep-link (the `id` of the focused artifact).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// Viewer or boot mode.
    pub mode: ViewMode,
    /// Boot-mode dev-server port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_port: Option<u16>,
    /// Boot-mode dev-server command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_command: Option<String>,
}

impl ViewSessionPayload {
    /// Look up a browsable artifact by its stable id.
    pub fn artifact_by_id(&self, id: &str) -> Option<&ViewArtifact> {
        self.artifacts.iter().find(|a| a.id == id)
    }
}

/// The open/closed status of a view session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewStatus {
    /// Open.
    Open,
    /// Closed.
    Closed,
}

/// A pin dropped on an OUTPUT screenshot during visual review — reuses the
/// pin annotation model (relative coordinate + note) over an artifact's
/// rendered screenshot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VisualReviewPin {
    /// X coordinate (0..1 relative to the screenshot width).
    pub x: f64,
    /// Y coordinate (0..1 relative to the screenshot height).
    pub y: f64,
    /// The note attached to the pin.
    pub note: String,
}

/// The annotation bundle a visual review produces over an output screenshot:
/// pins on the rendered surface plus free-text comments. This is the FEEDBACK
/// the operator gives by annotating the OUTPUT (distinct from a pre-build
/// design direction).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct VisualReviewAnnotations {
    /// Pin annotations on the output screenshot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<VisualReviewPin>,
    /// Free-text comments on the output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<String>,
}

impl VisualReviewAnnotations {
    /// Whether the bundle carries any feedback at all (a pin or a comment).
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty() && self.comments.is_empty()
    }
}

/// The visual-review session payload (`session_type = "visual_review"`) — a
/// VISUAL REVIEW of an OUTPUT screenshot.
///
/// The operator annotates the rendered screenshot of one of the run's output
/// artifacts (pins + comments over the captured surface), producing feedback.
/// Reuses the pin annotation model from the design-direction flow, but points
/// at a *produced* artifact rather than a pre-build design preview.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct VisualReviewSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Session lifecycle status.
    pub status: SessionStatus,
    /// The run slug whose output is under review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_slug: Option<String>,
    /// The station that produced the output, if scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// The id of the [`ViewArtifact`] being annotated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    /// Run-relative path of the artifact being annotated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    /// URL of the output screenshot the operator annotates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_url: Option<String>,
    /// Optional prompt rendered above the screenshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The annotations the operator produced, once submitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<VisualReviewAnnotations>,
}

/// The proof-session payload (`session_type = "proof"`) — the Prove station's
/// objective evidence surfaced as a session.
///
/// Carries the run's [`Proof`] (surface-tagged web vitals + audits, or bench
/// percentiles + throughput) so the desktop app can render the NUMBERS.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProofSessionPayload {
    /// The session id.
    pub session_id: String,
    /// Session lifecycle status.
    pub status: SessionStatus,
    /// The run slug the proof belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_slug: Option<String>,
    /// The station the proof was measured at, if scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// The objective evidence.
    pub proof: Proof,
}

/// The session payload returned by `GET /api/session/:id`, discriminated on
/// `session_type`.
///
/// The `Review` variant is intentionally the largest (it is the load-bearing
/// phase-1 payload); the size disparity is accepted to keep the public
/// constructor ergonomic for every caller.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "session_type", rename_all = "snake_case")]
pub enum SessionPayload {
    /// A checkpoint review (the load-bearing variant).
    Review(ReviewSessionPayload),
    /// A multi-question prompt.
    Question(QuestionSessionPayload),
    /// A design-direction selection.
    Direction(DirectionSessionPayload),
    /// A blocking picker selection.
    Picker(PickerSessionPayload),
    /// A non-blocking artifact browser.
    View(ViewSessionPayload),
    /// A visual review of an output screenshot (pin annotations -> feedback).
    VisualReview(VisualReviewSessionPayload),
    /// Objective evidence from the Prove station.
    Proof(ProofSessionPayload),
}

impl SessionPayload {
    /// The discriminator string (`session_type`) for this payload.
    pub fn session_type(&self) -> &'static str {
        match self {
            SessionPayload::Review(_) => "review",
            SessionPayload::Question(_) => "question",
            SessionPayload::Direction(_) => "direction",
            SessionPayload::Picker(_) => "picker",
            SessionPayload::View(_) => "view",
            SessionPayload::VisualReview(_) => "visual_review",
            SessionPayload::Proof(_) => "proof",
        }
    }

    /// The session id, regardless of variant.
    pub fn session_id(&self) -> &str {
        match self {
            SessionPayload::Review(p) => &p.session_id,
            SessionPayload::Question(p) => &p.session_id,
            SessionPayload::Direction(p) => &p.session_id,
            SessionPayload::Picker(p) => &p.session_id,
            SessionPayload::View(p) => &p.session_id,
            SessionPayload::VisualReview(p) => &p.session_id,
            SessionPayload::Proof(p) => &p.session_id,
        }
    }

    /// An INTERACTIVE operator session that the engine persists per-run and
    /// surfaces on the run's desktop channel: a Question, Direction, or Picker.
    /// Returns the run it belongs to (when known) and whether it is still OPEN
    /// (awaiting the operator) vs already answered/decided. Other variants
    /// (review/view/proof/visual_review) return `None` — they are derived from
    /// run state, not persisted as standalone operator prompts.
    pub fn interactive(&self) -> Option<InteractiveSession<'_>> {
        let (run, open) = match self {
            SessionPayload::Question(p) => {
                (p.run_slug.as_deref(), p.status == SessionStatus::Pending)
            }
            SessionPayload::Direction(p) => {
                (p.run_slug.as_deref(), p.status == SessionStatus::Pending)
            }
            SessionPayload::Picker(p) => {
                (p.run_slug.as_deref(), p.status == SessionStatus::Pending)
            }
            _ => return None,
        };
        Some(InteractiveSession { run, open })
    }
}

/// The run + open-state of an [`SessionPayload::interactive`] session.
#[derive(Debug, Clone, Copy)]
pub struct InteractiveSession<'a> {
    /// The run the session belongs to, when the payload records one.
    pub run: Option<&'a str>,
    /// Whether the session is still awaiting the operator.
    pub open: bool,
}
