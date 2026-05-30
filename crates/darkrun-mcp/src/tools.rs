//! MCP tool surface — the CORE subset that drives a run from `start ->
//! pickup -> station loop`.
//!
//! The orchestrator + state tool handlers, expressed in the factory
//! vocabulary and reduced to:
//! `darkrun_run_start`, `darkrun_run_next`, `darkrun_run_show`,
//! `darkrun_unit_list`, `darkrun_factory_list`, `darkrun_checkpoint_decide`.
//!
//! Each tool validates its input (via schemars-typed structs) and returns a
//! structured JSON result. The manager never runs LLM agents — the tools
//! return the next-action instruction, the caller performs it, then re-ticks.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::schemars::JsonSchema;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde::{Deserialize, Serialize};

use darkrun_core::domain::{FeedbackStatus, Status};
use darkrun_core::StateStore;

use darkrun_http::SessionRegistry;

use crate::factory::{list_factories, resolve_factory};
use crate::position::{checkpoint_decide, run_start, run_tick};
use crate::sessions::{self, ArchetypeSpec, PickerOptionSpec, QuestionOptionSpec};
use crate::{feedback, proof, runs, units};

/// The darkrun MCP server: a manager bound to a repo root, holding the shared
/// in-memory [`SessionRegistry`] that the in-process HTTP/WS server also serves
/// from.
///
/// Durable run state (run.md, state.json, units/, feedback/, proof.json) lives
/// on disk via [`StateStore`]; the EPHEMERAL interactive sessions
/// (question/direction/picker payloads) live only in `sessions` — never on disk.
/// Because the registry is a clonable shared handle, a session a tool handler
/// upserts is immediately visible to the HTTP handlers connected to the bound
/// port.
#[derive(Clone)]
pub struct DarkrunServer {
    repo_root: Arc<PathBuf>,
    sessions: SessionRegistry,
    /// The HTTP/WS port announced to the agent in `instructions`, when the
    /// server co-hosts the in-process HTTP server. `None` for bare unit tests.
    announced_addr: Option<std::net::SocketAddr>,
    /// The active agent harness's capability set — drives tool filtering,
    /// instruction adaptation, and which MCP prompts are bridged. Defaults to
    /// Claude Code (the maximal reference) until [`DarkrunServer::with_harness`].
    caps: Arc<darkrun_harness::Capabilities>,
}

impl DarkrunServer {
    /// Build a server rooted at `repo_root` with a fresh in-memory session
    /// registry (state lives under `<repo_root>/.darkrun`).
    ///
    /// Used by callers that do not co-host the HTTP server (e.g. unit tests).
    /// The in-process `darkrun mcp` host instead builds one shared registry and
    /// passes it via [`DarkrunServer::with_sessions`] so the HTTP/WS server and
    /// the MCP tools observe the same sessions.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self::with_sessions(repo_root, SessionRegistry::new())
    }

    /// Build a server rooted at `repo_root` sharing the given in-memory session
    /// registry with the in-process HTTP/WS server.
    pub fn with_sessions(repo_root: impl Into<PathBuf>, sessions: SessionRegistry) -> Self {
        Self {
            repo_root: Arc::new(repo_root.into()),
            sessions,
            announced_addr: None,
            caps: Arc::new(darkrun_harness::Harness::ClaudeCode.capabilities()),
        }
    }

    /// Record the in-process HTTP/WS bind address so it is announced to the
    /// agent in the MCP server `instructions`.
    pub fn with_announced_addr(mut self, addr: std::net::SocketAddr) -> Self {
        self.announced_addr = Some(addr);
        self
    }

    /// Adapt the server to a specific agent harness — sets the capability set
    /// that drives tool filtering, instruction adaptation, and prompt bridging.
    pub fn with_harness(mut self, harness: darkrun_harness::Harness) -> Self {
        self.caps = Arc::new(harness.capabilities());
        self
    }

    /// The active harness capability set.
    pub fn capabilities(&self) -> &darkrun_harness::Capabilities {
        &self.caps
    }

    /// The shared in-memory session registry this server upserts into — the same
    /// handle the in-process HTTP/WS server serves from. Lets an embedder (or a
    /// test simulating the HTTP answer handler) observe/mutate live sessions.
    pub fn sessions(&self) -> &SessionRegistry {
        &self.sessions
    }

    fn store(&self) -> StateStore {
        StateStore::new(self.repo_root.as_ref())
    }
}

fn ok_json<T: Serialize>(value: &T) -> std::result::Result<CallToolResult, ErrorData> {
    match serde_json::to_value(value) {
        Ok(v) => Ok(CallToolResult::structured(v)),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "serialization error: {e}"
        ))])),
    }
}

fn err_text(message: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.to_string())])
}

// ── Tool input schemas ──────────────────────────────────────────────────

/// Input for `darkrun_run_start`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunStartInput {
    /// URL-safe run slug (the `.darkrun/<slug>/` directory).
    pub slug: String,
    /// The factory (methodology) to drive the run. Defaults to `software`.
    #[serde(default = "default_factory")]
    pub factory: String,
    /// Optional human-readable title.
    #[serde(default)]
    pub title: Option<String>,
    /// Run sizing mode. Defaults to `continuous`.
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_factory() -> String {
    "software".to_string()
}
fn default_mode() -> String {
    "continuous".to_string()
}

/// Input for tools that operate on an existing run.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunRef {
    /// The run slug.
    pub slug: String,
}

