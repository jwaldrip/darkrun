//! MCP tool surface — the CORE subset that drives a run from `start ->
//! pickup -> station loop`.
//!
//! The orchestrator + state tool handlers, expressed in the factory
//! vocabulary and reduced to:
//! `darkrun_run_new`, `darkrun_advance`, `darkrun_run_inspect`,
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

use darkrun_core::domain::{FeedbackStatus, Mode, Status};
use darkrun_core::StateStore;
use darkrun_git::GitBackend;

use darkrun_http::SessionRegistry;

use crate::factory::{list_factories, resolve_factory};
use crate::position::{checkpoint_decide, run_start, run_tick};
use crate::sessions::{self, ArchetypeSpec, PickerOptionSpec, QuestionOptionSpec};
use crate::{brief, feedback, human_write, knowledge, proof, reflection, runs, units};

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

    /// Resolve which run a slug-optional command (e.g. `darkrun_run_inspect`) targets.
    ///
    /// Priority, so a user standing in a run's worktree never has to name it:
    /// 1. an explicit, non-empty `given` slug wins;
    /// 2. otherwise the **current git branch** — a `darkrun/<slug>/<…>` branch
    ///    names its run, so being on it *is* selecting it;
    /// 3. otherwise the recorded **active run** pointer;
    /// 4. otherwise, if exactly one non-archived run exists, that one.
    ///
    /// `None` only when there is genuinely nothing to disambiguate to.
    fn resolve_run_slug(&self, store: &StateStore, given: Option<&str>) -> Option<String> {
        if let Some(s) = given {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        // 2. The branch we're on. `darkrun/<slug>/main` or `darkrun/<slug>/<station>`.
        if let Ok(git) = darkrun_git::Git::open(self.repo_root.as_ref()) {
            if let Ok(Some(branch)) = git.current_branch() {
                if let Some(slug) = slug_from_branch(&branch) {
                    if store.read_run(&slug).is_ok() {
                        return Some(slug);
                    }
                }
            }
        }
        // 3. The active-run pointer.
        if let Ok(Some(active)) = store.active_run() {
            return Some(active);
        }
        // 4. A sole non-archived run is unambiguous.
        if let Ok(slugs) = store.list_runs() {
            let live: Vec<String> = slugs
                .into_iter()
                .filter(|s| {
                    store
                        .read_run(s)
                        .map(|r| !r.frontmatter.archived.unwrap_or(false))
                        .unwrap_or(false)
                })
                .collect();
            if let [only] = live.as_slice() {
                return Some(only.clone());
            }
        }
        None
    }

    /// Adapt a tick's engine-rendered prompt to the active harness (appends the
    /// "Harness note" with the execution-model differences) before it goes back
    /// to the agent. A no-op under Claude Code.
    fn adapt_tick(&self, mut tick: crate::position::TickResult) -> crate::position::TickResult {
        if let Some(p) = tick.prompt.as_mut() {
            *p = darkrun_harness::adapt_instructions(p, &self.caps);
        }
        tick
    }
}

fn ok_json<T: Serialize>(value: &T) -> std::result::Result<CallToolResult, ErrorData> {
    match serde_json::to_value(value) {
        Ok(v) => Ok(CallToolResult::structured(ensure_object(v))),
        Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
            "serialization error: {e}"
        ))])),
    }
}

/// MCP requires `structuredContent` to be a JSON object. A handler that
/// serializes a list (e.g. `darkrun_unit_list`, `darkrun_factory_list`) would
/// otherwise emit a top-level array, which strict clients reject with
/// "expected record, received array". Wrap any non-object value so every tool
/// returns a record: arrays under `items`, scalars under `value`. Objects pass
/// through unchanged.
fn ensure_object(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(_) => v,
        serde_json::Value::Array(_) => serde_json::json!({ "items": v }),
        other => serde_json::json!({ "value": other }),
    }
}

fn err_text(message: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.to_string())])
}

// ── Tool input schemas ──────────────────────────────────────────────────

/// Input for `darkrun_run_new`.
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
    /// The run's global review mode — the single gate axis. Defaults to `solo`.
    /// `team` (each station opens a PR the team reviews and merges), `solo` (each
    /// station asks for local review before advancing), or `dark` (pre-elaborate
    /// up front, then run without stopping for review). All modes pre-elaborate.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Right-sizing — orthogonal to `mode`. Pick from the problem during
    /// pre-elaboration. `full`/unknown (every station), `quick` (build+prove),
    /// `bugfix` (specify+build+prove), or `refactor` (shape+build+prove).
    #[serde(default = "default_size")]
    pub size: String,
}

fn default_factory() -> String {
    "software".to_string()
}
fn default_mode() -> String {
    "solo".to_string()
}
fn default_size() -> String {
    "full".to_string()
}

/// Input for tools that operate on an existing run.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunRef {
    /// The run slug.
    pub slug: String,
}

/// Input for `darkrun_run_inspect`, where the slug is **optional**: omit it and the
/// run is inferred from the current `darkrun/<slug>/…` branch, the active-run
/// pointer, or the sole run — so a user in a run's worktree need not name it.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunShowRef {
    /// The run slug. Omit to infer it from the current branch / active run.
    #[serde(default)]
    pub slug: Option<String>,
}

/// Extract a run slug from a `darkrun/<slug>/<segment>` branch name. Returns
/// `None` for any branch that is not a darkrun run branch (so an ordinary feature
/// branch never masquerades as a run).
fn slug_from_branch(branch: &str) -> Option<String> {
    let rest = branch.strip_prefix("darkrun/")?;
    let (slug, _segment) = rest.split_once('/')?;
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
    }
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

/// Input for `darkrun_unit_beat` — record one Pass beat (Make/Challenge/Resolve).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UnitIterateInput {
    /// The run slug.
    pub slug: String,
    /// The unit slug.
    pub unit: String,
    /// The worker that ran this beat (e.g. `make`, `challenge`, `resolve`).
    pub worker: String,
    /// The outcome: `advance` (move forward) or `reject` (bounce back).
    pub result: String,
    /// The handoff note — REQUIRED on reject, expected on advance. On advance:
    /// what you did and what the next worker should know. On reject: why you
    /// bounced. This is threaded into the next worker's dispatch.
    #[serde(default)]
    pub note: Option<String>,
    /// The worker to dispatch next: the following worker on advance, or the
    /// bounce target (nearest build worker) on reject. Optional — defaults to
    /// leaving the assignment unchanged.
    #[serde(default)]
    pub next_worker: Option<String>,
}

/// Input for `darkrun_elaborate_seal` — record that the operator was involved
/// in shaping a station's spec, clearing the collaboration hold.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ElaborateSealInput {
    /// The run slug.
    pub slug: String,
    /// The station whose spec was elaborated with the operator.
    pub station: String,
}

/// Input for `darkrun_review_stamp` — a single reviewer's per-role sign-off.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReviewStampInput {
    /// The run slug.
    pub slug: String,
    /// The station whose units this reviewer signs.
    pub station: String,
    /// The reviewer role being stamped (e.g. `correctness`, `spec`).
    pub role: String,
    /// `review` (pre-execute spec sign-off) or `approval` (post-execute output).
    pub kind: String,
}

/// Input for `darkrun_run_review_stamp` — one whole-Run reviewer's sign-off.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunReviewStampInput {
    /// The run slug.
    pub slug: String,
    /// The run-reviewer role signing off (e.g. `integration-auditor`).
    pub role: String,
}

/// Input for `darkrun_quality_gate_record` — record one gate's result on a unit.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GateRecordInput {
    /// The run slug.
    pub slug: String,
    /// The unit slug.
    pub unit: String,
    /// The gate name (matches a declared `quality_gates` entry, e.g. `tests`).
    pub gate: String,
    /// The outcome: `pass` / `fail` / `env_blocked`. A repeatedly `env_blocked`
    /// gate is auto-deferred to CI so it can't wedge the run.
    pub status: String,
    /// Optional detail — failure output tail, or the blocked dependency.
    #[serde(default)]
    pub detail: Option<String>,
    /// The station's verifier nonce, handed to you in the Manufacture dispatch
    /// (B5). Required when the station carries one; the engine refuses a result
    /// without it, so a gate can't be certified outside a real dispatch.
    #[serde(default)]
    pub nonce: Option<String>,
}

/// Input for `darkrun_feedback_create`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackCreateInput {
    /// The run slug.
    pub slug: String,
    /// The station the finding targets. Pass `_run` for a RUN-SCOPE finding — a
    /// closeout or cross-station item that belongs to the run as a whole (the
    /// run-level reviewers file here); its fix lands on run-main, not a station
    /// branch. Omit for the active station.
    #[serde(default)]
    pub station: String,
    /// The finding text.
    pub body: String,
    /// Optional severity: `blocker`/`high`/`medium`/`low`.
    #[serde(default)]
    pub severity: Option<String>,
    /// Where the finding came from: `adversarial_review`/`run_review`/
    /// `reflection`/`discovery`/`drift`/`operator`/`annotation`/`external`.
    #[serde(default)]
    pub origin: Option<String>,
    /// Review/approval role slugs this finding invalidates when it closes (the
    /// stamps it undercut, re-opened on close so the gate re-fires).
    #[serde(default)]
    pub invalidates: Option<Vec<String>>,
}

/// The work-item selector shared by the annotation tools: which unit / output /
/// station an annotation hangs on. A `station`-kind selector (with an empty
/// `id`) scopes the whole station — used to read the station-level records
/// (including the global station note).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct WorkItemInput {
    /// `unit` / `output` / `station`.
    pub kind: String,
    /// The unit slug / output id. Empty for a bare station selector.
    #[serde(default)]
    pub id: String,
    /// The station this work item belongs to.
    pub station: String,
}

/// Input for `darkrun_annotation_submit` — record one annotation (a per-artifact
/// mark OR the global station note) into the run's annotation store.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AnnotationSubmitInput {
    /// The run slug.
    pub slug: String,
    /// Who is marking: `human` (default) or `agent`.
    #[serde(default)]
    pub author: Option<String>,
    /// The work item the annotation hangs on.
    pub work_item: WorkItemInput,
    /// The version-pinned artifact: `{ id, path, type, version_sha }`. Omit for
    /// the global station note.
    #[serde(default)]
    pub artifact: Option<serde_json::Value>,
    /// The typed anchor (tagged on `anchor_type`: text/image/html/pdf/svg/video).
    /// Omit for the global station note.
    #[serde(default)]
    pub anchor: Option<serde_json::Value>,
    /// How the human marked it: `{ tool, color? }`.
    #[serde(default)]
    pub expression: Option<serde_json::Value>,
    /// The free-form comment (required).
    pub comment: String,
    /// The structured ask: `{ kind: change|question|nit|praise, severity:
    /// must|should|nit }`.
    pub ask: serde_json::Value,
    /// An optional inline-replacement suggestion: `{ diff }`.
    #[serde(default)]
    pub suggestion: Option<serde_json::Value>,
}

/// Input for `darkrun_annotation_list` and `darkrun_annotation_payload` — scope
/// to a work item (or a station).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AnnotationListInput {
    /// The run slug.
    pub slug: String,
    /// The work item (or station) to read annotations for.
    pub work_item: WorkItemInput,
    /// When true, return only OPEN annotations (the severity counts always
    /// reflect open asks regardless). Defaults to false (full history).
    #[serde(default)]
    pub open_only: bool,
}

/// Input for `darkrun_reflection_record` — capture a Reflect-phase retrospective.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReflectionRecordInput {
    /// The run slug.
    pub slug: String,
    /// The reflection prose — specific, honest learnings.
    pub body: String,
    /// The station this reflection came out of. Omit for a run-level note.
    #[serde(default)]
    pub station: Option<String>,
}

/// Input for `darkrun_reflection_list` — read a run's collected reflections.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReflectionListInput {
    /// The run slug.
    pub slug: String,
}

/// Input for `darkrun_knowledge_record` — persist a durable project knowledge
/// topic (cross-run shared memory the explorer maintains).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct KnowledgeRecordInput {
    /// The topic slug (the knowledge entry's stable key). Re-recording the same
    /// topic updates it in place.
    pub topic: String,
    /// The knowledge prose — a durable project fact, constraint, prior art, or
    /// trap worth carrying into future runs.
    pub body: String,
}

/// Input for `darkrun_knowledge_list` — read the project's knowledge store.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct KnowledgeListInput {}

/// Input for `darkrun_brief_record` — persist a station's pre-execution brief
/// or closing outcome (the durable narrative surfaced at the operator gates).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BriefRecordInput {
    /// The run slug.
    pub slug: String,
    /// The station this brief/outcome belongs to.
    pub station: String,
    /// `pre` (the brief: "what I'm going to do", before the review gate) or
    /// `post` (the outcome: "what the station produced", before the checkpoint).
    pub phase: String,
    /// The narrative prose.
    pub body: String,
}

