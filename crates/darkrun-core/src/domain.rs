//! Domain types for the darkrun factory model.
//!
//! Vocabulary (factory/assembly-line metaphor):
//! - `Factory`  — a methodology
//! - `Station`  — one risk-eliminating stage
//! - `Unit`     — a decomposed piece of work
//! - `Pass`     — one Make->Challenge->Resolve iteration
//! - `Worker`   — an agent performing a beat of a Pass
//! - `Run`      — a top-level execution
//! - `Explorer` — gathers context
//! - `Reviewer` — verifies output
//! - `Checkpoint` — the gate ending a station
//!
//! Hierarchy: Factory > Station > Unit > Pass.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle status shared by Runs, Stations, and Units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not yet started.
    #[default]
    Pending,
    /// Currently being worked.
    Active,
    /// In flight (alias the manager uses for active execution).
    InProgress,
    /// Finished and locked.
    Completed,
    /// Blocked on an unmet dependency or gate.
    Blocked,
}

/// The fixed taxonomy of phases every Station walks, in order:
/// `Spec -> Review -> Manufacture -> Audit -> Reflect -> Checkpoint`.
///
/// Explore + Decompose happen in `Spec`; the Pass-loop (Make -> Challenge ->
/// Resolve) runs in `Manufacture`; verification AND the quality checks/tests
/// both happen in `Audit` (audit verifies the output against the spec *and*
/// runs the tests — there is no separate tests phase); `Reflect` is an
/// autonomous retrospective that feeds the run-level reflections; the gate
/// runs in `Checkpoint`. Note the `Spec` *phase* (every station has one) is
/// distinct from the `Specify` *station* — they sit at different levels of
/// Factory > Station > phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StationPhase {
    /// Specify the work: run Explorers, then Decompose into Units with criteria.
    Spec,
    /// Review the spec before any output is manufactured.
    Review,
    /// Manufacture the output: the Pass-loop (Make -> Challenge -> Resolve).
    Manufacture,
    /// Audit the manufactured output against the spec AND run the quality
    /// checks / tests (the old `Tests` phase folded in here).
    Audit,
    /// Reflect: an autonomous retrospective that captures learnings feeding the
    /// run-level reflections, before the gate fires.
    Reflect,
    /// The Checkpoint gate fires (auto/ask/external/await).
    Checkpoint,
}

/// The kind of gate a Checkpoint applies at the end of a Station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    /// Advance automatically once reviews pass.
    Auto,
    /// Ask the local operator before advancing.
    Ask,
    /// Hand off to an external review surface (e.g. a PR).
    External,
    /// Block on a `darkrun_await_gate` call until a decision arrives.
    Await,
}

/// The outcome a Checkpoint produced when it last fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOutcome {
    /// The station advanced.
    Advanced,
    /// Held awaiting an operator decision.
    Paused,
    /// Blocked — rework routed back as drift.
    Blocked,
    /// Awaiting an external/await decision.
    Awaiting,
}

/// Git policy for a Run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct RunGit {
    /// How changes are integrated (e.g. "worktree-per-unit").
    #[serde(default)]
    pub change_strategy: String,
    /// Whether the engine auto-merges completed branches.
    #[serde(default)]
    pub auto_merge: bool,
    /// Whether merges are squashed.
    #[serde(default)]
    pub auto_squash: bool,
}

/// Frontmatter for a Run document (`.darkrun/<run>/run.md`).
///
/// Carries the factory name and the active station for the Run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct RunFrontmatter {
    /// Human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The factory (methodology) driving this run.
    pub factory: String,
    /// Run sizing mode (e.g. "continuous", "right-sized").
    #[serde(default)]
    pub mode: String,
    /// The station the legacy write-cache points at (derived state is authoritative).
    #[serde(default)]
    pub active_station: String,
    /// Lifecycle status.
    #[serde(default)]
    pub status: Status,
    /// Whether this run is archived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Git integration policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<RunGit>,
}

/// A parsed Run document: frontmatter + markdown body.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Run {
    /// URL-safe identifier (the `.darkrun/<slug>/` directory name).
    pub slug: String,
    /// Parsed frontmatter.
    pub frontmatter: RunFrontmatter,
    /// Title resolved from frontmatter or the first H1.
    pub title: String,
    /// Raw markdown body (everything after the frontmatter fence).
    pub body: String,
}