/// Input for `darkrun_checkpoint_decide`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CheckpointDecideInput {
    /// The run slug.
    pub slug: String,
    /// `true` advances the station; `false` holds it and routes rework back.
    pub approved: bool,
    /// Optional feedback recorded when not approved.
    #[serde(default)]
    pub feedback: Option<String>,
}

/// Input for `darkrun_unit_get`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UnitRef {
    /// The run slug.
    pub slug: String,
    /// The unit slug.
    pub unit: String,
}

/// Input for `darkrun_unit_create`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UnitCreateInput {
    /// The run slug.
    pub slug: String,
    /// The new unit's slug (unique within the run).
    pub unit: String,
    /// The station this unit belongs to.
    pub station: String,
    /// Optional display title.
    #[serde(default)]
    pub title: Option<String>,
    /// Slugs of units this one depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// Input for `darkrun_unit_update` — corrective, field-scoped edits.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UnitUpdateInput {
    /// The run slug.
    pub slug: String,
    /// The unit slug.
    pub unit: String,
    /// New status: `pending`/`active`/`in_progress`/`completed`/`blocked`.
    #[serde(default)]
    pub status: Option<String>,
    /// New dependency set (pending units only).
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    /// New worker assignment.
    #[serde(default)]
    pub worker: Option<String>,
    /// New declared input paths (pending units only).
    #[serde(default)]
    pub inputs: Option<Vec<String>>,
    /// New declared output paths.
    #[serde(default)]
    pub outputs: Option<Vec<String>>,
}

/// Input for `darkrun_feedback_create`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackCreateInput {
    /// The run slug.
    pub slug: String,
    /// The station the finding targets.
    pub station: String,
    /// The finding text.
    pub body: String,
    /// Optional severity: `blocker`/`high`/`medium`/`low`.
    #[serde(default)]
    pub severity: Option<String>,
}

/// Input for `darkrun_feedback_list`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackListInput {
    /// The run slug.
    pub slug: String,
    /// When true, include settled (terminal) items. Defaults to including all.
    #[serde(default = "default_true")]
    pub include_settled: bool,
}

fn default_true() -> bool {
    true
}

/// Input for feedback id-scoped tools (`resolve`/`reject`/`move`/`severity`).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackResolveInput {
    /// The run slug.
    pub slug: String,
    /// The feedback id (e.g. `fb-01`).
    pub feedback_id: String,
    /// The terminal status to apply: `addressed`/`answered`/`non_actionable`/`closed`.
    #[serde(default = "default_addressed")]
    pub status: String,
}

fn default_addressed() -> String {
    "addressed".to_string()
}

/// Input for `darkrun_feedback_reject`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackRejectInput {
    /// The run slug.
    pub slug: String,
    /// The feedback id.
    pub feedback_id: String,
    /// Why the finding is invalid/stale.
    pub reason: String,
}

/// Input for `darkrun_feedback_move`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackMoveInput {
    /// The run slug.
    pub slug: String,
    /// The feedback id.
    pub feedback_id: String,
    /// The station to relocate the finding to.
    pub to_station: String,
}

/// Input for `darkrun_run_list`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunListInput {
    /// When true, include archived runs. Defaults to false.
    #[serde(default)]
    pub include_archived: bool,
}

/// Input for `darkrun_run_archive`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunArchiveInput {
    /// The run slug.
    pub slug: String,
    /// Set the archived flag (true to archive, false to restore). Defaults to true.
    #[serde(default = "default_true")]
    pub archived: bool,
}

/// Input for `darkrun_factory_detail`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FactoryRef {
    /// The factory name (e.g. `software`).
    pub factory: String,
}

// ── Visual-session tool input schemas ───────────────────────────────────

/// One selectable option in a `darkrun_question` — an optionally-image-backed
/// design choice the operator can pick.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct QuestionOptionInput {
    /// Canonical option id echoed back in the answer's `selected[]`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional generated-image URL (a mockup / design option to pick among).
    #[serde(default)]
    pub image_url: Option<String>,
    /// Optional longer description rendered under the label.
    #[serde(default)]
    pub description: Option<String>,
}

/// Input for `darkrun_question` — emit a VISUAL QUESTION the operator answers.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct QuestionInput {
    /// The run slug the session belongs to.
    pub slug: String,
    /// Optional title rendered above the prompt.
    #[serde(default)]
    pub title: Option<String>,
    /// The question prompt (required).
    pub prompt: String,
    /// Optional markdown context preamble.
    #[serde(default)]
    pub context: Option<String>,
    /// The selectable options (at least one required).
    pub options: Vec<QuestionOptionInput>,
    /// Whether more than one option may be selected. Defaults to false.
    #[serde(default)]
    pub multi_select: bool,
    /// Reference image URLs the question annotates (distinct from per-option
    /// images).
    #[serde(default)]
    pub image_urls: Vec<String>,
}

/// One design archetype card in a `darkrun_direction` — an image-backed design
/// direction the operator chooses + annotates.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ArchetypeInput {
    /// Canonical archetype id echoed back as `chosen_archetype`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Generated preview-image URL (required).
    pub image_url: String,
    /// Description of the design direction this archetype represents.
    pub description: String,
}