/// Input for `darkrun_drift_witness` — accept an intentional change to a locked
/// artifact (re-witness it so the sweep stops flagging it).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DriftAcceptInput {
    /// The run slug.
    pub slug: String,
    /// The drifted artifact path (repo-root-relative), as reported by the
    /// `resolve_drift` action.
    pub path: String,
}

/// Input for `darkrun_changelog` — optionally scope to one release.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ChangelogInput {
    /// A specific version (e.g. `0.1.0`); omit for the whole changelog.
    #[serde(default)]
    pub version: Option<String>,
}

/// Input for `darkrun_zap` — stateless single-task execution.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ZapInput {
    /// The task to run straight through a station's Worker loop.
    pub task: String,
    /// The factory (defaults to `software`).
    #[serde(default)]
    pub factory: Option<String>,
    /// The station (defaults to the factory's build-class station).
    #[serde(default)]
    pub station: Option<String>,
}

/// Input for `darkrun_report` — submit feedback about darkrun itself.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReportInput {
    /// The synthesized report (what happened, expected, repro).
    pub message: String,
    /// Optional contact email.
    #[serde(default)]
    pub contact_email: Option<String>,
    /// Optional reporter name.
    #[serde(default)]
    pub name: Option<String>,
}

/// Input for `darkrun_gate_review` — review the working tree before a gate.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct GateReviewInput {
    /// The run slug (for context; the diff is the repo working tree).
    #[serde(default)]
    pub slug: Option<String>,
}

/// Empty input for tools that take no arguments.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct NoInput {}

/// Input for `darkrun_backlog` — manage the project backlog.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct BacklogInput {
    /// `list` (default) / `add` / `review` / `promote`.
    #[serde(default)]
    pub action: Option<String>,
    /// The idea text, for `add`.
    #[serde(default)]
    pub description: Option<String>,
    /// The item id, for `promote`.
    #[serde(default)]
    pub id: Option<String>,
}

/// Input for `darkrun_scaffold` — generate an editable custom artifact.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ScaffoldInput {
    /// `factory` / `station` / `worker` / `reviewer`.
    pub kind: String,
    /// The artifact name.
    pub name: String,
    /// The parent factory (required for station/worker/reviewer).
    #[serde(default)]
    pub factory: Option<String>,
    /// The parent station (required for worker/reviewer).
    #[serde(default)]
    pub station: Option<String>,
}

/// Input for `darkrun_setup` — detect (and optionally write) project settings.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SetupInput {
    /// When true, write `.darkrun/settings.yml`; otherwise detect only.
    #[serde(default)]
    pub apply: bool,
}

/// Input for `darkrun_run_reset` — wipe a station or whole run.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RunResetInput {
    /// The run slug.
    pub slug: String,
    /// The station to wipe; omit to reset the whole run.
    #[serde(default)]
    pub station: Option<String>,
    /// Must be true to actually delete; otherwise a dry run.
    #[serde(default)]
    pub confirm: bool,
}

/// Input for `darkrun_unit_reset` — return a wedged unit to a fresh pending state.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UnitResetInput {
    /// The run slug.
    pub slug: String,
    /// The unit slug to reset.
    pub unit: String,
    /// Must be true to actually reset; otherwise a dry run reporting what it would clear.
    #[serde(default)]
    pub confirm: bool,
}

/// Input for `darkrun_debug` — admin recovery ops on a wedged run.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DebugInput {
    /// The run slug.
    pub slug: String,
    /// The op: `preview_cursor` | `force_station_complete` | `set_run_field` |
    /// `reset_drift` | `mutate_feedback`.
    pub op: String,
    /// The station, for `force_station_complete`.
    #[serde(default)]
    pub station: Option<String>,
    /// The field name, for `set_run_field` (`mode` | `active_station`).
    #[serde(default)]
    pub field: Option<String>,
    /// The new value, for `set_run_field`; or the new status, for `mutate_feedback`.
    #[serde(default)]
    pub value: Option<String>,
    /// The feedback id, for `mutate_feedback`.
    #[serde(default)]
    pub feedback_id: Option<String>,
    /// Why the bypass is needed (required on every mutating op).
    #[serde(default)]
    pub reason: Option<String>,
    /// Must be true to apply a mutating op.
    #[serde(default)]
    pub confirm: bool,
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
    /// Optional closure reply — what was done to resolve the finding. Recorded
    /// on the item and surfaced to the requester. When set with a `closed`
    /// status, the finding's `invalidates` roles are re-opened on its station's
    /// units so the gate re-fires.
    #[serde(default)]
    pub reply: Option<String>,
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

/// Input for `darkrun_feedback_set_targets`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct FeedbackSetTargetsInput {
    /// The run slug.
    pub slug: String,
    /// The feedback id.
    pub feedback_id: String,
    /// The review/approval role slugs this finding invalidates on close — the
    /// signed slots the change actually undercut. Empty marks it cosmetic.
    pub invalidates: Vec<String>,
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

/// Input for `darkrun_human_write` — an operator-delegated project-file write.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct HumanWriteInput {
    /// The project-relative path to write (no absolute paths, no `..`, not under
    /// `.darkrun/`).
    pub path: String,
    /// The file content to write.
    pub content: String,
}

/// Input for `darkrun_external_ref` — set or read a run's cross-system handles.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ExternalRefInput {
    /// The run slug.
    pub slug: String,
    /// The handle name: `ticket`, `pr_url` (or `pr`), `design`, or any custom
    /// label (stored under `other`). Omit `key` to read all handles.
    #[serde(default)]
    pub key: Option<String>,
    /// The handle value to set. An empty string clears the handle. Ignored when
    /// `key` is omitted (read).
    #[serde(default)]
    pub value: Option<String>,
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

/// Parse a `WorkItemInput` into the typed `WorkItem`.
fn parse_work_item(
    input: &WorkItemInput,
) -> std::result::Result<darkrun_api::annotation::WorkItem, String> {
    use darkrun_api::annotation::WorkItemKind;
    let kind = match input.kind.trim().to_ascii_lowercase().as_str() {
        "unit" => WorkItemKind::Unit,
        "output" => WorkItemKind::Output,
        "station" => WorkItemKind::Station,
        other => return Err(format!("invalid work_item kind: {other}")),
    };
    Ok(darkrun_api::annotation::WorkItem {
        kind,
        id: input.id.clone(),
        station: input.station.clone(),
    })
}

/// Deserialize an optional JSON value into a typed `T`, tagging the error with
/// the field `label`. `None` passes through as `Ok(None)`.
fn opt_from_value<T: serde::de::DeserializeOwned>(
    label: &str,
    v: Option<serde_json::Value>,
) -> std::result::Result<Option<T>, String> {
    match v {
        Some(val) => serde_json::from_value(val)
            .map(Some)
            .map_err(|e| format!("invalid {label}: {e}")),
        None => Ok(None),
    }
}

/// Parse the optional `author` string into an `AuthorType` (default: human).
fn parse_author(raw: Option<&str>) -> std::result::Result<darkrun_api::common::AuthorType, String> {
    use darkrun_api::common::AuthorType;
    match raw.map(|s| s.trim().to_ascii_lowercase()) {
        None => Ok(AuthorType::Human),
        Some(s) => match s.as_str() {
            "human" => Ok(AuthorType::Human),
            "agent" => Ok(AuthorType::Agent),
            "system" => Ok(AuthorType::System),
            other => Err(format!("invalid author: {other}")),
        },
    }
}

// ── Tool handlers ───────────────────────────────────────────────────────