/// Frontmatter for a Unit document (`.darkrun/<run>/units/<slug>.md`).
///
/// Carries the unit's passes, its worker assignment, and its station.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UnitFrontmatter {
    /// Optional display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Unit kind (free-form, factory-defined).
    #[serde(default)]
    pub unit_type: String,
    /// Lifecycle status.
    #[serde(default)]
    pub status: Status,
    /// Slugs of units this one depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// The current Pass index (was: bolt).
    #[serde(default)]
    pub pass: u32,
    /// The Worker currently assigned (was: hat).
    #[serde(default)]
    pub worker: String,
    /// Optional model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The station this unit belongs to (injected when read from a station dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// Run-relative paths to artifacts the unit consumed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Run-relative paths to artifacts the unit produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// A parsed Unit document: frontmatter + markdown body.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Unit {
    /// URL-safe identifier (the `units/<slug>.md` file stem).
    pub slug: String,
    /// Parsed frontmatter.
    pub frontmatter: UnitFrontmatter,
    /// Title resolved from frontmatter or the first H1.
    pub title: String,
    /// Raw markdown body.
    pub body: String,
}

impl Unit {
    /// The unit's lifecycle status.
    pub fn status(&self) -> Status {
        self.frontmatter.status
    }

    /// The station this unit belongs to, defaulting to the synthetic root.
    pub fn station(&self) -> &str {
        self.frontmatter.station.as_deref().unwrap_or("_root")
    }
}

/// One Pass over a Unit — a Make -> Challenge -> Resolve iteration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Pass {
    /// Zero-based pass index within the unit.
    pub index: u32,
    /// The unit slug this pass operated on.
    pub unit: String,
    /// The beat currently in flight.
    pub beat: PassBeat,
}

/// The three beats of a single Pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PassBeat {
    /// Produce the artifact.
    Make,
    /// Adversarially attack the artifact.
    Challenge,
    /// Reconcile the attack into the artifact.
    Resolve,
}

/// A Worker — an agent that performs a beat of a Pass (was: hat).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Worker {
    /// Worker identifier (e.g. "builder", "challenger").
    pub name: String,
    /// Optional model the worker runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Whether this worker terminates a pass (triggers merge/advance).
    #[serde(default)]
    pub terminal: bool,
}

/// An Explorer — gathers the context a Station needs (was: discovery-agent).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Explorer {
    /// Explorer identifier (e.g. "context", "value").
    pub name: String,
    /// What this explorer is mandated to gather.
    #[serde(default)]
    pub mandate: String,
}

/// A Reviewer — verifies output against criteria, independent of Workers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Reviewer {
    /// Reviewer identifier (e.g. "value", "feasibility").
    pub name: String,
    /// The dimension this reviewer checks.
    #[serde(default)]
    pub dimension: String,
}

/// The Checkpoint that gates the end of a Station.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Checkpoint {
    /// The gate kind.
    pub kind: CheckpointKind,
    /// RFC3339 timestamp the gate was entered, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered_at: Option<String>,
    /// The outcome the gate last produced, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CheckpointOutcome>,
}

/// Derived per-Station state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Station {
    /// Station name (e.g. "frame", "build").
    pub station: String,
    /// Lifecycle status.
    #[serde(default)]
    pub status: Status,
    /// Current phase within the station.
    pub phase: StationPhase,
    /// The checkpoint gating this station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Severity of a Feedback finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSeverity {
    /// Stops the checkpoint.
    Blocker,
    /// Fix before delivery.
    High,
    /// Should fix.
    Medium,
    /// Nit.
    Low,
}

/// Lifecycle status of a Feedback item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    /// Open and unaddressed.
    Pending,
    /// A fix-worker loop is in flight.
    Fixing,
    /// A fix landed.
    Addressed,
    /// Resolved by a reply, no code delta.
    Answered,
    /// Valid but no actionable code fix.
    NonActionable,
    /// Fix-loop cap exceeded; awaiting human intervention.
    Escalated,
    /// Terminally closed.
    Closed,
    /// Rejected as invalid.
    Rejected,
}

/// A Feedback item routed back from a Checkpoint (`feedback/*.md`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Feedback {
    /// Stable feedback identifier.
    pub id: String,
    /// The run this feedback belongs to.
    pub run: String,
    /// The station the feedback targets.
    pub station: String,
    /// Lifecycle status.
    pub status: FeedbackStatus,
    /// Finding severity (absent until classified).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<FeedbackSeverity>,
    /// Free-text finding body.
    #[serde(default)]
    pub body: String,
    /// RFC3339 creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// The kind of artifact a Drift entry witnessed mutating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DriftKind {
    /// A unit body changed.
    Spec,
    /// A declared output changed.
    Output,
    /// An explorer output changed.
    DiscoveryOutput,
    /// An explorer mandate changed.
    DiscoveryMandate,
}

/// A drift entry — a witnessed artifact whose on-disk content no longer
/// matches its stored hash (flagged by the drift sweep).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Drift {
    /// Run-relative path of the mutated artifact.
    pub path: String,
    /// The station the artifact belongs to.
    pub station: String,
    /// The run the artifact belongs to.
    pub run: String,
    /// What kind of artifact drifted.
    pub kind: DriftKind,
    /// Human-readable age string.
    #[serde(default)]
    pub age: String,
    /// The unit that owns the artifact, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}