/// Input for `darkrun_direction` — emit a DESIGN DIRECTION session.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DirectionInput {
    /// The run slug the session belongs to.
    pub slug: String,
    /// Optional title rendered above the prompt.
    #[serde(default)]
    pub title: Option<String>,
    /// The prompt rendered above the archetype cards (required).
    pub prompt: String,
    /// Optional markdown preamble.
    #[serde(default)]
    pub context: Option<String>,
    /// The design archetypes to choose between (at least one required).
    pub archetypes: Vec<ArchetypeInput>,
}

/// One selectable option in a `darkrun_picker`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct PickerOptionInput {
    /// Canonical id echoed back on selection.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether the option hides behind a "show all" expansion.
    #[serde(default)]
    pub secondary: Option<bool>,
}

/// Input for `darkrun_picker` — emit a blocking selection among options.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct PickerInput {
    /// The run slug the session belongs to.
    pub slug: String,
    /// The selection kind: `factory`/`mode`/`station`/`confirm`/`url_input`.
    pub kind: String,
    /// Title (required).
    pub title: String,
    /// Prompt text (required).
    pub prompt: String,
    /// Selectable options (at least one required).
    pub options: Vec<PickerOptionInput>,
}

/// Input for the `darkrun_*_result` readers — read back a visual session's
/// submitted answer/selection.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SessionResultInput {
    /// The run slug the session belongs to.
    pub slug: String,
    /// The session id minted when the session was created.
    pub session_id: String,
}

/// Input for `darkrun_run_surface` — classify or read a run's verification
/// surface. With `surface` set, the run is classified (and persisted onto the
/// frontmatter); omitted, the tool just reads the current classification.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunSurfaceInput {
    /// The run slug.
    pub slug: String,
    /// The surface token to classify the run as. One of `library`, `api`,
    /// `web_ui` (or `web-ui`/`webui`), `tui`, `cli`, `desktop`, `mobile`,
    /// `data`. Omit to read the current surface without changing it.
    #[serde(default)]
    pub surface: Option<String>,
}

/// Input for `darkrun_proof_attach` — attach surface-routed objective evidence
/// (the Prove station's NUMBERS) to a run.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ProofAttachInput {
    /// The run slug.
    pub slug: String,
    /// The objective proof object. Its `surface` must match the run's
    /// classified surface. Shape: `{ "surface": "web_ui", "web": { "vitals":
    /// {"lcp": 1200.0, ...}, "audits": [{"name": "contrast", "value": "4.8:1",
    /// "pass": true}], "screenshot_url": "..." } }` for visual surfaces, or
    /// `{ "surface": "api", "bench": { "p50": .., "p95": .., "p99": ..,
    /// "throughput": .., "samples": .. } }` for bench surfaces. A terminal
    /// (cli/tui) surface carries a screenshot-only `web` block.
    pub proof: serde_json::Value,
    /// The station the proof was measured at (e.g. `prove`). Omit for a
    /// run-level proof.
    #[serde(default)]
    pub station: Option<String>,
}

/// Input for `darkrun_proof_get` — read a run's attached objective proof.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ProofGetInput {
    /// The run slug.
    pub slug: String,
    /// The station whose proof to read. Falls back to the run-level proof when
    /// the station has none; omit to read the run-level proof directly.
    #[serde(default)]
    pub station: Option<String>,
}

fn parse_status_arg(raw: &str) -> Option<Status> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Some(Status::Pending),
        "active" => Some(Status::Active),
        "in_progress" | "inprogress" => Some(Status::InProgress),
        "completed" => Some(Status::Completed),
        "blocked" => Some(Status::Blocked),
        _ => None,
    }
}

fn parse_feedback_status_arg(raw: &str) -> Option<FeedbackStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Some(FeedbackStatus::Pending),
        "fixing" => Some(FeedbackStatus::Fixing),
        "addressed" => Some(FeedbackStatus::Addressed),
        "answered" => Some(FeedbackStatus::Answered),
        "non_actionable" | "nonactionable" => Some(FeedbackStatus::NonActionable),
        "escalated" => Some(FeedbackStatus::Escalated),
        "closed" => Some(FeedbackStatus::Closed),
        "rejected" => Some(FeedbackStatus::Rejected),
        _ => None,
    }
}

// ── Tool handlers ───────────────────────────────────────────────────────