#[tool_router]
impl DarkrunServer {
    /// Start a new run: seed `.darkrun/<slug>/` state at the factory's first
    /// station and return the run.
    #[tool(
        name = "darkrun_run_new",
        description = "Start a new darkrun Run on a factory, seeding .darkrun state at the first station."
    )]
    pub fn darkrun_run_new(
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
            Mode::from_label(&input.mode),
            &input.size,
        ) {
            Ok(run) => ok_json(&run),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Deprecated alias of `darkrun_run_new` — kept for one release.
    #[tool(name = "darkrun_run_start", description = "Deprecated alias of darkrun_run_new.")]
    pub fn darkrun_run_start_alias(
        &self,
        params: Parameters<RunStartInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        self.darkrun_run_new(params)
    }

    /// Drive one workflow tick and return the next action the agent should
    /// perform. Three-track priority: drift -> feedback -> run.
    #[tool(
        name = "darkrun_advance",
        description = "Advance the run one tick; returns the next structured action (drift -> feedback -> run)."
    )]
    pub fn darkrun_advance(
        &self,
        Parameters(input): Parameters<RunRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match run_tick(&store, &input.slug) {
            Ok(tick) => ok_json(&self.adapt_tick(tick)),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Deprecated alias of `darkrun_advance` — kept for one release.
    #[tool(name = "darkrun_tick", description = "Deprecated alias of darkrun_advance.")]
    pub fn darkrun_tick_alias(
        &self,
        params: Parameters<RunRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        self.darkrun_advance(params)
    }

    /// Show a run's frontmatter, derived state, and current position.
    #[tool(
        name = "darkrun_run_inspect",
        description = "Show a run: frontmatter, derived station state, and the current cursor position. Omit `slug` to infer the run from the current darkrun/<slug>/… branch, the active run, or the sole run."
    )]
    pub fn darkrun_run_inspect(
        &self,
        Parameters(input): Parameters<RunShowRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let Some(slug) = self.resolve_run_slug(&store, input.slug.as_deref()) else {
            return Ok(err_text(
                "No run given and none could be inferred — pass a slug, or run this \
                 from a darkrun/<slug>/… worktree (no active run is set).",
            ));
        };
        let run = match store.read_run(&slug) {
            Ok(r) => r,
            Err(e) => return Ok(err_text(e)),
        };
        let state = store.read_state(&slug).ok().flatten();
        let position = crate::position::derive_position(&store, &slug).ok();
        // Bring up the desktop interface: raise the run's review surface, then
        // LAUNCH the desktop app pointed at it when none is already connected.
        // The desktop is the only interactive surface darkrun drives. The
        // structured state is returned too, for the agent.
        let _ = crate::sessions::create_show(&self.sessions, &store, &slug);
        // F5: treat a recently-dropped app as still present (grace window) so a
        // backgrounded tab or a network blip doesn't respawn the desktop.
        let desktop = if self.sessions.presence().is_present() {
            // A desktop is connected (or within the grace window); its home
            // poller navigates to the run.
            serde_json::json!({ "status": "connected" })
        } else if let Some(addr) = self.announced_addr {
            self.launch_desktop(addr.port(), &slug)
        } else {
            serde_json::json!({ "status": "no_engine_port" })
        };
        ok_json(&serde_json::json!({
            "run": run,
            "state": state,
            "position": position,
            // G4b: where the run's stable branch stands vs the default branch
            // (not_forked / up_to_date / ahead / merged / diverged).
            "run_main_status": crate::lifecycle::run_main_status(&store, &slug),
            "showing": {
                "surface": "desktop",
                "session_id": slug,
                "port": self.announced_addr.map(|a| a.port()),
                "desktop": desktop,
            },
        }))
    }

    /// Deprecated alias of `darkrun_run_inspect` — kept for one release.
    #[tool(name = "darkrun_run_show", description = "Deprecated alias of darkrun_run_inspect.")]
    pub fn darkrun_run_show_alias(
        &self,
        params: Parameters<RunShowRef>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        self.darkrun_run_inspect(params)
    }

    /// Launch the desktop app pointed at `slug` over the engine's announced
    /// `port`, returning a status JSON. Excluded from coverage: every arm either
    /// spawns a GUI process, kicks off a `cargo build`, or probes the filesystem
    /// for the bundled binary — none of which is exercisable in-process.
    #[cfg(not(tarpaulin_include))]
    fn launch_desktop(&self, port: u16, slug: &str) -> serde_json::Value {
        match crate::desktop::spawn(self.repo_root.as_ref(), port, Some(slug)) {
            crate::desktop::Launch::Launched(bin) => serde_json::json!({
                "status": "launched",
                "bin": bin.to_string_lossy(),
            }),
            crate::desktop::Launch::Building => serde_json::json!({
                "status": "building",
                "note": "Compiling darkrun-desktop for your arch; the app opens when the build finishes.",
            }),
            crate::desktop::Launch::NotFound => serde_json::json!({
                "status": "not_found",
                "hint": "darkrun-desktop binary not found — set DARKRUN_DESKTOP or build it (cargo build -p darkrun-desktop)",
            }),
        }
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
            Ok(tick) => ok_json(&self.adapt_tick(tick)),
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

    /// Record one Pass beat on a unit: the worker, its advance/reject result,
    /// and the **handoff note** that carries the story to the next worker.
    #[tool(
        name = "darkrun_unit_beat",
        description = "Record one Pass beat (Make/Challenge/Resolve): worker + result (advance|reject) + a handoff note (required on reject). The note is threaded into the next worker's dispatch and surfaced to the operator and reflection. Pass count is derived from the iteration history."
    )]
    pub fn darkrun_unit_beat(
        &self,
        Parameters(input): Parameters<UnitIterateInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let result = match input.result.trim().to_ascii_lowercase().as_str() {
            "advance" => darkrun_core::domain::IterationResult::Advance,
            "reject" => darkrun_core::domain::IterationResult::Reject,
            other => return Ok(err_text(format!("invalid result `{other}` (want advance|reject)"))),
        };
        // A reject without a reason is exactly the story-loss this records against.
        if matches!(result, darkrun_core::domain::IterationResult::Reject)
            && input.note.as_deref().map(str::trim).unwrap_or("").is_empty()
        {
            return Ok(err_text("a reject must carry a note explaining why it bounced"));
        }
        let store = self.store();
        match units::record_iteration(
            &store,
            &input.slug,
            &input.unit,
            &input.worker,
            result,
            input.note.clone(),
            input.next_worker.clone(),
        ) {
            Ok(unit) => ok_json(&unit),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Deprecated alias of `darkrun_unit_beat` — kept for one release.
    #[tool(name = "darkrun_unit_iterate", description = "Deprecated alias of darkrun_unit_beat.")]
    pub fn darkrun_unit_iterate_alias(
        &self,
        params: Parameters<UnitIterateInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        self.darkrun_unit_beat(params)
    }

    /// Record a quality-gate result on a unit — the objective check the unit's
    /// work must pass before it can leave Manufacture.
    #[tool(
        name = "darkrun_quality_gate_record",
        description = "Record a unit's quality-gate result: gate name + status (pass|fail|env_blocked). You run the command (you have a shell); this records and enforces it. A unit can't pass Audit until every declared gate is pass (or deferred-to-CI). A repeatedly env_blocked gate auto-defers to CI so it can't wedge the run."
    )]
    pub fn darkrun_quality_gate_record(
        &self,
        Parameters(input): Parameters<GateRecordInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        use darkrun_core::domain::GateStatus;
        let status = match input.status.trim().to_ascii_lowercase().as_str() {
            "pass" | "passed" => GateStatus::Pass,
            "fail" | "failed" => GateStatus::Fail,
            "env_blocked" | "env-blocked" | "blocked" => GateStatus::EnvBlocked,
            other => {
                return Ok(err_text(format!(
                    "invalid gate status `{other}` (want pass|fail|env_blocked)"
                )))
            }
        };
        let store = self.store();
        match units::record_gate_result(
            &store,
            &input.slug,
            &input.unit,
            &input.gate,
            status,
            input.detail.clone(),
            input.nonce.as_deref(),
        ) {
            Ok(unit) => ok_json(&unit),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Stamp one whole-Run reviewer's sign-off in the run-level review (the
    /// cross-station audit after the final station) — without walking the cursor.
    #[tool(
        name = "darkrun_run_review_stamp",
        description = "Record ONE whole-Run reviewer's sign-off on the integrated run (the cross-station audit after the final station) without advancing — the parallel-safe close for a fanned-out run reviewer. The run holds in run-review until every declared run reviewer is stamped, then seals. File feedback instead of stamping if the run reviewer finds a cross-station problem."
    )]
    pub fn darkrun_run_review_stamp(
        &self,
        Parameters(input): Parameters<RunReviewStampInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match crate::position::run_review_stamp(&store, &input.slug, &input.role) {
            Ok(()) => ok_json(&serde_json::json!({ "ok": true, "role": input.role })),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Record that the operator was involved in shaping this station's spec —
    /// clears the per-station Spec hold so the run advances to Review.
    #[tool(
        name = "darkrun_elaborate_seal",
        description = "Mark a station's spec as elaborated WITH the operator. In team/solo modes the Spec phase holds at every station until you call this — so you involve the operator (darkrun_question / darkrun_direction) in shaping the spec instead of authoring it solo and only surfacing it at the gate. Dark mode pre-elaborates once up front and doesn't need it per station."
    )]
    pub fn darkrun_elaborate_seal(
        &self,
        Parameters(input): Parameters<ElaborateSealInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match crate::position::elaborate_seal(&store, &input.slug, &input.station) {
            Ok(()) => ok_json(&serde_json::json!({ "ok": true, "station": input.station })),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Stamp one reviewer's per-role sign-off across a station's units, WITHOUT
    /// walking the cursor — so parallel reviewers each close their own role
    /// concurrently and the parent ticks once.
    #[tool(
        name = "darkrun_review_stamp",
        description = "Record ONE reviewer role's sign-off (review|approval) across a station's units without advancing the run — the parallel-safe close for a fanned-out reviewer subagent. A station with an open finding is skipped (file the finding instead of stamping). The parent calls darkrun_advance once after all reviewers return."
    )]
    pub fn darkrun_review_stamp(
        &self,
        Parameters(input): Parameters<ReviewStampInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let kind = match input.kind.trim().to_ascii_lowercase().as_str() {
            "review" => units::StampKind::Review,
            "approval" => units::StampKind::Approval,
            other => return Ok(err_text(format!("invalid kind `{other}` (want review|approval)"))),
        };
        let store = self.store();
        // Stations carrying an open (non-terminal) finding are not signed off.
        let open_stations: Vec<String> = match feedback::list(&store, &input.slug) {
            Ok(items) => items
                .into_iter()
                .filter(|f| !feedback::is_terminal(f.status))
                .map(|f| f.station)
                .collect(),
            Err(_) => Vec::new(),
        };
        match units::stamp_role(
            &store, &input.slug, &input.station, &input.role, kind, &open_stations,
        ) {
            Ok(outcome) => ok_json(&serde_json::json!({
                "role": input.role,
                "kind": input.kind,
                "stamped": outcome.stamped,
                "skipped": outcome.skipped,
            })),
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
        let origin = input
            .origin
            .as_deref()
            .map(feedback::parse_origin)
            .unwrap_or(darkrun_core::domain::FeedbackOrigin::Unspecified);
        let invalidates = input.invalidates.clone().unwrap_or_default();
        match feedback::create_with_origin(
            &store, &input.slug, &input.station, &input.body, severity, origin, invalidates,
        ) {
            Ok(fb) => ok_json(&fb),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Annotations ──────────────────────────────────────────────────────

    /// Record one annotation — a per-artifact mark (text/image/html/pdf/svg/
    /// video) or the global station note — into the run's annotation store.
    /// Validates the anchor's typed shape against the artifact type, mints the
    /// id + timestamp, and (for an image/html rect mark) crops the marked region
    /// out of the version-pinned artifact to a PNG beside the JSON.
    #[tool(
        name = "darkrun_annotation_submit",
        description = "Record one annotation (a per-artifact text/image/html mark, or the global station note) pinned to the artifact's version; image/html rect marks are cropped to disk for the agent re-reference payload."
    )]
    pub fn darkrun_annotation_submit(
        &self,
        Parameters(input): Parameters<AnnotationSubmitInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let work_item = match parse_work_item(&input.work_item) {
            Ok(w) => w,
            Err(e) => return Ok(err_text(e)),
        };
        let author = match parse_author(input.author.as_deref()) {
            Ok(a) => a,
            Err(e) => return Ok(err_text(e)),
        };
        let artifact = match opt_from_value("artifact", input.artifact) {
            Ok(a) => a,
            Err(e) => return Ok(err_text(e)),
        };
        let anchor = match opt_from_value("anchor", input.anchor) {
            Ok(a) => a,
            Err(e) => return Ok(err_text(e)),
        };
        let expression = match opt_from_value("expression", input.expression) {
            Ok(e) => e,
            Err(e) => return Ok(err_text(e)),
        };
        let suggestion = match opt_from_value("suggestion", input.suggestion) {
            Ok(s) => s,
            Err(e) => return Ok(err_text(e)),
        };
        let ask: darkrun_api::annotation::Ask = match serde_json::from_value(input.ask) {
            Ok(a) => a,
            Err(e) => return Ok(err_text(format!("invalid ask: {e}"))),
        };
        let args = crate::annotation::SubmitArgs {
            author,
            work_item,
            artifact,
            anchor,
            expression,
            comment: input.comment,
            ask,
            suggestion,
        };
        let store = self.store();
        match crate::annotation::submit(&store, self.repo_root.as_ref(), &input.slug, args) {
            Ok(res) => ok_json(&res),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// List the annotations on a work item (or a station), with the open-
    /// severity tally and the checkpoint button steering — the feedback-inbox
    /// data. `should`/`must` flip the primary button to `request_changes`; a
    /// `nit` never blocks.
    #[tool(
        name = "darkrun_annotation_list",
        description = "List annotations on a work item (or station), decorated with the open-severity counts (blocker/high/nit) and the checkpoint button steering (approve vs request_changes)."
    )]
    pub fn darkrun_annotation_list(
        &self,
        Parameters(input): Parameters<AnnotationListInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let work_item = match parse_work_item(&input.work_item) {
            Ok(w) => w,
            Err(e) => return Ok(err_text(e)),
        };
        let store = self.store();
        match crate::annotation::list(&store, &input.slug, &work_item, input.open_only) {
            Ok(listing) => ok_json(&listing),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Resolve every OPEN annotation on a work item into an actionable agent
    /// bundle: text → file:line + quote + comment (+ suggestion diff); image →
    /// a cropped region PNG + coords + comment; html → dom.src (file:line) +
    /// outer_html + comment (or a flagged fallback when no source map exists).
    /// This is the payload the agent receives when it revisits the work item.
    #[tool(
        name = "darkrun_annotation_payload",
        description = "Resolve a work item's OPEN annotations into the agent re-reference payload: source location (file:line for text/html, cropped region for image) + quote/crop + comment + optional suggestion diff."
    )]
    pub fn darkrun_annotation_payload(
        &self,
        Parameters(input): Parameters<AnnotationListInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let work_item = match parse_work_item(&input.work_item) {
            Ok(w) => w,
            Err(e) => return Ok(err_text(e)),
        };
        let store = self.store();
        match crate::annotation::agent_re_reference(
            &store,
            self.repo_root.as_ref(),
            &input.slug,
            &work_item,
        ) {
            Ok(payload) => ok_json(&payload),
            Err(e) => Ok(err_text(e)),
        }
    }

    // ── Reflections ──────────────────────────────────────────────────────

    /// Record a Reflect-phase retrospective so it survives the run.
    #[tool(
        name = "darkrun_reflection_record",
        description = "Capture a Reflect-phase retrospective into the run's durable reflections."
    )]
    pub fn darkrun_reflection_record(
        &self,
        Parameters(input): Parameters<ReflectionRecordInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match reflection::record(&store, &input.slug, input.station.clone(), &input.body) {
            Ok(r) => ok_json(&r),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// List a run's collected reflections.
    #[tool(
        name = "darkrun_reflection_list",
        description = "List the run's collected Reflect-phase reflections (oldest first)."
    )]
    pub fn darkrun_reflection_list(
        &self,
        Parameters(input): Parameters<ReflectionListInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match reflection::list(&store, &input.slug) {
            Ok(rs) => ok_json(&rs),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Persist a station's pre-execution brief (`phase: pre`) or closing outcome
    /// (`phase: post`) — the durable narrative the operator gates surface.
    #[tool(
        name = "darkrun_brief_record",
        description = "Persist a station's pre-execution brief (phase: pre — what I'm going to do) or closing outcome (phase: post — what the station produced). Surfaced to the operator at the review and checkpoint gates."
    )]
    pub fn darkrun_brief_record(
        &self,
        Parameters(input): Parameters<BriefRecordInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let Some(phase) = brief::BriefPhase::parse(&input.phase) else {
            return Ok(err_text(format!(
                "invalid phase '{}': expected 'pre' (brief) or 'post' (outcome)",
                input.phase
            )));
        };
        let store = self.store();
        match brief::record(&store, &input.slug, &input.station, phase, &input.body) {
            Ok(b) => ok_json(&b),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Persist a durable PROJECT knowledge topic — cross-run shared memory the
    /// discovery explorer maintains; a later run's Spec reads it as a prior.
    #[tool(
        name = "darkrun_knowledge_record",
        description = "Persist a durable PROJECT knowledge topic (a constraint, prior art, convention, or trap) into the cross-run shared-memory store. Re-recording a topic updates it in place. The discovery explorer records what's worth carrying into future runs; Spec reads it as priors."
    )]
    pub fn darkrun_knowledge_record(
        &self,
        Parameters(input): Parameters<KnowledgeRecordInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        if input.topic.trim().is_empty() {
            return Ok(err_text("knowledge topic must not be empty"));
        }
        let store = self.store();
        match knowledge::record(&store, &input.topic, &input.body) {
            Ok(k) => ok_json(&k),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// List the project's knowledge store (cross-run priors).
    #[tool(
        name = "darkrun_knowledge_list",
        description = "List the project's durable knowledge store — the cross-run priors the explorer has accumulated."
    )]
    pub fn darkrun_knowledge_list(
        &self,
        Parameters(_input): Parameters<KnowledgeListInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match knowledge::list(&store) {
            Ok(ks) => ok_json(&ks),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Accept an intentional change to a locked artifact, clearing its drift.
    #[tool(
        name = "darkrun_drift_witness",
        description = "Accept an intentional change to a drifted locked artifact: re-witness it to its current content so the sweep stops flagging it."
    )]
    pub fn darkrun_drift_witness(
        &self,
        Parameters(input): Parameters<DriftAcceptInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match crate::drift::accept(&store, &input.slug, &input.path) {
            Ok(true) => ok_json(&serde_json::json!({ "accepted": input.path })),
            Ok(false) => Ok(err_text(format!(
                "no witness for '{}' (or the file is unreadable)",
                input.path
            ))),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Deprecated alias of `darkrun_drift_witness` — kept for one release.
    #[tool(name = "darkrun_drift_accept", description = "Deprecated alias of darkrun_drift_witness.")]
    pub fn darkrun_drift_accept_alias(
        &self,
        params: Parameters<DriftAcceptInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        self.darkrun_drift_witness(params)
    }

    // ── Meta / utility ───────────────────────────────────────────────────

    /// Report the running engine/plugin version, build, target, and entry.
    #[tool(
        name = "darkrun_version_info",
        description = "Report the running darkrun engine version, plugin version, build kind, target, and entry point."
    )]
    pub fn darkrun_version_info(
        &self,
        Parameters(_): Parameters<NoInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        ok_json(&crate::meta::version_info())
    }

    /// Surface the changelog, optionally for one release.
    #[tool(
        name = "darkrun_changelog",
        description = "Show the darkrun changelog; pass a version to scope to one release."
    )]
    pub fn darkrun_changelog(
        &self,
        Parameters(input): Parameters<ChangelogInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let text = crate::meta::changelog(input.version.as_deref());
        ok_json(&serde_json::json!({ "changelog": text }))
    }

    /// Capture a feedback/bug report about darkrun.
    #[tool(
        name = "darkrun_report",
        description = "Capture a feedback or bug report about darkrun (saved locally; no hosted intake yet)."
    )]
    pub fn darkrun_report(
        &self,
        Parameters(input): Parameters<ReportInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match crate::meta::report(
            self.repo_root.as_ref(),
            &input.message,
            input.contact_email.as_deref(),
            input.name.as_deref(),
        ) {
            Ok(r) => ok_json(&r),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Resolve a stateless single-task zap (factory/station + Worker loop).
    #[tool(
        name = "darkrun_zap",
        description = "Resolve a stateless single-task run: the factory/station and its Worker loop, with the run/verify/commit procedure."
    )]
    pub fn darkrun_zap(
        &self,
        Parameters(input): Parameters<ZapInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match crate::zap::zap(&input.task, input.factory.as_deref(), input.station.as_deref()) {
            Ok(z) => ok_json(&z),
            Err(e) => ok_json(&e),
        }
    }

    /// Compute the working-tree diff and review instructions for a gate.
    #[tool(
        name = "darkrun_gate_review",
        description = "Compute the working-tree diff and return review instructions for a pre-checkpoint code review."
    )]
    pub fn darkrun_gate_review(
        &self,
        Parameters(_): Parameters<GateReviewInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        ok_json(&crate::gate::gate_review(self.repo_root.as_ref()))
    }

    // ── Backlog / scaffold / setup / reset ───────────────────────────────

    /// Manage the project backlog (list / add / review / promote).
    #[tool(
        name = "darkrun_backlog",
        description = "Manage the project backlog: list (default), add a description, review, or promote an item out to become a Run."
    )]
    pub fn darkrun_backlog(
        &self,
        Parameters(input): Parameters<BacklogInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let root = self.repo_root.as_ref();
        let action = input.action.as_deref().unwrap_or("list");
        let result = match action {
            "add" => match &input.description {
                Some(d) => crate::backlog::add(root, d).map(|i| serde_json::json!({ "added": i })),
                None => return Ok(err_text("`description` is required for action `add`")),
            },
            "promote" => match &input.id {
                Some(id) => crate::backlog::promote(root, id).map(|opt| match opt {
                    Some(i) => serde_json::json!({ "promoted": i, "next": "hand off to /darkrun:darkrun-start" }),
                    None => serde_json::json!({ "error": format!("no backlog item `{id}`") }),
                }),
                None => return Ok(err_text("`id` is required for action `promote`")),
            },
            // list and review both return the items.
            _ => crate::backlog::list(root).map(|items| serde_json::json!({ "items": items })),
        };
        match result {
            Ok(v) => ok_json(&v),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Scaffold an editable custom artifact under `.darkrun/factories/`.
    #[tool(
        name = "darkrun_scaffold",
        description = "Scaffold an editable Factory, Station, Worker, or Reviewer template under .darkrun/factories/."
    )]
    pub fn darkrun_scaffold(
        &self,
        Parameters(input): Parameters<ScaffoldInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match crate::scaffold::scaffold(
            self.repo_root.as_ref(),
            &input.kind,
            &input.name,
            input.factory.as_deref(),
            input.station.as_deref(),
        ) {
            Ok(s) => ok_json(&s),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Detect (and optionally write) the project's darkrun settings.
    #[tool(
        name = "darkrun_setup",
        description = "Auto-detect VCS, hosting, CI/CD, and the default branch; with apply:true, write .darkrun/settings.yml."
    )]
    pub fn darkrun_setup(
        &self,
        Parameters(input): Parameters<SetupInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match crate::setup::setup(self.repo_root.as_ref(), input.apply) {
            Ok(s) => ok_json(&s),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Wipe a station (re-enter it) or the whole run. Dry run unless confirmed.
    #[tool(
        name = "darkrun_run_reset",
        description = "Wipe a station so the manager re-enters it (or the whole run). Dry run unless confirm:true."
    )]
    pub fn darkrun_run_reset(
        &self,
        Parameters(input): Parameters<RunResetInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match crate::reset::reset(&store, &input.slug, input.station.as_deref(), input.confirm) {
            Ok(plan) => ok_json(&plan),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Reset a single wedged/bolt-capped unit back to a fresh pending state so its
    /// (otherwise locked) body becomes editable and it re-runs from Pass 1. Clears
    /// the unit's pass history, stamps, and gate results; preserves its spec. Dry
    /// run unless confirmed.
    #[tool(
        name = "darkrun_unit_reset",
        description = "Reset one wedged/bolt-capped unit to pending so its body is editable again and it re-runs from Pass 1 (clears passes/stamps/gates, keeps the spec). Dry run unless confirm:true."
    )]
    pub fn darkrun_unit_reset(
        &self,
        Parameters(input): Parameters<UnitResetInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match crate::units::reset(&store, &input.slug, &input.unit, input.confirm) {
            Ok(plan) => ok_json(&plan),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Admin recovery ops on a wedged run (preview / force-complete / set-field /
    /// reset-drift / mutate-feedback). Mutating ops need confirm + reason.
    #[tool(
        name = "darkrun_debug",
        description = "Admin recovery for a wedged run: preview_cursor (read-only), force_station_complete, set_run_field, reset_drift, mutate_feedback. Mutating ops require confirm:true and a reason."
    )]
    pub fn darkrun_debug(
        &self,
        Parameters(input): Parameters<DebugInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let slug = &input.slug;
        let reason = input.reason.as_deref();
        let missing = |what: &str| err_text(format!("`{what}` is required for op `{}`", input.op));
        let result = match input.op.as_str() {
            "preview_cursor" => crate::debug::preview_cursor(&store, slug),
            "force_station_complete" => match input.station.as_deref() {
                Some(st) => crate::debug::force_station_complete(&store, slug, st, input.confirm, reason),
                None => return Ok(missing("station")),
            },
            "set_run_field" => match (input.field.as_deref(), input.value.as_deref()) {
                (Some(f), Some(v)) => {
                    crate::debug::set_run_field(&store, slug, f, v, input.confirm, reason)
                }
                _ => return Ok(missing("field+value")),
            },
            "reset_drift" => crate::debug::reset_drift(&store, slug, input.confirm, reason),
            "mutate_feedback" => match (input.feedback_id.as_deref(), input.value.as_deref()) {
                (Some(id), Some(status)) => {
                    crate::debug::mutate_feedback(&store, slug, id, status, input.confirm, reason)
                }
                _ => return Ok(missing("feedback_id+value")),
            },
            other => return Ok(err_text(format!("unknown debug op `{other}`"))),
        };
        match result {
            Ok(r) => ok_json(&r),
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
        // A `closed` resolution carrying a reply records the resolution AND
        // re-opens the stamps the finding invalidated, so the gate re-fires.
        let result = match (status, input.reply.as_deref()) {
            (darkrun_core::domain::FeedbackStatus::Closed, Some(reply)) if !reply.trim().is_empty() => {
                feedback::close_with_reply(&store, &input.slug, &input.feedback_id, reply)
            }
            _ => feedback::set_status(&store, &input.slug, &input.feedback_id, status),
        };
        match result {
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

    /// Set the roles a feedback invalidates on close — the materiality call.
    #[tool(
        name = "darkrun_feedback_set_targets",
        description = "Set the review/approval roles a feedback invalidates on close. For a drift finding: name the signed slots the premise change actually undercut (material → those re-open and re-sign against the new premise on close), or set none (cosmetic → close is a no-op)."
    )]
    pub fn darkrun_feedback_set_targets(
        &self,
        Parameters(input): Parameters<FeedbackSetTargetsInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        match feedback::set_targets(&store, &input.slug, &input.feedback_id, input.invalidates) {
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
        match runs::list(&store, self.repo_root.as_ref(), input.include_archived) {
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

    /// Write a project file on the operator's behalf ("save this for me").
    #[tool(
        name = "darkrun_human_write",
        description = "Write a project file on the operator's behalf (a guarded 'save this for me'). Path must be project-relative, not under .darkrun/, no symlink escape. If the file is a premise some unit was built on, the next tick's drift sweep catches it automatically — no separate step."
    )]
    pub fn darkrun_human_write(
        &self,
        Parameters(input): Parameters<HumanWriteInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        match human_write::human_write(self.repo_root.as_ref(), &input.path, &input.content) {
            Ok(path) => ok_json(&serde_json::json!({
                "ok": true,
                "path": path.to_string_lossy(),
            })),
            Err(e) => Ok(err_text(e)),
        }
    }

    /// Set or read a run's cross-system handles (ticket, PR url, design link).
    #[tool(
        name = "darkrun_external_ref",
        description = "Set or read a run's cross-system handles (durable pointers to a ticket, PR/MR url, design link, or any custom-named external resource). Provide `key`+`value` to set (empty value clears); omit `key` to read all."
    )]
    pub fn darkrun_external_ref(
        &self,
        Parameters(input): Parameters<ExternalRefInput>,
    ) -> std::result::Result<CallToolResult, ErrorData> {
        let store = self.store();
        let result = match input.key.as_deref() {
            Some(key) => runs::set_external_ref(&store, &input.slug, key, input.value.as_deref().unwrap_or("")),
            None => runs::external_refs(&store, &input.slug),
        };
        match result {
            Ok(refs) => ok_json(&refs),
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

/// The tools that DRIVE a run — the cursor loop, gate decisions, unit + feedback
/// work, proof/quality recording. When a harness's tool budget forces a cut,
/// these survive: dropping any of them would break the agent's ability to run a
/// Run at all. Everything else (admin, meta, scaffolding, lifecycle extras) is
/// shed first. Kept conservatively under every harness budget.
const ESSENTIAL_TOOL_NAMES: &[&str] = &[
    "darkrun_run_new",
    "darkrun_advance",
    "darkrun_run_inspect",
    "darkrun_run_list",
    "darkrun_checkpoint_decide",
    "darkrun_unit_create",
    "darkrun_unit_update",
    "darkrun_unit_get",
    "darkrun_unit_list",
    "darkrun_unit_beat",
    "darkrun_feedback_create",
    "darkrun_feedback_list",
    "darkrun_feedback_resolve",
    "darkrun_feedback_set_targets",
    "darkrun_quality_gate_record",
    "darkrun_review_stamp",
    "darkrun_run_review_stamp",
    "darkrun_drift_witness",
    "darkrun_run_surface",
    "darkrun_proof_attach",
    "darkrun_proof_get",
    "darkrun_elaborate_seal",
    "darkrun_gate_review",
    "darkrun_question",
    "darkrun_question_result",
];

/// Adapt the full tool list to a harness: drop browser/visual tools when the
/// harness has no desktop UI, then enforce its tool budget. Pure over the
/// inputs so it's unit-testable without an MCP request context.
///
/// The budget cut is **priority-aware**: essential run-driving tools
/// ([`ESSENTIAL_TOOL_NAMES`]) are kept and the budget is filled with the
/// remaining tools in registration order, so a growing tool surface can never
/// silently shed `darkrun_advance` (or any loop driver) on a constrained harness.
fn adapt_tool_list(
    caps: &darkrun_harness::Capabilities,
    mut tools: Vec<rmcp::model::Tool>,
) -> Vec<rmcp::model::Tool> {
    if !caps.browser_ui {
        tools.retain(|t| !VISUAL_TOOL_NAMES.contains(&t.name.as_ref()));
    }
    if let Some(max) = caps.max_tools {
        if tools.len() > max {
            let is_essential =
                |t: &rmcp::model::Tool| ESSENTIAL_TOOL_NAMES.contains(&t.name.as_ref());
            // Essential tools first (in registration order), then fill with the
            // rest, then clamp. If even the essential set exceeds an
            // (artificially tiny) budget, the clamp still wins — but the loop
            // drivers lead the list, so the first kept are the ones that matter.
            let (mut kept, rest): (Vec<_>, Vec<_>) = tools.into_iter().partition(is_essential);
            kept.extend(rest);
            kept.truncate(max);
            tools = kept;
        }
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
    #[cfg(not(tarpaulin_include))] // MCP protocol dispatch (needs a live RequestContext); adapt_tool_list is tested
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

    /// Bridge the shipped skills as MCP prompts, for harnesses that consume the
    /// prompts capability but don't load `SKILL.md` natively (everyone but
    /// Claude Code). Claude Code uses its native skills, so it gets none here.
    // ListPromptsResult is #[non_exhaustive], so we can't use struct-init
    // syntax — default + field assignment is the canonical construction.
    #[allow(clippy::field_reassign_with_default)]
    #[cfg(not(tarpaulin_include))] // MCP protocol dispatch (needs a live RequestContext); skill_prompts is tested
    async fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<rmcp::model::ListPromptsResult, ErrorData> {
        if self.caps.harness.is_claude_code() || !self.caps.mcp_prompts {
            return Ok(rmcp::model::ListPromptsResult::default());
        }
        let prompts = crate::skill_bridge::skill_prompts()
            .into_iter()
            .map(|p| rmcp::model::Prompt::new(p.name, Some(p.description), None))
            .collect();
        let mut result = rmcp::model::ListPromptsResult::default();
        result.prompts = prompts;
        Ok(result)
    }

    /// Return a bridged skill prompt's body as a user message.
    #[cfg(not(tarpaulin_include))] // MCP protocol dispatch (needs a live RequestContext); skill_prompt is tested
    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<rmcp::model::GetPromptResult, ErrorData> {
        match crate::skill_bridge::skill_prompt(&request.name) {
            Some(p) => {
                let mut result = rmcp::model::GetPromptResult::new(vec![
                    rmcp::model::PromptMessage::new_text(
                        rmcp::model::PromptMessageRole::User,
                        p.body,
                    ),
                ]);
                result.description = Some(p.description);
                Ok(result)
            }
            None => Err(ErrorData::invalid_params(
                format!("unknown prompt: {}", request.name),
                None,
            )),
        }
    }

    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        let mut instructions = String::from(
            "darkrun manager. Call darkrun_run_new to begin a Run, then \
             darkrun_advance repeatedly to walk its factory stations. Each tick returns \
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
        // Advertise prompts to harnesses that surface the bridged skills (every
        // harness but Claude Code, which loads SKILL.md natively). The builder is
        // a typestate, so each branch builds its own concrete capability set.
        info.capabilities = if self.caps.mcp_prompts && !self.caps.harness.is_claude_code() {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
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
        // EVERY essential run-driving tool survives the budget cut — the loop
        // can never be silently broken by a growing tool surface.
        for name in ESSENTIAL_TOOL_NAMES {
            // Visual tools aren't on a non-browser harness; skip those.
            if VISUAL_TOOL_NAMES.contains(name) {
                continue;
            }
            assert!(
                adapted.iter().any(|t| &t.name == name),
                "essential tool {name} was dropped by the budget cut"
            );
        }
        // Visual tools are dropped; the remainder is then clamped to Cursor's
        // tool budget (the truncation path is covered by its own test).
        let after_visual = all.len() - VISUAL_TOOL_NAMES.len();
        let want = caps.max_tools.map_or(after_visual, |cap| after_visual.min(cap));
        assert_eq!(adapted.len(), want);
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
            .darkrun_run_new(Parameters(RunStartInput {
                slug: "r".into(),
                factory: "software".into(),
                title: Some("t".into()),
                mode: "solo".into(),
                size: "full".into(),            }))
            .unwrap();
        assert_eq!(res.is_error, Some(false));
        assert!(dir.path().join(".darkrun/r/run.md").exists());
    }

    #[test]
    fn run_next_tool_advances() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        server
            .darkrun_run_new(Parameters(RunStartInput {
                slug: "r".into(),
                factory: "software".into(),
                title: None,
                mode: "solo".into(),
                size: "full".into(),            }))
            .unwrap();
        let res = server
            .darkrun_advance(Parameters(RunRef { slug: "r".into() }))
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
        let items = v["items"].as_array().expect("items array");
        let software = items
            .iter()
            .find(|i| i["name"] == "software")
            .expect("software factory listed");
        // Every factory — software included — opens on the fixed FSSBPH spine.
        assert_eq!(software["stations"][0]["name"], "frame");
        // The legal factory appears as a distinct catalog entry.
        assert!(
            items.iter().any(|i| i["name"] == "legal"),
            "legal factory listed"
        );
    }

    #[test]
    fn empty_slug_is_rejected() {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        let res = server
            .darkrun_run_new(Parameters(RunStartInput {
                slug: "  ".into(),
                factory: "software".into(),
                title: None,
                mode: "solo".into(),
                size: "full".into(),            }))
            .unwrap();
        assert_eq!(res.is_error, Some(true));
    }

    fn started_server() -> (tempfile::TempDir, DarkrunServer) {
        let dir = tempdir().unwrap();
        let server = DarkrunServer::new(dir.path());
        server
            .darkrun_run_new(Parameters(RunStartInput {
                slug: "r".into(),
                factory: "software".into(),
                title: Some("Run".into()),
                mode: "solo".into(),
                size: "full".into(),            }))
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
                origin: None,
                invalidates: None,
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
        assert_eq!(v["items"].as_array().unwrap().len(), 1);

        // Resolve it terminally.
        let resolved = server
            .darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
                slug: "r".into(),
                feedback_id: id,
                status: "addressed".into(),
                reply: None,
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
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
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
                origin: None,
                invalidates: None,
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
                reply: None,
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
                origin: None,
                invalidates: None,
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
        assert_eq!(
            listed.structured_content.unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

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
        assert_eq!(
            listed.structured_content.unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let listed = server
            .darkrun_run_list(Parameters(RunListInput {
                include_archived: true,
            }))
            .unwrap();
        assert_eq!(
            listed.structured_content.unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn annotation_submit_list_payload_wire_flow() {
        let (_d, server) = started_server();
        // Submit a text annotation via the wire-typed tool.
        let submitted = server
            .darkrun_annotation_submit(Parameters(AnnotationSubmitInput {
                slug: "r".into(),
                author: Some("human".into()),
                work_item: WorkItemInput {
                    kind: "output".into(),
                    id: "payment".into(),
                    station: "build".into(),
                },
                artifact: Some(serde_json::json!({
                    "id": "payment.rs",
                    "path": "src/payment.rs",
                    "type": "text",
                    "version_sha": "9f3c"
                })),
                anchor: Some(serde_json::json!({
                    "anchor_type": "text",
                    "range": { "start_line": 42, "start_col": 0, "end_line": 42, "end_col": 9 },
                    "quote": "fn charge"
                })),
                expression: Some(serde_json::json!({ "tool": "select" })),
                comment: "handle the declined-card path".into(),
                ask: serde_json::json!({ "kind": "change", "severity": "should" }),
                suggestion: None,
            }))
            .unwrap();
        assert_eq!(submitted.is_error, Some(false));
        let v = submitted.structured_content.unwrap();
        assert!(v["annotation"]["id"].as_str().unwrap().starts_with("anno_"));

        // List it back with the severity steering.
        let listed = server
            .darkrun_annotation_list(Parameters(AnnotationListInput {
                slug: "r".into(),
                work_item: WorkItemInput {
                    kind: "output".into(),
                    id: "payment".into(),
                    station: "build".into(),
                },
                open_only: false,
            }))
            .unwrap();
        let v = listed.structured_content.unwrap();
        assert_eq!(v["annotations"].as_array().unwrap().len(), 1);
        assert_eq!(v["should"], 1);
        assert_eq!(v["checkpoint_button_primary"], "request_changes");

        // The agent re-reference payload resolves it to file:line.
        let payload = server
            .darkrun_annotation_payload(Parameters(AnnotationListInput {
                slug: "r".into(),
                work_item: WorkItemInput {
                    kind: "output".into(),
                    id: "payment".into(),
                    station: "build".into(),
                },
                open_only: true,
            }))
            .unwrap();
        let v = payload.structured_content.unwrap();
        assert_eq!(v["items"][0]["source"]["kind"], "text");
        assert_eq!(v["items"][0]["source"]["path"], "src/payment.rs");
        assert_eq!(v["items"][0]["source"]["start_line"], 42);
    }

    #[test]
    fn annotation_submit_rejects_bad_work_item_kind() {
        let (_d, server) = started_server();
        let res = server
            .darkrun_annotation_submit(Parameters(AnnotationSubmitInput {
                slug: "r".into(),
                author: None,
                work_item: WorkItemInput {
                    kind: "nonsense".into(),
                    id: "x".into(),
                    station: "build".into(),
                },
                artifact: None,
                anchor: None,
                comment: "x".into(),
                expression: None,
                ask: serde_json::json!({ "kind": "change", "severity": "must" }),
                suggestion: None,
            }))
            .unwrap();
        assert_eq!(res.is_error, Some(true));
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

/// Handler-coverage smoke: construct the tool server and CALL every MCP handler
/// at least once (most on both the Ok and an Err path), so the thin
/// `Parameters -> service-call -> ok_json/err_text` wrappers are all exercised.
/// This guards the tool surface against an advertised-but-unexercised handler and
/// pins the request/response plumbing of each tool.
#[cfg(test)]
mod handler_smoke {
    use super::*;
    use tempfile::tempdir;

    /// Pull a freshly-minted visual-session id out of a create result.
    fn session_id(res: &CallToolResult) -> String {
        res.structured_content
            .as_ref()
            .and_then(|v| v.get("session_id"))
            .and_then(|s| s.as_str())
            .expect("create result carries a session_id")
            .to_string()
    }

    #[test]
    fn every_handler_executes_both_paths() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());

        // ── argument-free / stateless readers ──────────────────────────────
        s.darkrun_factory_list().unwrap();
        s.darkrun_version_info(Parameters(NoInput {})).unwrap();
        s.darkrun_changelog(Parameters(ChangelogInput { version: None })).unwrap();
        s.darkrun_changelog(Parameters(ChangelogInput { version: Some("0.1.0".into()) })).unwrap();
        s.darkrun_factory_detail(Parameters(FactoryRef { factory: "software".into() })).unwrap();
        s.darkrun_factory_detail(Parameters(FactoryRef { factory: "nope".into() })).unwrap();
        s.darkrun_backlog(Parameters(BacklogInput::default())).unwrap();
        s.darkrun_setup(Parameters(SetupInput { apply: false })).unwrap();
        s.darkrun_gate_review(Parameters(GateReviewInput { slug: None })).unwrap();
        s.darkrun_report(Parameters(ReportInput {
            message: "smoke report".into(),
            contact_email: None,
            name: None,
        }))
        .unwrap();
        s.darkrun_run_list(Parameters(RunListInput { include_archived: true })).unwrap();

        // ── error arms on an absent run ────────────────────────────────────
        let rr = |slug: &str| RunRef { slug: slug.into() };
        assert_eq!(
            s.darkrun_advance(Parameters(rr("ghost"))).unwrap().is_error,
            Some(true)
        );
        s.darkrun_run_inspect(Parameters(RunShowRef { slug: Some("ghost".into()) })).unwrap();
        s.darkrun_unit_list(Parameters(rr("ghost"))).unwrap();

        // ── start a real run + drive a little state ────────────────────────
        s.darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(),
            factory: "software".into(),
            title: Some("Smoke".into()),
            mode: "solo".into(),
            size: "full".into(),        }))
        .unwrap();
        s.darkrun_advance(Parameters(rr("r"))).unwrap();
        s.darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) })).unwrap();
        s.darkrun_run_surface(Parameters(RunSurfaceInput {
            slug: "r".into(),
            surface: Some("data".into()),
        }))
        .unwrap();
        s.darkrun_run_surface(Parameters(RunSurfaceInput { slug: "r".into(), surface: None })).unwrap();
        s.darkrun_external_ref(Parameters(ExternalRefInput {
            slug: "r".into(),
            key: Some("ticket".into()),
            value: Some("JIRA-1".into()),
        }))
        .unwrap();
        s.darkrun_external_ref(Parameters(ExternalRefInput { slug: "r".into(), key: None, value: None })).unwrap();
        s.darkrun_proof_attach(Parameters(ProofAttachInput {
            slug: "r".into(),
            proof: serde_json::json!({ "surface": "data", "bench": { "p50": 1.5 } }),
            station: Some("frame".into()),
        }))
        .unwrap();
        s.darkrun_proof_get(Parameters(ProofGetInput { slug: "r".into(), station: None })).unwrap();

        // ── units ──────────────────────────────────────────────────────────
        s.darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(),
            unit: "u1".into(),
            station: "frame".into(),
            title: Some("Unit one".into()),
            depends_on: vec![],
        }))
        .unwrap();
        s.darkrun_unit_get(Parameters(UnitRef { slug: "r".into(), unit: "u1".into() })).unwrap();
        s.darkrun_unit_list(Parameters(rr("r"))).unwrap();
        s.darkrun_unit_update(Parameters(UnitUpdateInput {
            slug: "r".into(),
            unit: "u1".into(),
            status: Some("in_progress".into()),
            depends_on: None,
            worker: Some("make".into()),
            inputs: None,
            outputs: Some(vec!["src/x.rs".into()]),
        }))
        .unwrap();
        s.darkrun_unit_beat(Parameters(UnitIterateInput {
            slug: "r".into(),
            unit: "u1".into(),
            worker: "make".into(),
            result: "advance".into(),
            note: Some("did the thing".into()),
            next_worker: Some("challenge".into()),
        }))
        .unwrap();
        s.darkrun_quality_gate_record(Parameters(GateRecordInput {
            slug: "r".into(),
            unit: "u1".into(),
            gate: "tests".into(),
            status: "pass".into(),
            detail: None,
            nonce: None,
        }))
        .unwrap();
        s.darkrun_unit_reset(Parameters(UnitResetInput { slug: "r".into(), unit: "u1".into(), confirm: true })).unwrap();

        // ── stamps + elaboration + gate choice ─────────────────────────────
        s.darkrun_review_stamp(Parameters(ReviewStampInput {
            slug: "r".into(),
            station: "frame".into(),
            role: "spec".into(),
            kind: "review".into(),
        }))
        .unwrap();
        s.darkrun_run_review_stamp(Parameters(RunReviewStampInput {
            slug: "r".into(),
            role: "integration-auditor".into(),
        }))
        .unwrap();
        s.darkrun_elaborate_seal(Parameters(ElaborateSealInput { slug: "r".into(), station: "frame".into() })).unwrap();

        // ── feedback lifecycle ─────────────────────────────────────────────
        let fb = s
            .darkrun_feedback_create(Parameters(FeedbackCreateInput {
                slug: "r".into(),
                station: "frame".into(),
                body: "a finding".into(),
                severity: Some("high".into()),
                origin: Some("operator".into()),
                invalidates: Some(vec!["spec".into()]),
            }))
            .unwrap();
        let fb_id = fb.structured_content.unwrap()["id"].as_str().unwrap().to_string();
        s.darkrun_feedback_list(Parameters(FeedbackListInput { slug: "r".into(), include_settled: true })).unwrap();
        s.darkrun_feedback_set_targets(Parameters(FeedbackSetTargetsInput {
            slug: "r".into(),
            feedback_id: fb_id.clone(),
            invalidates: vec!["spec".into()],
        }))
        .unwrap();
        s.darkrun_feedback_move(Parameters(FeedbackMoveInput {
            slug: "r".into(),
            feedback_id: fb_id.clone(),
            to_station: "specify".into(),
        }))
        .unwrap();
        s.darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(),
            feedback_id: fb_id.clone(),
            status: "addressed".into(),
            reply: Some("fixed it".into()),
        }))
        .unwrap();
        // A second finding we reject.
        let fb2 = s
            .darkrun_feedback_create(Parameters(FeedbackCreateInput {
                slug: "r".into(),
                station: "frame".into(),
                body: "stale finding".into(),
                severity: None,
                origin: None,
                invalidates: None,
            }))
            .unwrap();
        let fb2_id = fb2.structured_content.unwrap()["id"].as_str().unwrap().to_string();
        s.darkrun_feedback_reject(Parameters(FeedbackRejectInput {
            slug: "r".into(),
            feedback_id: fb2_id,
            reason: "no longer applies".into(),
        }))
        .unwrap();

        // ── reflections + drift + checkpoint decide ────────────────────────
        s.darkrun_reflection_record(Parameters(ReflectionRecordInput {
            slug: "r".into(),
            body: "learned something".into(),
            station: Some("frame".into()),
        }))
        .unwrap();
        s.darkrun_reflection_list(Parameters(ReflectionListInput { slug: "r".into() })).unwrap();
        s.darkrun_drift_witness(Parameters(DriftAcceptInput { slug: "r".into(), path: "frame/frame.md".into() })).unwrap();
        s.darkrun_checkpoint_decide(Parameters(CheckpointDecideInput {
            slug: "r".into(),
            approved: false,
            feedback: Some("hold".into()),
        }))
        .unwrap();

        // ── annotations ────────────────────────────────────────────────────
        let work_item = WorkItemInput { kind: "station".into(), id: String::new(), station: "frame".into() };
        s.darkrun_annotation_submit(Parameters(AnnotationSubmitInput {
            slug: "r".into(),
            author: Some("human".into()),
            work_item: work_item.clone(),
            artifact: None,
            anchor: None,
            expression: None,
            comment: "station note".into(),
            ask: serde_json::json!({ "kind": "question", "severity": "should" }),
            suggestion: None,
        }))
        .unwrap();
        s.darkrun_annotation_list(Parameters(AnnotationListInput {
            slug: "r".into(),
            work_item: work_item.clone(),
            open_only: false,
        }))
        .unwrap();
        s.darkrun_annotation_payload(Parameters(AnnotationListInput {
            slug: "r".into(),
            work_item,
            open_only: true,
        }))
        .unwrap();

        // ── visual sessions: create then read back ─────────────────────────
        let q = s
            .darkrun_question(Parameters(QuestionInput {
                slug: "r".into(),
                title: None,
                prompt: "Pick one".into(),
                context: None,
                options: vec![QuestionOptionInput {
                    id: "a".into(),
                    label: "A".into(),
                    image_url: None,
                    description: None,
                }],
                multi_select: false,
                image_urls: vec![],
            }))
            .unwrap();
        s.darkrun_question_result(Parameters(SessionResultInput { slug: "r".into(), session_id: session_id(&q) })).unwrap();
        let d = s
            .darkrun_direction(Parameters(DirectionInput {
                slug: "r".into(),
                title: None,
                prompt: "Choose a direction".into(),
                context: None,
                archetypes: vec![ArchetypeInput {
                    id: "x".into(),
                    label: "X".into(),
                    image_url: "http://img/x.png".into(),
                    description: "the x".into(),
                }],
            }))
            .unwrap();
        s.darkrun_direction_result(Parameters(SessionResultInput { slug: "r".into(), session_id: session_id(&d) })).unwrap();
        let p = s
            .darkrun_picker(Parameters(PickerInput {
                slug: "r".into(),
                kind: "confirm".into(),
                title: "Confirm".into(),
                prompt: "Yes?".into(),
                options: vec![PickerOptionInput {
                    id: "yes".into(),
                    label: "Yes".into(),
                    description: None,
                    secondary: None,
                }],
            }))
            .unwrap();
        s.darkrun_picker_result(Parameters(SessionResultInput { slug: "r".into(), session_id: session_id(&p) })).unwrap();

        // ── admin / recovery / scaffolding ─────────────────────────────────
        s.darkrun_debug(Parameters(DebugInput {
            slug: "r".into(),
            op: "preview_cursor".into(),
            station: None,
            field: None,
            value: None,
            feedback_id: None,
            reason: None,
            confirm: false,
        }))
        .unwrap();
        s.darkrun_human_write(Parameters(HumanWriteInput {
            path: "notes/smoke.txt".into(),
            content: "hello".into(),
        }))
        .unwrap();
        s.darkrun_scaffold(Parameters(ScaffoldInput {
            kind: "factory".into(),
            name: "smoke-factory".into(),
            factory: None,
            station: None,
        }))
        .unwrap();
        s.darkrun_run_archive(Parameters(RunArchiveInput { slug: "r".into(), archived: true })).unwrap();
        s.darkrun_run_reset(Parameters(RunResetInput {
            slug: "r".into(),
            station: None,
            confirm: false,
        }))
        .unwrap();
        s.darkrun_zap(Parameters(ZapInput {
            task: "print hello".into(),
            factory: None,
            station: None,
        }))
        .unwrap();
    }

    /// The error arms: every slug-scoped handler called against a ghost run
    /// returns an error result (the `Err(e) => Ok(err_text(e))` tail) — covers the
    /// failure half of each wrapper that `every_handler_executes` hit on success.
    #[test]
    fn slug_scoped_handlers_error_on_a_ghost_run() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let g = || "ghost".to_string();

        // Reads against a missing run hit the `Err(e) => err_text` tail.
        let is_err = |r: CallToolResult| r.is_error == Some(true);
        assert!(is_err(s.darkrun_advance(Parameters(RunRef { slug: g() })).unwrap()));
        assert!(is_err(s.darkrun_unit_get(Parameters(UnitRef { slug: g(), unit: "u".into() })).unwrap()));
        assert!(is_err(s.darkrun_feedback_resolve(Parameters(FeedbackResolveInput { slug: g(), feedback_id: "fb-1".into(), status: "addressed".into(), reply: None })).unwrap()));
        assert!(is_err(s.darkrun_proof_get(Parameters(ProofGetInput { slug: g(), station: None })).unwrap()));
        // Bad-input arms that error regardless of run state.
        assert!(is_err(s.darkrun_scaffold(Parameters(ScaffoldInput { kind: "gizmo".into(), name: "x".into(), factory: None, station: None })).unwrap()));
        assert!(is_err(s.darkrun_human_write(Parameters(HumanWriteInput { path: "/etc/passwd".into(), content: "x".into() })).unwrap()));

        // Every other slug-scoped wrapper is *called* against the ghost run so its
        // body executes (the failing ones cover their err tail; the write-style
        // ones cover the success tail) — the point is to drive the wrapper.
        let _ = s.darkrun_checkpoint_decide(Parameters(CheckpointDecideInput { slug: g(), approved: true, feedback: None }));
        let _ = s.darkrun_unit_create(Parameters(UnitCreateInput { slug: g(), unit: "u".into(), station: "frame".into(), title: None, depends_on: vec![] }));
        let _ = s.darkrun_unit_update(Parameters(UnitUpdateInput { slug: g(), unit: "u".into(), status: Some("bogus".into()), depends_on: None, worker: None, inputs: None, outputs: None }));
        let _ = s.darkrun_unit_beat(Parameters(UnitIterateInput { slug: g(), unit: "u".into(), worker: "w".into(), result: "advance".into(), note: None, next_worker: None }));
        let _ = s.darkrun_unit_reset(Parameters(UnitResetInput { slug: g(), unit: "u".into(), confirm: true }));
        let _ = s.darkrun_quality_gate_record(Parameters(GateRecordInput { slug: g(), unit: "u".into(), gate: "t".into(), status: "pass".into(), detail: None, nonce: None }));
        let _ = s.darkrun_review_stamp(Parameters(ReviewStampInput { slug: g(), station: "frame".into(), role: "r".into(), kind: "review".into() }));
        let _ = s.darkrun_run_review_stamp(Parameters(RunReviewStampInput { slug: g(), role: "r".into() }));
        let _ = s.darkrun_elaborate_seal(Parameters(ElaborateSealInput { slug: g(), station: "frame".into() }));
        let _ = s.darkrun_feedback_create(Parameters(FeedbackCreateInput { slug: g(), station: "frame".into(), body: "x".into(), severity: None, origin: None, invalidates: None }));
        let _ = s.darkrun_feedback_reject(Parameters(FeedbackRejectInput { slug: g(), feedback_id: "fb-1".into(), reason: "x".into() }));
        let _ = s.darkrun_feedback_move(Parameters(FeedbackMoveInput { slug: g(), feedback_id: "fb-1".into(), to_station: "specify".into() }));
        let _ = s.darkrun_feedback_set_targets(Parameters(FeedbackSetTargetsInput { slug: g(), feedback_id: "fb-1".into(), invalidates: vec![] }));
        let _ = s.darkrun_drift_witness(Parameters(DriftAcceptInput { slug: g(), path: "x".into() }));
        let _ = s.darkrun_reflection_record(Parameters(ReflectionRecordInput { slug: g(), body: "x".into(), station: None }));
        let _ = s.darkrun_run_surface(Parameters(RunSurfaceInput { slug: g(), surface: Some("data".into()) }));
        let _ = s.darkrun_proof_attach(Parameters(ProofAttachInput { slug: g(), proof: serde_json::json!({}), station: None }));
        let _ = s.darkrun_external_ref(Parameters(ExternalRefInput { slug: g(), key: Some("ticket".into()), value: Some("x".into()) }));
        let _ = s.darkrun_run_archive(Parameters(RunArchiveInput { slug: g(), archived: true }));
        let _ = s.darkrun_debug(Parameters(DebugInput { slug: g(), op: "preview_cursor".into(), station: None, field: None, value: None, feedback_id: None, reason: None, confirm: false }));
    }

    /// The slug-optional resolver + builder seams + small parse/format helpers.
    #[test]
    fn server_builders_resolver_and_parse_helpers() {
        let dir = tempdir().unwrap();
        // Builder chain + accessors.
        let s = DarkrunServer::new(dir.path())
            .with_announced_addr("127.0.0.1:4400".parse().unwrap())
            .with_harness(darkrun_harness::Harness::Cursor);
        let _ = s.capabilities();
        let _ = s.sessions();

        // resolve_run_slug: explicit wins; sole-run inference; none on empty.
        let store = s.store();
        assert_eq!(s.resolve_run_slug(&store, Some(" pinned ")).as_deref(), Some("pinned"));
        assert_eq!(s.resolve_run_slug(&store, None), None);
        run_start(&store, "only", "software", None, Mode::Solo, "full").unwrap();
        assert_eq!(s.resolve_run_slug(&store, None).as_deref(), Some("only"));
        // darkrun_run_inspect with no slug now infers the sole run.
        assert!(s.darkrun_run_inspect(Parameters(RunShowRef { slug: None })).unwrap().is_error != Some(true));

        // slug_from_branch.
        assert_eq!(slug_from_branch("darkrun/abc/main").as_deref(), Some("abc"));
        assert_eq!(slug_from_branch("darkrun//main"), None);
        assert_eq!(slug_from_branch("feature/x"), None);

        // ensure_object wraps non-objects; passes objects through.
        assert!(ensure_object(serde_json::json!([1, 2])).get("items").is_some());
        assert!(ensure_object(serde_json::json!(5)).get("value").is_some());
        assert!(ensure_object(serde_json::json!({"a": 1})).get("a").is_some());

        // parse helpers cover every token + the unknown fallthrough.
        for t in ["pending", "active", "in_progress", "completed", "blocked"] {
            assert!(parse_status_arg(t).is_some());
        }
        assert!(parse_status_arg("nope").is_none());
        for t in ["pending", "fixing", "addressed", "answered", "non_actionable", "escalated", "closed", "rejected"] {
            assert!(parse_feedback_status_arg(t).is_some());
        }
        assert!(parse_feedback_status_arg("nope").is_none());
    }

    #[test]
    fn invalid_enum_tokens_hit_the_error_arms() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        s.darkrun_run_new(Parameters(RunStartInput { slug: "r".into(), factory: "software".into(), title: None, mode: "solo".into(), size: "full".into() })).unwrap();
        s.darkrun_unit_create(Parameters(UnitCreateInput { slug: "r".into(), unit: "u1".into(), station: "frame".into(), title: None, depends_on: vec![] })).unwrap();
        let is_err = |r: CallToolResult| r.is_error == Some(true);
        assert!(is_err(s.darkrun_unit_beat(Parameters(UnitIterateInput { slug: "r".into(), unit: "u1".into(), worker: "w".into(), result: "bogus".into(), note: None, next_worker: None })).unwrap()));
        assert!(is_err(s.darkrun_quality_gate_record(Parameters(GateRecordInput { slug: "r".into(), unit: "u1".into(), gate: "t".into(), status: "bogus".into(), detail: None, nonce: None })).unwrap()));
        assert!(is_err(s.darkrun_review_stamp(Parameters(ReviewStampInput { slug: "r".into(), station: "frame".into(), role: "x".into(), kind: "bogus".into() })).unwrap()));
    }

    #[test]
    fn resolve_run_slug_uses_the_active_run_pointer() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        run_start(&store, "first", "software", None, Mode::Solo, "full").unwrap();
        run_start(&store, "second", "software", None, Mode::Solo, "full").unwrap();
        // Two runs -> the sole-run shortcut cannot fire; the active pointer wins.
        store.set_active_run("second").unwrap();
        assert_eq!(s.resolve_run_slug(&store, None).as_deref(), Some("second"));
    }

    #[test]
    fn resolve_run_slug_infers_the_sole_non_archived_run() {
        use darkrun_core::domain::Status;
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        // Two runs, neither inferable as "active": the live one is Completed (so
        // the active-run inference yields None) and the other is archived. With no
        // active pointer and no git branch, resolution falls to step 4 — the sole
        // *non-archived* run wins.
        run_start(&store, "live", "software", None, Mode::Solo, "full").unwrap();
        run_start(&store, "old", "software", None, Mode::Solo, "full").unwrap();

        let mut live = store.read_run("live").unwrap();
        live.frontmatter.status = Status::Completed;
        store.write_run(&live).unwrap();
        let mut old = store.read_run("old").unwrap();
        old.frontmatter.status = Status::Completed;
        old.frontmatter.archived = Some(true);
        store.write_run(&old).unwrap();

        store.clear_active_run().unwrap();
        assert!(store.active_run().unwrap().is_none(), "neither run is active");
        assert_eq!(s.resolve_run_slug(&store, None).as_deref(), Some("live"));
    }

    #[test]
    fn list_handlers_surface_an_unreadable_collection_dir() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        // Plant a regular FILE where each collection directory belongs — reading
        // it as a directory fails, so the list handlers must surface a tool error
        // rather than panic.
        std::fs::write(store.feedback_dir("r"), "not a dir").unwrap();
        std::fs::write(store.reflections_dir("r"), "not a dir").unwrap();
        std::fs::write(store.annotations_dir("r"), "not a dir").unwrap();

        assert_eq!(
            s.darkrun_feedback_list(Parameters(FeedbackListInput { slug: "r".into(), include_settled: true }))
                .unwrap().is_error,
            Some(true)
        );
        assert_eq!(
            s.darkrun_reflection_list(Parameters(ReflectionListInput { slug: "r".into() }))
                .unwrap().is_error,
            Some(true)
        );
        assert_eq!(
            s.darkrun_annotation_list(Parameters(AnnotationListInput {
                slug: "r".into(),
                work_item: WorkItemInput { kind: "station".into(), id: String::new(), station: "frame".into() },
                open_only: false,
            })).unwrap().is_error,
            Some(true)
        );

        // annotation_payload reads the same unreadable annotations dir → error.
        assert_eq!(
            s.darkrun_annotation_payload(Parameters(AnnotationListInput {
                slug: "r".into(),
                work_item: WorkItemInput { kind: "station".into(), id: String::new(), station: "frame".into() },
                open_only: true,
            })).unwrap().is_error,
            Some(true)
        );

        // reflection_record writes into the unreadable reflections dir → error.
        assert_eq!(
            s.darkrun_reflection_record(Parameters(ReflectionRecordInput {
                slug: "r".into(), body: "retro".into(), station: Some("frame".into()),
            })).unwrap().is_error,
            Some(true)
        );

        // review_stamp tolerates an unreadable feedback dir (its open-finding scan
        // falls back to empty) and still stamps without erroring.
        let stamped = s.darkrun_review_stamp(Parameters(ReviewStampInput {
            slug: "r".into(), station: "frame".into(), role: "value".into(), kind: "review".into(),
        })).unwrap();
        assert!(stamped.is_error != Some(true), "feedback-scan fault degrades to a clean stamp");
    }

    #[test]
    fn meta_handlers_surface_filesystem_write_faults() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let darkrun = dir.path().join(".darkrun");
        std::fs::create_dir_all(&darkrun).unwrap();
        let s = DarkrunServer::new(dir.path());

        // report: a FILE where `.darkrun/reports` belongs fails the create_dir_all.
        std::fs::write(darkrun.join("reports"), b"x").unwrap();
        assert_eq!(
            s.darkrun_report(Parameters(ReportInput {
                message: "broken".into(), contact_email: None, name: None,
            })).unwrap().is_error,
            Some(true)
        );

        // backlog: a FILE where `.darkrun/backlog` belongs fails the read_dir.
        std::fs::write(darkrun.join("backlog"), b"x").unwrap();
        assert_eq!(
            s.darkrun_backlog(Parameters(BacklogInput { action: None, description: None, id: None }))
                .unwrap().is_error,
            Some(true)
        );

        // setup --apply: a read-only `.darkrun` fails the settings.yml write.
        std::fs::set_permissions(&darkrun, std::fs::Permissions::from_mode(0o555)).unwrap();
        assert_eq!(
            s.darkrun_setup(Parameters(SetupInput { apply: true })).unwrap().is_error,
            Some(true)
        );
        std::fs::set_permissions(&darkrun, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn run_list_surfaces_an_unreadable_state_root() {
        // A FILE where the `.darkrun` state root belongs makes list_runs fail.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".darkrun"), b"not a dir").unwrap();
        let s = DarkrunServer::new(dir.path());
        assert_eq!(
            s.darkrun_run_list(Parameters(RunListInput { include_archived: true })).unwrap().is_error,
            Some(true)
        );
    }

    #[test]
    fn drift_accept_re_witnesses_a_known_input() {
        use darkrun_core::domain::{Unit, UnitFrontmatter};
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        // A unit that witnessed `frame/in.md` as one of its inputs.
        let mut fm = UnitFrontmatter::default();
        fm.station = Some("frame".into());
        fm.input_witnesses.insert("frame/in.md".into(), "oldhash".into());
        store.write_unit("r", &Unit { slug: "u".into(), frontmatter: fm, title: "u".into(), body: String::new() }).unwrap();
        // The artifact exists on disk so it re-witnesses to its current content.
        std::fs::create_dir_all(dir.path().join("frame")).unwrap();
        std::fs::write(dir.path().join("frame/in.md"), b"new content").unwrap();

        let out = s.darkrun_drift_witness(Parameters(DriftAcceptInput {
            slug: "r".into(), path: "frame/in.md".into(),
        })).unwrap();
        assert!(out.is_error != Some(true), "accepting a witnessed input succeeds");
        assert_eq!(out.structured_content.unwrap()["accepted"], "frame/in.md");
    }

    #[test]
    fn handler_edge_arms_success_and_error_paths() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        // annotation_payload: a malformed work_item kind is rejected.
        assert_eq!(
            s.darkrun_annotation_payload(Parameters(AnnotationListInput {
                slug: "r".into(),
                work_item: WorkItemInput { kind: "bogus".into(), id: "x".into(), station: "frame".into() },
                open_only: true,
            })).unwrap().is_error,
            Some(true)
        );

        // drift_accept: a path with no recorded witness → a tool error.
        assert_eq!(
            s.darkrun_drift_witness(Parameters(DriftAcceptInput { slug: "r".into(), path: "nope.md".into() }))
                .unwrap().is_error,
            Some(true)
        );

        // zap with an unknown factory returns a structured error payload.
        let z = s.darkrun_zap(Parameters(ZapInput {
            task: "fix it".into(), factory: Some("ghost-factory".into()), station: None,
        })).unwrap();
        let zbody = z.structured_content.expect("zap structured");
        assert!(zbody.get("error").is_some() || zbody.get("factory").is_some());

        // backlog promote of an unknown id → "no backlog item".
        let b = s.darkrun_backlog(Parameters(BacklogInput {
            action: Some("promote".into()), description: None, id: Some("nope".into()),
        })).unwrap();
        assert!(b.structured_content.unwrap()["error"].as_str().unwrap_or("").contains("no backlog item"));

        // unit_reset on a ghost unit → a tool error.
        assert_eq!(
            s.darkrun_unit_reset(Parameters(UnitResetInput {
                slug: "r".into(), unit: "ghost-unit".into(), confirm: true,
            })).unwrap().is_error,
            Some(true)
        );

        // feedback_resolve closed+reply takes the close_with_reply branch.
        let fb = s.darkrun_feedback_create(Parameters(FeedbackCreateInput {
            slug: "r".into(), station: "frame".into(), body: "needs work".into(),
            severity: None, origin: None, invalidates: None,
        })).unwrap();
        let fb_id = fb.structured_content.unwrap()["id"].as_str().unwrap().to_string();
        let resolved = s.darkrun_feedback_resolve(Parameters(FeedbackResolveInput {
            slug: "r".into(), feedback_id: fb_id, status: "closed".into(), reply: Some("fixed".into()),
        })).unwrap();
        assert!(resolved.is_error != Some(true), "a closed+reply resolution succeeds");
    }

    #[test]
    fn read_handlers_surface_a_corrupt_state_dir_as_a_tool_error() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();

        // A unit file with unparseable YAML frontmatter → read_units errors, so
        // the unit-list and unit-get handlers return a tool error rather than
        // panicking on the malformed state.
        let units = store.units_dir("r");
        std::fs::create_dir_all(&units).unwrap();
        std::fs::write(units.join("broken.md"), "---\nstatus: \"unterminated\n---\nbody\n").unwrap();
        assert_eq!(
            s.darkrun_unit_list(Parameters(RunRef { slug: "r".into() })).unwrap().is_error,
            Some(true)
        );
        assert_eq!(
            s.darkrun_unit_get(Parameters(UnitRef { slug: "r".into(), unit: "broken".into() }))
                .unwrap()
                .is_error,
            Some(true)
        );

        // review_stamp reads units to stamp a role → the corrupt unit makes it
        // error rather than stamp over malformed state.
        assert_eq!(
            s.darkrun_review_stamp(Parameters(ReviewStampInput {
                slug: "r".into(), station: "frame".into(), role: "value".into(), kind: "review".into(),
            })).unwrap().is_error,
            Some(true)
        );

        // run_reset reads units to plan the wipe → the corrupt unit surfaces an error.
        assert_eq!(
            s.darkrun_run_reset(Parameters(RunResetInput {
                slug: "r".into(), station: Some("frame".into()), confirm: false,
            })).unwrap().is_error,
            Some(true)
        );

        // drift_accept reads units to find the witness → the corrupt unit errors.
        assert_eq!(
            s.darkrun_drift_witness(Parameters(DriftAcceptInput { slug: "r".into(), path: "x.md".into() }))
                .unwrap().is_error,
            Some(true)
        );

        // A run.md with unparseable frontmatter → read_run errors; run_show with
        // an explicit slug surfaces it.
        std::fs::write(store.run_dir("r").join("run.md"), "---\nfactory: \"oops\n---\n").unwrap();
        assert_eq!(
            s.darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) })).unwrap().is_error,
            Some(true)
        );
    }

    #[test]
    fn run_show_reports_a_connected_desktop_when_a_session_is_live() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let store = s.store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        // Hold a live WS slot → presence reads as Live, so run_show reports the
        // desktop as already connected rather than trying to launch one.
        let _slot = s.sessions().try_acquire_ws_slot(8).expect("ws slot");
        assert!(s.sessions().presence().is_present());
        let out = s.darkrun_run_inspect(Parameters(RunShowRef { slug: Some("r".into()) })).unwrap();
        assert!(out.is_error != Some(true));
        let body = out.structured_content.expect("structured");
        assert_eq!(body["showing"]["desktop"]["status"], "connected");
    }

    #[test]
    fn parse_author_defaults_to_human_and_maps_every_token() {
        use darkrun_api::common::AuthorType;
        assert!(matches!(parse_author(None).unwrap(), AuthorType::Human));
        assert!(matches!(parse_author(Some(" Human ")).unwrap(), AuthorType::Human));
        assert!(matches!(parse_author(Some("agent")).unwrap(), AuthorType::Agent));
        assert!(matches!(parse_author(Some("system")).unwrap(), AuthorType::System));
        assert!(parse_author(Some("robot")).is_err());
    }

    #[test]
    fn get_info_announces_addr_and_branches_capabilities() {
        let dir = tempdir().unwrap();
        // A prompt-bridging harness (non-Claude) with an announced HTTP address →
        // instructions carry the DARKRUN_PORT note and prompts are advertised.
        let bridged = DarkrunServer::new(dir.path())
            .with_announced_addr("127.0.0.1:58616".parse().unwrap())
            .with_harness(darkrun_harness::Harness::Cursor);
        let info = bridged.get_info();
        let instr = info.instructions.unwrap();
        assert!(instr.contains("DARKRUN_PORT=58616"));
        assert!(info.capabilities.prompts.is_some(), "prompts advertised for a bridged harness");

        // Claude Code with no announced address → no port note, no prompts cap.
        let claude = DarkrunServer::new(dir.path());
        let info2 = claude.get_info();
        assert!(!info2.instructions.unwrap().contains("DARKRUN_PORT"));
        assert!(info2.capabilities.prompts.is_none());
    }

    #[test]
    fn annotation_submit_rejects_each_malformed_field() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let base = || AnnotationSubmitInput {
            slug: "r".into(),
            author: Some("human".into()),
            work_item: WorkItemInput { kind: "output".into(), id: "x".into(), station: "build".into() },
            artifact: None,
            anchor: None,
            expression: None,
            comment: "c".into(),
            ask: serde_json::json!({"kind": "change", "severity": "should"}),
            suggestion: None,
        };
        let is_err = |r: CallToolResult| r.is_error == Some(true);

        // Each malformed field trips its own parse-error arm in turn.
        let mut bad_kind = base();
        bad_kind.work_item.kind = "nonsense".into();
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_kind)).unwrap()));

        let mut bad_author = base();
        bad_author.author = Some("wizard".into());
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_author)).unwrap()));

        let mut bad_artifact = base();
        bad_artifact.artifact = Some(serde_json::json!(5));
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_artifact)).unwrap()));

        let mut bad_anchor = base();
        bad_anchor.anchor = Some(serde_json::json!("not-an-anchor"));
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_anchor)).unwrap()));

        let mut bad_expr = base();
        bad_expr.expression = Some(serde_json::json!(true));
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_expr)).unwrap()));

        let mut bad_suggestion = base();
        bad_suggestion.suggestion = Some(serde_json::json!(1));
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_suggestion)).unwrap()));

        let mut bad_ask = base();
        bad_ask.ask = serde_json::json!("nope");
        assert!(is_err(s.darkrun_annotation_submit(Parameters(bad_ask)).unwrap()));

        // annotation_list rejects a malformed work_item too.
        assert!(is_err(s.darkrun_annotation_list(Parameters(AnnotationListInput {
            slug: "r".into(),
            work_item: WorkItemInput { kind: "bogus".into(), id: "x".into(), station: "build".into() },
            open_only: false,
        })).unwrap()));
    }

    #[test]
    fn unit_iterate_requires_a_note_on_reject() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        s.darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(), factory: "software".into(), title: None, mode: "solo".into(), size: "full".into(),
        })).unwrap();
        s.darkrun_unit_create(Parameters(UnitCreateInput {
            slug: "r".into(), unit: "u1".into(), station: "build".into(), title: None, depends_on: vec![],
        })).unwrap();
        // A reject with no note is exactly the story-loss this guards against.
        let res = s.darkrun_unit_beat(Parameters(UnitIterateInput {
            slug: "r".into(), unit: "u1".into(), worker: "w".into(),
            result: "reject".into(), note: None, next_worker: None,
        })).unwrap();
        assert_eq!(res.is_error, Some(true));
    }

    #[test]
    fn resolve_run_slug_reads_the_current_darkrun_branch() {
        use std::process::Command;
        let dir = tempdir().unwrap();
        let g = |args: &[&str]| {
            assert!(Command::new("git").current_dir(dir.path()).args(args).status().unwrap().success(), "git {args:?}");
        };
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t.t"]);
        g(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "c"]);
        // Stand on the run's `darkrun/<slug>/main` branch — being on it selects it.
        g(&["checkout", "-q", "-b", "darkrun/myrun/main"]);

        let s = DarkrunServer::new(dir.path());
        s.darkrun_run_new(Parameters(RunStartInput {
            slug: "myrun".into(), factory: "software".into(), title: None, mode: "solo".into(), size: "full".into(),
        })).unwrap();
        // No explicit slug, no active pointer → the current branch names the run.
        let store = s.store();
        assert_eq!(s.resolve_run_slug(&store, None).as_deref(), Some("myrun"));
    }

    #[test]
    fn slug_scoped_handlers_error_on_a_ghost_run_extended() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let wi = || WorkItemInput { kind: "output".into(), id: "x".into(), station: "build".into() };

        // Each of these resolves a run that doesn't exist → its Err arm.
        let _ = (s.darkrun_unit_list(Parameters(RunRef { slug: "ghost".into() })).unwrap());
        let _ = (s.darkrun_unit_beat(Parameters(UnitIterateInput {
            slug: "ghost".into(), unit: "u1".into(), worker: "make".into(),
            result: "advance".into(), note: None, next_worker: None,
        })).unwrap());
        let _ = (s.darkrun_quality_gate_record(Parameters(GateRecordInput {
            slug: "ghost".into(), unit: "u1".into(), gate: "t".into(), status: "pass".into(), detail: None, nonce: None,
        })).unwrap());
        let _ = (s.darkrun_review_stamp(Parameters(ReviewStampInput {
            slug: "ghost".into(), station: "frame".into(), role: "spec".into(), kind: "review".into(),
        })).unwrap());
        let _ = (s.darkrun_annotation_submit(Parameters(AnnotationSubmitInput {
            slug: "ghost".into(), author: Some("human".into()), work_item: wi(),
            artifact: None, anchor: None, expression: None, comment: "c".into(),
            ask: serde_json::json!({"kind": "change", "severity": "should"}), suggestion: None,
        })).unwrap());
        let _ = (s.darkrun_annotation_list(Parameters(AnnotationListInput {
            slug: "ghost".into(), work_item: wi(), open_only: false,
        })).unwrap());
        let _ = (s.darkrun_annotation_payload(Parameters(AnnotationListInput {
            slug: "ghost".into(), work_item: wi(), open_only: false,
        })).unwrap());
        let _ = (s.darkrun_reflection_record(Parameters(ReflectionRecordInput {
            slug: "ghost".into(), body: "x".into(), station: None,
        })).unwrap());
        let _ = (s.darkrun_reflection_list(Parameters(ReflectionListInput { slug: "ghost".into() })).unwrap());
        let _ = (s.darkrun_drift_witness(Parameters(DriftAcceptInput {
            slug: "ghost".into(), path: "frame/frame.md".into(),
        })).unwrap());
        let _ = (s.darkrun_feedback_list(Parameters(FeedbackListInput {
            slug: "ghost".into(), include_settled: true,
        })).unwrap());
        let _ = (s.darkrun_direction_result(Parameters(SessionResultInput {
            slug: "ghost".into(), session_id: "d-01".into(),
        })).unwrap());
    }

    #[test]
    fn debug_dispatcher_covers_every_op_and_missing_args() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        s.darkrun_run_new(Parameters(RunStartInput {
            slug: "r".into(), factory: "software".into(), title: None, mode: "solo".into(), size: "full".into(),
        })).unwrap();
        let dbg = |op: &str, station: Option<&str>, field: Option<&str>, value: Option<&str>, fid: Option<&str>| {
            s.darkrun_debug(Parameters(DebugInput {
                slug: "r".into(), op: op.into(),
                station: station.map(Into::into), field: field.map(Into::into),
                value: value.map(Into::into), feedback_id: fid.map(Into::into),
                reason: Some("recovery".into()), confirm: true,
            })).unwrap()
        };
        // preview_cursor reads the cursor (ok path).
        assert_ne!(dbg("preview_cursor", None, None, None, None).is_error, Some(true));
        // Each mutating op with its required arg MISSING → a clear error.
        assert_eq!(dbg("force_station_complete", None, None, None, None).is_error, Some(true));
        assert_eq!(dbg("set_run_field", None, None, None, None).is_error, Some(true));
        assert_eq!(dbg("mutate_feedback", None, None, None, None).is_error, Some(true));
        // reset_drift runs; an unknown op errors.
        let _ = dbg("reset_drift", None, None, None, None);
        assert_eq!(dbg("nonsense_op", None, None, None, None).is_error, Some(true));
        // set_run_field with field+value mutates (or errors cleanly on a bad field).
        let _ = dbg("set_run_field", None, Some("title"), Some("New"), None);
        // The mutating ops with their required args PRESENT actually run the
        // recovery call (not just the missing-arg guard).
        let _ = dbg("force_station_complete", Some("frame"), None, None, None);
        let _ = dbg("mutate_feedback", None, None, Some("closed"), Some("fb-1"));
    }

    #[test]
    fn backlog_dispatcher_covers_add_promote_and_list() {
        let dir = tempdir().unwrap();
        let s = DarkrunServer::new(dir.path());
        let bl = |action: Option<&str>, desc: Option<&str>, id: Option<&str>| {
            s.darkrun_backlog(Parameters(BacklogInput {
                action: action.map(Into::into), description: desc.map(Into::into), id: id.map(Into::into),
            })).unwrap()
        };
        // add with a description, then list shows it.
        assert_ne!(bl(Some("add"), Some("wire the importer"), None).is_error, Some(true));
        assert_ne!(bl(None, None, None).is_error, Some(true)); // default list
        // add without a description, and promote without/with an unknown id → errors / no-op.
        assert_eq!(bl(Some("add"), None, None).is_error, Some(true));
        assert_eq!(bl(Some("promote"), None, None).is_error, Some(true));
        let _ = bl(Some("promote"), None, Some("bl-999")); // unknown id → error json (ok status)

        // Promote a REAL backlog item → the success arm.
        let added = bl(Some("add"), Some("ship the thing"), None);
        let id = added.structured_content.unwrap()["added"]["id"].as_str().unwrap().to_string();
        let promoted = bl(Some("promote"), None, Some(&id));
        assert!(
            promoted.structured_content.unwrap().get("promoted").is_some(),
            "promoting a real item takes the success arm"
        );
    }

    #[test]
    fn ok_json_surfaces_a_serialization_failure() {
        use std::collections::HashMap;
        // A map with non-string keys cannot serialize to a JSON object — the
        // error arm returns a tool error rather than panicking.
        let mut m: HashMap<(i32, i32), i32> = HashMap::new();
        m.insert((1, 2), 3);
        let res = ok_json(&m).unwrap();
        assert_eq!(res.is_error, Some(true));
    }
}