#[tool_router]
impl DarkrunServer {
    /// Start a new run: seed `.darkrun/<slug>/` state at the factory's first
    /// station and return the run.
    #[tool(
        name = "darkrun_run_start",
        description = "Start a new darkrun Run on a factory, seeding .darkrun state at the first station."
    )]
    pub fn darkrun_run_start(
        &self,
        Parameters(input): Parameters<RunStartInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        if input.slug.trim().is_empty() {
            return Ok(err_text("slug must not be empty"));
        }
        let store = self.store();
        match run_start(
            &store,
            &input.slug,
            &input.factory,
            input.title.clone(),
            &input.mode,
        ) {
            Ok(run) => ok_json(&run),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Drive one workflow tick and return the next action the agent should
    /// perform. Three-track priority: drift -> feedback -> run.
    #[tool(
        name = "darkrun_run_next",
        description = "Advance the run one tick; returns the next structured action (drift -> feedback -> run)."
    )]
    pub fn darkrun_run_next(
        &self,
        Parameters(input): Parameters<RunRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match run_tick(&store, &input.slug) {
            Ok(tick) => ok_json(&tick),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Show a run's frontmatter, derived state, and current position.
    #[tool(
        name = "darkrun_run_show",
        description = "Show a run: frontmatter, derived station state, and the current cursor position."
    )]
    pub fn darkrun_run_show(
        &self,
        Parameters(input): Parameters<RunRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let run = match store.read_run(&input.slug) {
            Ok(r) => r,
            Err(e) => return Ok(err_text(e)),
        };
        let state = store.read_state(&input.slug).ok().flatten();
        let position = crate::position::derive_position(&store, &input.slug).ok();
        ok_json(&serde_json::json!({
            "run": run,
            "state": state,
            "position": position,
        }))
    }

    /// List a run's units with their status and station.
    #[tool(
        name = "darkrun_unit_list",
        description = "List a run's units with status, station, and dependencies."
    )]
    pub fn darkrun_unit_list(
        &self,
        Parameters(input): Parameters<RunRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match store.read_units(&input.slug) {
            Ok(units) => ok_json(&units),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// List every factory available in this build, with their station plans.
    #[tool(
        name = "darkrun_factory_list",
        description = "List available factories and their ordered station plans."
    )]
    pub fn darkrun_factory_list(&self) -> std::result::Result<CallToolResult, ErrorData> {
        let factories: Vec<_> = list_factories()
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "name": f.name,
                    "stations": f.stations.iter().map(|s| serde_json::json!({
                        "name": s.name,
                        "kills": s.kills,
                        "artifact": s.artifact,
                        "checkpoint": s.checkpoint,
                        "workers": s.workers,
                        "reviewers": s.reviewers,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        ok_json(&factories)
    }

    /// Apply an operator decision to the active station's Checkpoint and
    /// re-tick. `approved` advances; otherwise the station holds and rework
    /// routes back as feedback.
    #[tool(
        name = "darkrun_checkpoint_decide",
        description = "Decide the active station's checkpoint: approve to advance, or reject to hold and route rework."
    )]
    pub fn darkrun_checkpoint_decide(
        &self,
        Parameters(input): Parameters<CheckpointDecideInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match checkpoint_decide(&store, &input.slug, input.approved, input.feedback.clone()) {
            Ok(tick) => ok_json(&tick),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Units ────────────────────────────────────────────────────────────

    /// Read a single unit's frontmatter and body.
    #[tool(
        name = "darkrun_unit_get",
        description = "Read one unit: its frontmatter (status, station, deps, worker) and body."
    )]
    pub fn darkrun_unit_get(
        &self,
        Parameters(input): Parameters<UnitRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match units::get(&store, &input.slug, &input.unit) {
            Ok(unit) => ok_json(&unit),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Decompose a station into a new pending unit.
    #[tool(
        name = "darkrun_unit_create",
        description = "Create a new pending unit on a station, with an optional title and dependency set."
    )]
    pub fn darkrun_unit_create(
        &self,
        Parameters(input): Parameters<UnitCreateInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match units::create(
            &store,
            &input.slug,
            &input.unit,
            &input.station,
            input.title.clone(),
            input.depends_on.clone(),
        ) {
            Ok(unit) => ok_json(&unit),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Apply a corrective, field-scoped update to a unit.
    #[tool(
        name = "darkrun_unit_update",
        description = "Update a unit's fields. Structural edits (deps/inputs) require the unit be pending."
    )]
    pub fn darkrun_unit_update(
        &self,
        Parameters(input): Parameters<UnitUpdateInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let status = match &input.status {
            Some(s) => match parse_status_arg(s) {
                Some(st) => Some(st),
                None => return Ok(err_text(format!("invalid status: {s}"))),
            },
            None => None,
        };
        let upd = units::UnitUpdate {
            status,
            depends_on: input.depends_on.clone(),
            worker: input.worker.clone(),
            inputs: input.inputs.clone(),
            outputs: input.outputs.clone(),
        };
        match units::update(&store, &input.slug, &input.unit, upd) {
            Ok(unit) => ok_json(&unit),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Feedback ─────────────────────────────────────────────────────────

    /// File a feedback finding against a station.
    #[tool(
        name = "darkrun_feedback_create",
        description = "Create a pending feedback finding on a station, with an optional severity."
    )]
    pub fn darkrun_feedback_create(
        &self,
        Parameters(input): Parameters<FeedbackCreateInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let severity = match &input.severity {
            Some(s) => match feedback::parse_severity(s) {
                Some(sev) => Some(sev),
                None => return Ok(err_text(format!("invalid severity: {s}"))),
            },
            None => None,
        };
        match feedback::create(&store, &input.slug, &input.station, &input.body, severity) {
            Ok(fb) => ok_json(&fb),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// List a run's feedback findings.
    #[tool(
        name = "darkrun_feedback_list",
        description = "List feedback findings for a run; set include_settled=false to hide terminal items."
    )]
    pub fn darkrun_feedback_list(
        &self,
        Parameters(input): Parameters<FeedbackListInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match feedback::list(&store, &input.slug) {
            Ok(all) => {
                let items: Vec<_> = all
                    .into_iter()
                    .filter(|f| input.include_settled || !feedback::is_terminal(f.status))
                    .collect();
                ok_json(&items)
            }
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Resolve a feedback finding by stamping a terminal status.
    #[tool(
        name = "darkrun_feedback_resolve",
        description = "Resolve a feedback finding with a terminal status (addressed/answered/non_actionable/closed)."
    )]
    pub fn darkrun_feedback_resolve(
        &self,
        Parameters(input): Parameters<FeedbackResolveInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let status = match parse_feedback_status_arg(&input.status) {
            Some(s) if feedback::is_terminal(s) => s,
            Some(_) => {
                return Ok(err_text(
                    "resolve requires a terminal status: addressed/answered/non_actionable/closed",
                ))
            }
            None => return Ok(err_text(format!("invalid status: {}", input.status))),
        };
        match feedback::set_status(&store, &input.slug, &input.feedback_id, status) {
            Ok(fb) => ok_json(&fb),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Reject a feedback finding as invalid/stale (a terminal transition).
    #[tool(
        name = "darkrun_feedback_reject",
        description = "Reject a feedback finding with a reason; terminal, so the manager stops re-dispatching it."
    )]
    pub fn darkrun_feedback_reject(
        &self,
        Parameters(input): Parameters<FeedbackRejectInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match feedback::reject(&store, &input.slug, &input.feedback_id, &input.reason) {
            Ok(fb) => ok_json(&fb),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Relocate a feedback finding to a different station (triage).
    #[tool(
        name = "darkrun_feedback_move",
        description = "Relocate a feedback finding to a different station for triage."
    )]
    pub fn darkrun_feedback_move(
        &self,
        Parameters(input): Parameters<FeedbackMoveInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match feedback::move_station(&store, &input.slug, &input.feedback_id, &input.to_station) {
            Ok(fb) => ok_json(&fb),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Runs ─────────────────────────────────────────────────────────────

    /// List every run with a compact summary.
    #[tool(
        name = "darkrun_run_list",
        description = "List runs (slug, title, factory, status, active station); set include_archived to show archived runs."
    )]
    pub fn darkrun_run_list(
        &self,
        Parameters(input): Parameters<RunListInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match runs::list(&store, input.include_archived) {
            Ok(list) => ok_json(&list),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Archive (or restore) a run.
    #[tool(
        name = "darkrun_run_archive",
        description = "Archive a run (or restore it with archived=false); archiving clears it from the active pointer."
    )]
    pub fn darkrun_run_archive(
        &self,
        Parameters(input): Parameters<RunArchiveInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match runs::set_archived(&store, &input.slug, input.archived) {
            Ok(()) => ok_json(&serde_json::json!({
                "slug": input.slug,
                "archived": input.archived,
            })),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Surface + proof ──────────────────────────────────────────────────

    /// Classify or read a run's verification SURFACE — the linchpin that routes
    /// which objective measurement Prove/Audit apply. Set `surface` to classify
    /// (Shape calls this once the deliverable is known); omit it to read the
    /// current classification back. Returns the surface plus its route flags
    /// (`is_visual`/`is_bench`/`is_terminal`) and the selected route
    /// (`web`/`bench`/`terminal`).
    #[tool(
        name = "darkrun_run_surface",
        description = "Classify or read a run's verification surface (library|api|web_ui|tui|cli|desktop|mobile|data); set `surface` to classify, omit to read. Routes which objective proof Prove/Audit apply."
    )]
    pub fn darkrun_run_surface(
        &self,
        Parameters(input): Parameters<RunSurfaceInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let result = match input.surface.as_deref() {
            Some(raw) => proof::set_surface(&store, &input.slug, raw),
            None => proof::get_surface(&store, &input.slug),
        };
        match result {
            Ok(res) => ok_json(&res),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Attach surface-routed objective EVIDENCE — the Prove station's NUMBERS —
    /// to a run. The proof's `surface` must match the run's classified surface;
    /// the response reports `block_matches_surface` (a visual surface must carry
    /// a `web` vitals/audits block, a bench surface a `bench` percentile block)
    /// so the agent cannot pass Prove on an eyeballed claim.
    #[tool(
        name = "darkrun_proof_attach",
        description = "Attach surface-routed objective proof (web vitals+audits, or bench percentiles+throughput) to a run; the proof surface must match the run's classified surface."
    )]
    pub fn darkrun_proof_attach(
        &self,
        Parameters(input): Parameters<ProofAttachInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let parsed: darkrun_api::proof::Proof = match serde_json::from_value(input.proof) {
            Ok(p) => p,
            Err(e) => return Ok(err_text(format!("invalid proof payload: {e}"))),
        };
        let store = self.store();
        match proof::attach_proof(&store, &input.slug, parsed, input.station) {
            Ok(resp) => ok_json(&resp),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Read a run's attached objective proof back — for the view/review, or for
    /// a downstream station to confirm Prove measured what the surface demands.
    /// Pass a `station` to read that station's scoped proof (falling back to the
    /// run-level proof), or omit it for the run-level proof.
    #[tool(
        name = "darkrun_proof_get",
        description = "Read a run's attached objective proof (surface + web/bench block); pass a station to read its scoped proof, omit for the run-level proof."
    )]
    pub fn darkrun_proof_get(
        &self,
        Parameters(input): Parameters<ProofGetInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match proof::get_proof(&store, &input.slug, input.station) {
            Ok(resp) => ok_json(&resp),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Factories ────────────────────────────────────────────────────────

    /// Show one factory's full station plan in detail.
    #[tool(
        name = "darkrun_factory_detail",
        description = "Show a single factory's ordered station plan (kills, artifact, checkpoint, workers, reviewers)."
    )]
    pub fn darkrun_factory_detail(
        &self,
        Parameters(input): Parameters<FactoryRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let factory = match resolve_factory(&input.factory) {
            Some(f) => f,
            None => return Ok(err_text(format!("unknown factory: {}", input.factory))),
        };
        ok_json(&serde_json::json!({
            "name": factory.name,
            "stations": factory.stations.iter().map(|s| serde_json::json!({
                "name": s.name,
                "kills": s.kills,
                "artifact": s.artifact,
                "checkpoint": s.checkpoint,
                "workers": s.workers,
                "reviewers": s.reviewers,
            })).collect::<Vec<_>>(),
        }))
    }

    // ── Visual sessions ──────────────────────────────────────────────────

    /// Emit a VISUAL QUESTION: pose the operator a prompt with a list of
    /// (optionally image-backed) options to pick among. Registers a pending
    /// question session the desktop app serves, and returns the session id +
    /// an "awaiting answer" handle. Read the answer back with
    /// `darkrun_question_result`.
    #[tool(
        name = "darkrun_question",
        description = "Ask the operator a visual multi/single-select question with image options; returns the awaiting session id."
    )]
    pub fn darkrun_question(
        &self,
        Parameters(input): Parameters<QuestionInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let options = input
            .options
            .into_iter()
            .map(|o| QuestionOptionSpec {
                id: o.id,
                label: o.label,
                image_url: o.image_url,
                description: o.description,
            })
            .collect();
        match sessions::create_question(
            &self.sessions,
            &input.slug,
            input.title,
            &input.prompt,
            input.context,
            options,
            input.multi_select,
            input.image_urls,
        ) {
            Ok(awaiting) => ok_json(&awaiting),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Read back the operator's answer to a question session: the selected
    /// option ids, any free text, and the current session status.
    #[tool(
        name = "darkrun_question_result",
        description = "Read back a question session's submitted answer (selected ids + text) and status."
    )]
    pub fn darkrun_question_result(
        &self,
        Parameters(input): Parameters<SessionResultInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match sessions::question_result(&self.sessions, &input.slug, &input.session_id) {
            Ok(q) => ok_json(&q),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Emit a DESIGN DIRECTION: present design archetypes (each an image-backed
    /// direction) for the operator to choose + annotate. Registers a pending
    /// direction session and returns the awaiting handle. Read the choice back
    /// with `darkrun_direction_result`.
    #[tool(
        name = "darkrun_direction",
        description = "Ask the operator for a design direction: pick + annotate one of several image-backed archetypes."
    )]
    pub fn darkrun_direction(
        &self,
        Parameters(input): Parameters<DirectionInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let archetypes = input
            .archetypes
            .into_iter()
            .map(|a| ArchetypeSpec {
                id: a.id,
                label: a.label,
                image_url: a.image_url,
                description: a.description,
            })
            .collect();
        match sessions::create_direction(
            &self.sessions,
            &input.slug,
            input.title,
            &input.prompt,
            input.context,
            archetypes,
        ) {
            Ok(awaiting) => ok_json(&awaiting),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Read back the operator's design direction: the chosen archetype id,
    /// annotations (pins / screenshot / comments), and the session status.
    #[tool(
        name = "darkrun_direction_result",
        description = "Read back a direction session's chosen archetype + annotations and status."
    )]
    pub fn darkrun_direction_result(
        &self,
        Parameters(input): Parameters<SessionResultInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match sessions::direction_result(&self.sessions, &input.slug, &input.session_id) {
            Ok(d) => ok_json(&d),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Emit a blocking PICKER: have the operator choose among labelled options.
    /// Registers a pending picker session and returns the awaiting handle. Read
    /// the selection back with `darkrun_picker_result`.
    #[tool(
        name = "darkrun_picker",
        description = "Ask the operator to choose among options (factory/mode/station/confirm/url_input); returns the awaiting session id."
    )]
    pub fn darkrun_picker(
        &self,
        Parameters(input): Parameters<PickerInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let kind = match sessions::parse_picker_kind(&input.kind) {
            Some(k) => k,
            None => return Ok(err_text(format!("invalid picker kind: {}", input.kind))),
        };
        let options = input
            .options
            .into_iter()
            .map(|o| PickerOptionSpec {
                id: o.id,
                label: o.label,
                description: o.description,
                secondary: o.secondary,
            })
            .collect();
        match sessions::create_picker(
            &self.sessions,
            &input.slug,
            kind,
            &input.title,
            &input.prompt,
            options,
        ) {
            Ok(awaiting) => ok_json(&awaiting),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Read back the operator's picker selection and the session status.
    #[tool(
        name = "darkrun_picker_result",
        description = "Read back a picker session's selected option id and status."
    )]
    pub fn darkrun_picker_result(
        &self,
        Parameters(input): Parameters<SessionResultInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match sessions::picker_result(&self.sessions, &input.slug, &input.session_id) {
            Ok(p) => ok_json(&p),
            Err(e) => Ok(err_text(e)),
        }
    }
}

/// Tools that require the browser/desktop review UI. Dropped on harnesses
/// without it (`browser_ui == false`): the design-direction surface is
/// inherently visual (mockup selection), so on those harnesses the agent falls
/// back to a text decision (elicitation or inline) per the adapted instructions.
const VISUAL_TOOL_NAMES: &[&str] = &["darkrun_direction", "darkrun_direction_result"];

/// Adapt the full tool list to a harness: drop browser/visual tools when the
/// harness has no desktop UI, then enforce its tool budget. Pure over the
/// inputs so it's unit-testable without an MCP request context.
fn adapt_tool_list(
    caps: &darkrun_harness::Capabilities,
    mut tools: Vec<rmcp::model::Tool>,
) -> Vec<rmcp::model::Tool> {
    if !caps.browser_ui {
        tools.retain(|t| !VISUAL_TOOL_NAMES.contains(&t.name.as_ref()));
    }
    if let Some(max) = caps.max_tools {
        tools.truncate(max);
    }
    tools
}

#[tool_handler]
impl ServerHandler for DarkrunServer {
    /// List the tools, adapted to the active harness: drop the browser/visual
    /// tools on harnesses without a desktop UI, then enforce the harness tool
    /// budget (e.g. Cursor caps at ~40). `get_info`/`call_tool` are generated by
    /// `#[tool_handler]`; defining `list_tools` here makes the macro skip its
    /// default so our filtered view wins.
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<rmcp::model::ListToolsResult, ErrorData> {
        // With ~26 tools we sit under every harness's budget today, so the
        // truncation in `adapt_tool_list` is a safety rail rather than a live
        // constraint; visual-tool removal is the active filter.
        let tools = adapt_tool_list(&self.caps, Self::tool_router().list_all());
        Ok(rmcp::model::ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        let mut instructions = String::from(
            "darkrun manager. Call darkrun_run_start to begin a Run, then \
             darkrun_run_next repeatedly to walk its factory stations. Each tick returns \
             a structured next-action instruction — perform it (write artifacts, \
             decompose units, complete passes), then re-tick. Use \
             darkrun_checkpoint_decide to resolve a station's gate.",
        );
        if let Some(addr) = self.announced_addr {
            instructions.push_str(&format!(
                " The interactive review server (HTTP/WS) is hosted in-process on \
                 http://{addr} — the desktop app reads DARKRUN_PORT={} to connect. \
                 Visual sessions raised via darkrun_question/direction/picker are \
                 served live from there.",
                addr.port()
            ));
        }
        info.instructions = Some(instructions);
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn claude_code_lists_every_tool_including_visual() {
        let all = DarkrunServer::tool_router().list_all();
        let caps = darkrun_harness::Harness::ClaudeCode.capabilities();
        let adapted = adapt_tool_list(&caps, all.clone());
        assert_eq!(adapted.len(), all.len());
        assert!(adapted.iter().any(|t| t.name == "darkrun_direction"));
    }

    #[test]
    fn non_browser_harness_drops_visual_tools() {
        let all = DarkrunServer::tool_router().list_all();
        let caps = darkrun_harness::Harness::Cursor.capabilities();
        let adapted = adapt_tool_list(&caps, all.clone());
        assert!(!adapted.iter().any(|t| t.name == "darkrun_direction"));
        assert!(!adapted.iter().any(|t| t.name == "darkrun_direction_result"));
        // Non-visual tools survive.
        assert!(adapted.iter().any(|t| t.name == "darkrun_run_next"));
        assert_eq!(adapted.len(), all.len() - VISUAL_TOOL_NAMES.len());
    }

    #[test]
    fn tool_budget_truncates_when_exceeded() {
        let mut caps = darkrun_harness::Harness::Cursor.capabilities();
        caps.max_tools = Some(5);
        let adapted = adapt_tool_list(&caps, DarkrunServer::tool_router().list_all());
        assert_eq!(adapted.len(), 5);
    }

    #[test]
    fn run_start_tool_creates_state() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        let res = server
            .darkrun_run_start(Parameters(RunStartInput {
                slug: "r".into(),
                factory: "software".into(),
                title: Some("t".into()),
                mode: "continuous".into(),
            }))
            .unwrap();
        assert_eq!(res.is_error, Some(false));
        assert!(dir.path().join(".darkrun/r/run.md").exists());
    }

    #[test]
    fn run_next_tool_advances() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        server
            .darkrun_run_start(Parameters(RunStartInput {
                slug: "r".into(),
                factory: "software".into(),
                title: None,
                mode: "continuous".into(),
            }))
            .unwrap();
        let res = server
            .darkrun_run_next(Parameters(RunRef { slug: "r".into() }))
            .unwrap();
        assert_eq!(res.is_error, Some(false));
        let v = res.structured_content.unwrap();
        assert_eq!(v["action"]["action"], "spec");
        assert_eq!(v["action"]["station"], "frame");
    }

    #[test]
    fn factory_list_tool_lists_software() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        let res = server.darkrun_factory_list().unwrap();
        let v = res.structured_content.unwrap();
        assert_eq!(v[0]["name"], "software");
        assert_eq!(v[0]["stations"][0]["name"], "frame");
    }

    #[test]
    fn empty_slug_is_rejected() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        let res = server
            .darkrun_run_start(Parameters(RunStartInput {
                slug: "  ".into(),
                factory: "software".into(),
                title: None,
                mode: "continuous".into(),
            }))
            .unwrap();
        assert_eq!(res.is_error, Some(true));
    }

    fn started_server() -> (tempfile::TempDir, DarkrunServer) {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        server
            .darkrun_run_start(Parameters(RunStartInput {
                slug: "r".into(),
                factory: "software".into(),
                title: Some("Run".into()),
                mode: "continuous".into(),
            }))
            .unwrap();
        (dir, server)
    }

    #[test]
    fn unit_create_get_update_roundtrip() {
        let (_d, server) = started_server();
        let created = server
            .darkrun_unit_create(Parameters(UnitCreateInput {
                slug: "r".into(),
                unit: "u1".into(),
                station: "frame".into(),
                title: Some("First".into()),
                depends_on: vec![],
            }))
            .unwrap();
        assert_eq!(created.is_error, Some(false));

        let got = server
            .darkrun_unit_get(Parameters(UnitRef {
                slug: "r".into(),
                unit: "u1".into(),
            }))
            .unwrap();
        let v = got.structured_content.unwrap();
        assert_eq!(v["slug"], "u1");
        assert_eq!(v["frontmatter"]["station"], "frame");

        let updated = server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: Some("completed".into()),
                depends_on: None,
                worker: None,
                inputs: None,
                outputs: None,
            }))
            .unwrap();
        let v = updated.structured_content.unwrap();
        assert_eq!(v["frontmatter"]["status"], "completed");
    }

    #[test]
    fn unit_update_rejects_bad_status() {
        let (_d, server) = started_server();
        server
            .darkrun_unit_create(Parameters(UnitCreateInput {
                slug: "r".into(),
                unit: "u1".into(),
                station: "frame".into(),
                title: None,
                depends_on: vec![],
            }))
            .unwrap();
        let res = server
            .darkrun_unit_update(Parameters(UnitUpdateInput {
                slug: "r".into(),
                unit: "u1".into(),
                status: Some("nonsense".into()),
                depends_on: None,
                worker: None,
                inputs: None,
                outputs: None,
            }))
            .unwrap();
        assert_eq!(res.is_error, Some(true));
    }

    #[test]
    fn feedback_create_list_resolve_flow() {
        let (_d, server) = started_server();
        let created = server
            .darkrun_feedback_create(Parameters(FeedbackCreateInput {
                slug: "r".into(),
                station: "frame".into(),
                body: "widget overflows".into(),
                severity: Some("high".into()),
            }))
            .unwrap();
        let v = created.structured_content.unwrap();
        let id = v["id"].as_str().unwrap().to_string();
        assert_eq!(v["severity"], "high");

        // Listing shows the open item.
        let listed = server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: true,
            }))
            .unwrap();
        let v = listed.structured_content.unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);

        // Resolve it terminally.
        let resolved = server
            .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
                slug: "r".into(),
                feedback_id: id,
                status: "addressed".into(),
            }))
            .unwrap();
        let v = resolved.structured_content.unwrap();
        assert_eq!(v["status"], "addressed");

        // Hiding settled now yields an empty list.
        let listed = server
            .darkrun_feedback_list(Parameters(FeedbackListInput {
                slug: "r".into(),
                include_settled: false,
            }))
            .unwrap();
        let v = listed.structured_content.unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn feedback_resolve_rejects_non_terminal_status() {
        let (_d, server) = started_server();
        let created = server
            .darkrun_feedback_create(Parameters(FeedbackCreateInput {
                slug: "r".into(),
                station: "frame".into(),
                body: "x".into(),
                severity: None,
            }))
            .unwrap();
        let id = created.structured_content.unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let res = server
            .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
                slug: "r".into(),
                feedback_id: id,
                status: "fixing".into(),
            }))
            .unwrap();
        assert_eq!(res.is_error, Some(true));
    }

    #[test]
    fn feedback_reject_and_move() {
        let (_d, server) = started_server();
        let a = server
            .darkrun_feedback_create(Parameters(FeedbackCreateInput {
                slug: "r".into(),
                station: "frame".into(),
                body: "a".into(),
                severity: None,
            }))
            .unwrap();
        let id_a = a.structured_content.unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let moved = server
            .darkrun_feedback_move(Parameters(FeedbackMoveInput {
                slug: "r".into(),
                feedback_id: id_a.clone(),
                to_station: "shape".into(),
            }))
            .unwrap();
        assert_eq!(moved.structured_content.unwrap()["station"], "shape");

        let rejected = server
            .darkrun_feedback_reject(Parameters(FeedbackRejectInput {
                slug: "r".into(),
                feedback_id: id_a,
                reason: "duplicate".into(),
            }))
            .unwrap();
        assert_eq!(rejected.structured_content.unwrap()["status"], "rejected");
    }

    #[test]
    fn run_list_and_archive_tools() {
        let (_d, server) = started_server();
        let listed = server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap();
        assert_eq!(listed.structured_content.unwrap().as_array().unwrap().len(), 1);

        server
            .darkrun_run_archive(Parameters(RunArchiveInput {
                slug: "r".into(),
                archived: true,
            }))
            .unwrap();

        let listed = server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: false,
            }))
            .unwrap();
        assert_eq!(listed.structured_content.unwrap().as_array().unwrap().len(), 0);

        let listed = server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: true,
            }))
            .unwrap();
        assert_eq!(listed.structured_content.unwrap().as_array().unwrap().len(), 1);
    }

    #[test]
    fn factory_detail_tool() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        let res = server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "software".into(),
            }))
            .unwrap();
        let v = res.structured_content.unwrap();
        assert_eq!(v["name"], "software");
        assert_eq!(v["stations"].as_array().unwrap().len(), 6);
        assert_eq!(v["stations"][5]["name"], "harden");

        let bad = server
            .darkrun_factory_detail(Parameters(FactoryRef {
                factory: "nope".into(),
            }))
            .unwrap();
        assert_eq!(bad.is_error, Some(true));
    }
}
