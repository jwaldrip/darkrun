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

use std::path::PathBuf;

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
    /// The pre-execution USER gate: review work is done and the operator is
    /// reviewing the station's spec/brief before any Unit is manufactured. A
    /// discrete hold the cursor surfaces to the desktop review surface — the
    /// pre-execution twin of `Checkpoint`. Resolved by `darkrun_checkpoint_decide`.
    UserGate,
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

/// The six fixed stations of the FSSBPH flow, in cost-of-late-discovery order.
///
/// This is a **hardcoded, mandatory mechanic** — every factory walks these six,
/// in this order, always. It is NOT overridable and has no on-disk definition:
/// the spine is the methodology's invariant, so it lives in code (an invariant,
/// not a fallback). A factory supplies only *orientation* for each position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    /// Kills *wrong-thing* — establish problem/user/value/metric/non-goals.
    Frame,
    /// Kills *ambiguity* — make "done" testable and unambiguous.
    Specify,
    /// Kills *expensive structural reversal* — choose a sound, reversible approach.
    Shape,
    /// Kills *implementation defects* — produce the work.
    Build,
    /// Kills *escaped defects* — verify independently of the producer.
    Prove,
    /// Kills *works-in-dev-dies-in-prod* — operationalize for reality.
    Harden,
}

impl Position {
    /// The fixed flow — the six positions in order. The methodology's spine.
    pub const FLOW: [Position; 6] = [
        Position::Frame,
        Position::Specify,
        Position::Shape,
        Position::Build,
        Position::Prove,
        Position::Harden,
    ];

    /// The on-disk directory name / slug for this position (`"frame"`, …). The
    /// `stations/<dir>/` content for every factory is keyed by this.
    pub fn dir(self) -> &'static str {
        match self {
            Position::Frame => "frame",
            Position::Specify => "specify",
            Position::Shape => "shape",
            Position::Build => "build",
            Position::Prove => "prove",
            Position::Harden => "harden",
        }
    }

    /// Parse a position slug, or `None` if it is not one of the six.
    pub fn parse(slug: &str) -> Option<Position> {
        Position::FLOW.into_iter().find(|p| p.dir() == slug)
    }

    /// This position's index in the flow (0 = Frame … 5 = Harden).
    pub fn index(self) -> usize {
        Position::FLOW.iter().position(|&p| p == self).unwrap_or(0)
    }
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

/// The run's **mode** — the single, global control of how much human review the
/// run involves. Chosen once at run start; every station's Checkpoint is derived
/// from it ([`Mode::gate`]), so there are *no* per-station gate settings.
///
/// All three modes **pre-elaborate** up front — gathering as much context from
/// the operator as possible before the run starts. They differ in what happens
/// afterwards:
/// - [`Team`](Mode::Team): each station elaborates with the operator and opens a
///   per-station PR/MR the team reviews and merges — the team is *in the loop*.
///   Gate: [`External`](CheckpointKind::External).
/// - [`Solo`](Mode::Solo): each station elaborates with the operator and asks for
///   a local review (desktop approval) — the operator is *in the loop*. Gate:
///   [`Ask`](CheckpointKind::Ask).
/// - [`Dark`](Mode::Dark): after the up-front pre-elaboration the run executes
///   without stopping for review — the operator is *on the loop* (monitors, adds
///   feedback, but the system never holds). Gate: [`Auto`](CheckpointKind::Auto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Team in the loop — external review, per-station PR, await merge.
    Team,
    /// Operator in the loop — local review, desktop approval.
    #[default]
    Solo,
    /// Operator on the loop — pre-elaborate once, then run without review stops.
    Dark,
}

impl Mode {
    /// Every mode, in escalating-autonomy order (most human → least).
    pub const ALL: [Mode; 3] = [Mode::Team, Mode::Solo, Mode::Dark];

    /// The canonical lowercase token (`team` / `solo` / `dark`).
    pub fn label(self) -> &'static str {
        match self {
            Mode::Team => "team",
            Mode::Solo => "solo",
            Mode::Dark => "dark",
        }
    }

    /// One-line description of who reviews under this mode.
    pub fn description(self) -> &'static str {
        match self {
            Mode::Team => {
                "team in the loop — each station opens a PR the team reviews and merges"
            }
            Mode::Solo => {
                "you in the loop — each station asks for local review before it advances"
            }
            Mode::Dark => {
                "you on the loop — pre-elaborate up front, then run without stopping for review"
            }
        }
    }

    /// The Checkpoint gate **every** station resolves under this mode — the global
    /// gate axis. There are no per-station overrides.
    pub fn gate(self) -> CheckpointKind {
        match self {
            Mode::Team => CheckpointKind::External,
            Mode::Solo => CheckpointKind::Ask,
            Mode::Dark => CheckpointKind::Auto,
        }
    }

    /// Whether each station opens a per-station PR/MR the human merges (the
    /// discrete branch-landing path). Only `team` does; `solo`/`dark` land the
    /// station branch in-process. The branch hierarchy itself is universal
    /// regardless of mode — only the PR-open-and-await-merge step is team-only.
    pub fn opens_station_pr(self) -> bool {
        matches!(self, Mode::Team)
    }

    /// Whether each station holds its Spec for operator elaboration before it
    /// advances to Review. `team`/`solo` involve the operator at every station;
    /// `dark` pre-elaborates once up front and then runs without per-station holds.
    pub fn holds_each_station(self) -> bool {
        !matches!(self, Mode::Dark)
    }

    /// Parse a mode from a label, tolerating the legacy mode strings the engine
    /// used before the team/solo/dark model. The old sizing strings
    /// (`quick`/`bugfix`/`refactor`) named a *plan*, not a review posture, so they
    /// resolve to the default `solo` — the size axis is now separate.
    pub fn from_label(s: &str) -> Mode {
        match s.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "team" | "discrete" | "discrete_hybrid" => Mode::Team,
            "dark" | "auto" | "autopilot" => Mode::Dark,
            // solo / continuous / collaborative / quick / bugfix / refactor /
            // full / standard / unknown all resolve to the in-the-loop default.
            _ => Mode::Solo,
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Mode::from_label(s))
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// Tolerant deserialize: accept the canonical tokens AND the legacy mode strings
// so existing `run.md` frontmatter keeps loading across the rename.
impl<'de> Deserialize<'de> for Mode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Mode::from_label(&s))
    }
}

/// The run-level final gate that holds a fully-manufactured run *before* it
/// seals — the parity for the predecessor's `pending_seal` / `intent_approved` tail.
///
/// When every station is locked but a `seal:` is declared, the manager emits
/// `PendingSeal` instead of `Sealed`: the run waits on an external decision
/// (e.g. a PR/MR merge) or an explicit await-gate before it is considered
/// delivered. Absent (`None`) → the run seals as soon as the last station locks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SealKind {
    /// Hold for an external surface (a PR/MR merge) before sealing.
    External,
    /// Hold on an await-gate decision before sealing.
    Await,
}

impl SealKind {
    /// The serde token for this seal kind.
    pub fn as_str(self) -> &'static str {
        match self {
            SealKind::External => "external",
            SealKind::Await => "await",
        }
    }
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

/// The kind of SURFACE a Run delivers — the linchpin that routes which
/// objective verification applies at the Prove/Audit stations.
///
/// Set at the Shape station, the surface classifies what the run produces so
/// downstream stations route measurement by it:
/// - [`Surface::WebUi`] / [`Surface::Desktop`] / [`Surface::Mobile`] — a real
///   headless browser: screenshot + web vitals + a11y/contrast/touch-target/
///   reduced-motion audits.
/// - [`Surface::Library`] / [`Surface::Api`] / [`Surface::Data`] — criterion
///   microbenchmarks + a small load harness (no browser); API-surface review.
/// - [`Surface::Tui`] / [`Surface::Cli`] — terminal/output snapshot + interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// A reusable code library (criterion benches + load harness).
    Library,
    /// A network API surface (criterion benches + load harness).
    Api,
    /// A web UI (headless browser: screenshot + vitals + a11y audits).
    WebUi,
    /// A terminal UI (terminal snapshot + interaction).
    Tui,
    /// A command-line tool (output snapshot + interaction).
    Cli,
    /// A desktop application (headless browser: screenshot + vitals + a11y).
    Desktop,
    /// A mobile application (headless browser: screenshot + vitals + a11y).
    Mobile,
    /// A data pipeline / dataset (criterion benches + load harness).
    Data,
}

impl Surface {
    /// The serde token for this surface (the snake_case wire string).
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Library => "library",
            Surface::Api => "api",
            Surface::WebUi => "web_ui",
            Surface::Tui => "tui",
            Surface::Cli => "cli",
            Surface::Desktop => "desktop",
            Surface::Mobile => "mobile",
            Surface::Data => "data",
        }
    }

    /// Parse a surface token, tolerating the common `web-ui`/`webui` spellings
    /// and trimming/case-folding. Returns `None` for an unknown token.
    pub fn parse(raw: &str) -> Option<Surface> {
        match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "library" | "lib" => Some(Surface::Library),
            "api" => Some(Surface::Api),
            "web_ui" | "webui" | "web" => Some(Surface::WebUi),
            "tui" => Some(Surface::Tui),
            "cli" => Some(Surface::Cli),
            "desktop" => Some(Surface::Desktop),
            "mobile" => Some(Surface::Mobile),
            "data" => Some(Surface::Data),
            _ => None,
        }
    }

    /// Whether this surface is verified through a real headless browser
    /// (screenshot + web vitals + a11y audits) rather than benches or a
    /// terminal snapshot.
    pub fn is_visual(self) -> bool {
        matches!(self, Surface::WebUi | Surface::Desktop | Surface::Mobile)
    }

    /// Whether this surface is verified through criterion microbenchmarks + a
    /// small load harness (no browser).
    pub fn is_bench(self) -> bool {
        matches!(self, Surface::Library | Surface::Api | Surface::Data)
    }

    /// Whether this surface is verified through a terminal/output snapshot +
    /// interaction.
    pub fn is_terminal(self) -> bool {
        matches!(self, Surface::Tui | Surface::Cli)
    }
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
    /// The run's global review mode (team/solo/dark) — the single gate axis.
    #[serde(default)]
    pub mode: Mode,
    /// The station the legacy write-cache points at (derived state is authoritative).
    #[serde(default)]
    pub active_station: String,
    /// Lifecycle status.
    #[serde(default)]
    pub status: Status,
    /// The SURFACE this run delivers — set at the Shape station, it routes
    /// which objective verification applies at Prove/Audit. `None` until
    /// classified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<Surface>,
    /// Whether this run is archived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// The git identity (email) that CREATED this run — stamped once at run
    /// start so authorship is knowable even before any commit lands on the
    /// run's branches (the Mine filter's fallback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    /// Git integration policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<RunGit>,
    /// The run-level final gate. When set, a fully-manufactured run holds at
    /// `PendingSeal` (awaiting an external merge or an await decision) instead
    /// of sealing the moment the last station locks. `None` → seal immediately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<SealKind>,
    /// COMPOSITE runs: the per-part topology (factory + station subset each
    /// part walks). A composite run is **not single-walkable** — the manager's
    /// cursor steps around it and the operator/agent coordinates the parts.
    /// `None` (the default) = a normal single-factory run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composite: Option<Vec<CompositePart>>,
    /// Composite sync points: `then` parts may not start until every `wait`
    /// entry (a `factory:station` handle) has completed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sync: Vec<SyncPoint>,
    /// Composite progress notes, keyed by `factory:station` handle — the
    /// coordination ledger the parts stamp as they advance.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub composite_state: std::collections::BTreeMap<String, String>,
    /// Durable pointers between this run and external systems — a ticket key, a
    /// PR/MR url, a design link, and any other named handles. Lets the run and
    /// the systems around it (issue tracker, hosting, design tool) reference one
    /// another (G2). Empty by default.
    #[serde(default, skip_serializing_if = "ExternalRefs::is_empty")]
    pub external_refs: ExternalRefs,
}

/// One part of a composite run: a factory and the stations of it this part walks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompositePart {
    /// The factory this part runs.
    pub factory: String,
    /// The station subset this part walks (empty = the factory's full line).
    #[serde(default)]
    pub stations: Vec<String>,
}

/// A composite sync point: `then` parts hold until every `wait` handle
/// (`factory:station`) completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyncPoint {
    /// The `factory:station` handles that must complete first.
    pub wait: Vec<String>,
    /// The `factory:station` handles released once `wait` is satisfied.
    pub then: Vec<String>,
}

/// Cross-system handles attached to a run — durable pointers the engine keeps
/// but does not interpret, so a darkrun run is discoverable from (and links back
/// to) the ticket, change request, and design that surround it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExternalRefs {
    /// The issue/ticket key this run delivers (e.g. `JIRA-123`, `#456`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    /// The change-request url (PR/MR) this run's work lands through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// A design/spec link (Figma, doc) the run was shaped against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<String>,
    /// Any other named handles, keyed by a caller-chosen label.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub other: std::collections::BTreeMap<String, String>,
}

impl ExternalRefs {
    /// Whether no handle of any kind is set (the serialize-skip predicate).
    pub fn is_empty(&self) -> bool {
        self.ticket.is_none()
            && self.pr_url.is_none()
            && self.design.is_none()
            && self.other.is_empty()
    }

    /// Set a handle by its well-known name (`ticket`/`pr_url`/`design`), falling
    /// back to the `other` map for any other key. An empty `value` clears it.
    pub fn set(&mut self, key: &str, value: &str) {
        let value = value.trim();
        let slot = match key {
            "ticket" => &mut self.ticket,
            "pr_url" | "pr" => &mut self.pr_url,
            "design" => &mut self.design,
            other => {
                if value.is_empty() {
                    self.other.remove(other);
                } else {
                    self.other.insert(other.to_string(), value.to_string());
                }
                return;
            }
        };
        *slot = if value.is_empty() { None } else { Some(value.to_string()) };
    }
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

impl Run {
    /// The SURFACE this run delivers, if classified.
    pub fn surface(&self) -> Option<Surface> {
        self.frontmatter.surface
    }

    /// Set the run's SURFACE (what the Shape station calls once it classifies
    /// the deliverable).
    pub fn set_surface(&mut self, surface: Surface) {
        self.frontmatter.surface = Some(surface);
    }
}

/// The result of one Pass iteration over a Unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IterationResult {
    /// The worker advanced (moves to the next worker / completes the loop).
    Advance,
    /// The worker rejected — bounces to the nearest preceding build worker.
    Reject,
}

/// One recorded Pass iteration on a Unit — an **append-only** beat in the
/// Make→Challenge→Resolve loop. The iteration array is the single source of
/// truth: the phase derivation reads it to decide whether `Manufacture` is done
/// (the last worker `advance`d), and the Pass *number* is derived from the array
/// length — never stored — so it can never disagree with the record.
///
/// Each iteration carries a `note`: the worker's **handoff** on advance ("what I
/// did, what the next worker should know") or its **reason** on reject ("why I
/// bounced this back"). That note is threaded into the next worker's dispatch so
/// the loop carries its own story — for the next worker, the operator, and the
/// reflection pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct UnitIteration {
    /// The worker that ran this iteration.
    #[serde(default)]
    pub worker: String,
    /// RFC3339 start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion (absent = still in flight; stamped on terminal result).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// The iteration's result (absent = still in flight).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<IterationResult>,
    /// The worker's handoff message — its rationale. On `advance`, what it did
    /// and what the next worker should know; on `reject`, why it bounced. Read
    /// into the next worker's dispatch and surfaced to the operator/reflection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl UnitIteration {
    /// Whether this iteration has reached a terminal result.
    pub fn is_complete(&self) -> bool {
        self.result.is_some()
    }
}

/// A review/approval stamp on a Unit — the witness that a role signed off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Stamp {
    /// RFC3339 timestamp the role signed.
    pub at: String,
}

/// A declared quality gate on a Unit — an objective check the unit's work must
/// pass before it can leave Manufacture. The *command* is project-specific
/// (`cargo test`, `npm run lint`) and supplied at decomposition by the agent who
/// knows the project; the engine doesn't run it (the agent does, it has a shell)
/// — the engine **records and enforces** the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QualityGate {
    /// Short name (e.g. `tests`, `lint`, `types`).
    pub name: String,
    /// The command that runs the check.
    #[serde(default)]
    pub command: String,
}

/// The outcome of running a quality gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// The check ran and passed.
    Pass,
    /// The check ran and failed — blocks Audit until fixed.
    Fail,
    /// The check could not run (a dependency was unavailable — DB down, tool
    /// absent). Not a failure of the work; doesn't stamp a pass, but after
    /// repeated env-blocks the gate may be deferred to CI rather than wedge.
    EnvBlocked,
    /// The check is delegated to CI (authoritative on the change request) after
    /// it could not converge locally. Satisfies the gate so the run can advance.
    DeferredToCi,
}

/// A recorded quality-gate result on a Unit — what happened when the gate ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateResult {
    /// The gate name this result is for.
    pub name: String,
    /// The outcome.
    pub status: GateStatus,
    /// RFC3339 timestamp the result was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// How many times this gate has been recorded (drives defer-to-CI).
    #[serde(default)]
    pub attempts: u32,
    /// Optional detail (failure output tail, the blocked dependency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Frontmatter for a Unit document (`.darkrun/<run>/stations/<station>/units/<slug>.md`).
///
/// Carries the unit's passes, its worker assignment, its station, and — the
/// signals the **shared phase derivation** ([`crate::derive`]) reads — its
/// `iterations`, per-role `reviews`/`approvals` stamps, and drift `input_witnesses`.
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
    /// The Worker currently assigned (was: hat). The *active* worker is derived
    /// from the last iteration; this is the assignment the next dispatch targets.
    #[serde(default)]
    pub worker: String,
    /// Optional model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The station this unit belongs to (injected when read from a station dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    /// When `true`, the operator has rolled this unit back for spec revision:
    /// the manager re-opens its spec (parity for the predecessor's `revise_unit_specs`)
    /// and holds the station until the flag clears.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub revise: bool,
    /// When `true`, a reset has been requested for this unit (e.g. from the
    /// desktop review UI) but not yet applied. The next tick performs the reset —
    /// clearing the unit's execution state back to `Pending` so its body unlocks —
    /// and clears this flag. The engine-consumed, declarative twin of the
    /// `darkrun_unit_reset` tool; lets a non-MCP surface request a reset.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset_requested: bool,
    /// Run-relative paths to artifacts the unit consumed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Run-relative paths to artifacts the unit produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// The unit's isolation branch (`darkrun/<slug>/units/<station>/<unit>`),
    /// stamped when the unit's Pass-loop enters its worktree. The Manufacture
    /// dispatch points the worker at this worktree so each unit's diff is
    /// isolated; it lands back onto the station branch when the unit locks.
    /// `None` outside a git-backed run. Engine-managed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// The Pass iteration history — `Manufacture` is done when the LAST iteration
    /// `advance`d on the station's last worker. Engine-managed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iterations: Vec<UnitIteration>,
    /// PRE-execute review stamps, keyed by reviewer role (`None` = unsigned). The
    /// `Review` phase holds until every required role is stamped. Engine-managed.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub reviews: std::collections::BTreeMap<String, Option<Stamp>>,
    /// POST-execute approval stamps, keyed by approval role (`None` = unsigned).
    /// The `Audit`/`Checkpoint` gate holds until every required role is stamped
    /// (incl. `user`, `quality_gates`). Engine-managed.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub approvals: std::collections::BTreeMap<String, Option<Stamp>>,
    /// Per-slot drift witnesses: `path -> sha256` of the inputs each signed slot
    /// was signed over; a changed witness re-opens that slot. Engine-managed.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub input_witnesses: std::collections::BTreeMap<String, String>,
    /// Objective quality gates this unit must pass before leaving Manufacture —
    /// declared by the agent at decomposition (the commands are project-specific).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality_gates: Vec<QualityGate>,
    /// Recorded gate results, keyed by gate name. A declared gate is *satisfied*
    /// when its result is `Pass` or `DeferredToCi`. Engine-recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_results: Vec<GateResult>,
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

    /// The Pass number — **derived** from the iteration history, never stored.
    /// One completed iteration is one beat; the count is the engine's runaway
    /// signal and the operator-visible "how many passes has this unit taken".
    pub fn pass(&self) -> u32 {
        self.frontmatter.iterations.len() as u32
    }

    /// The worker the next beat will dispatch — the current assignment, which
    /// `record_iteration` rolls forward on advance and back on reject.
    pub fn active_worker(&self) -> &str {
        &self.frontmatter.worker
    }

    /// The worker that ran the most recent beat (distinct from the *next*
    /// assignment), if any beat has run.
    pub fn last_worker(&self) -> Option<&str> {
        self.frontmatter.iterations.last().map(|it| it.worker.as_str())
    }

    /// The most recent iteration's handoff note, if any — the story the next
    /// worker (or the operator) should read before acting.
    pub fn last_note(&self) -> Option<&str> {
        self.frontmatter
            .iterations
            .iter()
            .rev()
            .find_map(|it| it.note.as_deref())
    }

    /// Whether every declared quality gate is **satisfied** — each has a recorded
    /// result of `Pass` or `DeferredToCi`. A unit with no declared gates is
    /// trivially satisfied. The Audit gate holds until this is true.
    pub fn gates_satisfied(&self) -> bool {
        self.frontmatter.quality_gates.iter().all(|g| {
            self.frontmatter.gate_results.iter().any(|r| {
                r.name == g.name
                    && matches!(r.status, GateStatus::Pass | GateStatus::DeferredToCi)
            })
        })
    }

    /// The names of declared gates that are not yet satisfied (failing, blocked,
    /// or never recorded) — what the agent still owes before Audit.
    pub fn unsatisfied_gates(&self) -> Vec<&str> {
        self.frontmatter
            .quality_gates
            .iter()
            .filter(|g| {
                !self.frontmatter.gate_results.iter().any(|r| {
                    r.name == g.name
                        && matches!(r.status, GateStatus::Pass | GateStatus::DeferredToCi)
                })
            })
            .map(|g| g.name.as_str())
            .collect()
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
    /// Whether the station's Spec has been **elaborated with the operator** — set
    /// by `darkrun_elaborate_seal` once the agent has involved the operator in
    /// shaping the spec. In `team`/`solo` the Spec phase holds at every station
    /// until this is true (collaboration backpressure); `dark` pre-elaborates once
    /// up front and skips the per-station holds.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub elaborated: bool,
    /// The checkpoint gating this station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<Checkpoint>,
    /// The station's working branch (`darkrun/<slug>/<station>`), set when the
    /// station is entered and a worktree is forked off run-main. `None` on
    /// legacy state and outside a git repo. Retained after landing as a record
    /// of where the station's work happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The discrete-mode draft PR/MR opened for this station's external
    /// Checkpoint (the hosting provider's ref — a number or URL). Set when the
    /// manager opens the station's draft PR via the hosting client; the gate
    /// resolves when this PR is detected merged. `None` for non-discrete runs and
    /// when no hosting client could open one (best-effort await fallback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_ref: Option<String>,
    /// The lifecycle status of the station's draft PR/MR — `draft` on open,
    /// `ready` once it's marked ready for review, `merged`/`closed` once
    /// resolved. Lets the engine (and the operator) see *where* the change
    /// request stands, not just that one exists. `None` until a PR is opened or
    /// on legacy state (G4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_status: Option<PrStatus>,
    /// RFC3339 timestamp the PR transitioned out of draft (became ready for
    /// review). `None` until that happens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_ready_at: Option<String>,
    /// RFC3339 timestamp the PR merged. `None` until merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_merged_at: Option<String>,
    /// The one-time verifier nonce minted when the engine dispatches this
    /// station's verification (Manufacture entry). `darkrun_quality_gate_record`
    /// refuses a result without the matching nonce, so an agent can't certify a
    /// quality gate it was never dispatched to run (B5). Cleared when the station
    /// completes and reminted on a fresh re-entry (rework).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_nonce: Option<String>,
    /// RFC3339 start timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// The lifecycle stage of a station's draft change request (G4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrStatus {
    /// Opened as a draft — not yet marked ready for review.
    Draft,
    /// Marked ready for review (no longer a draft), not yet merged.
    Ready,
    /// Merged — the gate resolves and the station advances.
    Merged,
    /// Closed without merging.
    Closed,
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

/// Where a Feedback item came from — its source, which routes how it is handled
/// and lets the operator and reflection tell an operator's revision from a drift
/// alarm from an adversarial reviewer's finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackOrigin {
    /// A station Reviewer's adversarial finding (the Review/Audit phase).
    AdversarialReview,
    /// A whole-Run reviewer's cross-station finding (run-level audit).
    RunReview,
    /// A reflection dimension's learning surfaced as actionable feedback.
    Reflection,
    /// An Explorer's discovery that needs an operator decision.
    Discovery,
    /// Witnessed artifact drift — an out-of-band change to a locked premise.
    Drift,
    /// The operator, via a checkpoint decision / request-changes.
    Operator,
    /// The operator, via an inline annotation on an artifact.
    Annotation,
    /// An external surface (a PR/MR review comment).
    External,
    /// Origin not recorded (legacy / unclassified).
    #[default]
    Unspecified,
}

/// A worker's reply when it closes a Feedback item — what it actually did, so
/// the requester (operator or reviewer) reads the resolution, not just the
/// status flip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClosureReply {
    /// What was done to resolve the finding.
    pub text: String,
    /// RFC3339 timestamp the reply was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
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
    /// Where this finding came from — routes handling and tells the story.
    #[serde(default, skip_serializing_if = "is_unspecified_origin")]
    pub origin: FeedbackOrigin,
    /// Free-text finding body.
    #[serde(default)]
    pub body: String,
    /// RFC3339 creation timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// The review/approval role slugs this feedback invalidates when it closes —
    /// the stamps that must be re-signed because this finding undercut them.
    /// Closing the feedback clears these on the target unit so the gate re-fires.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalidates: Vec<String>,
    /// The closer's reply — what was done — recorded when the item terminally
    /// closes. Surfaced to the requester so a close carries its resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_reply: Option<ClosureReply>,
}

/// serde skip helper: omit `origin` when it carries no information.
fn is_unspecified_origin(o: &FeedbackOrigin) -> bool {
    matches!(o, FeedbackOrigin::Unspecified)
}

/// A registered project: the persisted record the desktop enumerates to list
/// projects that exist on disk regardless of whether a live engine is serving
/// them.
///
/// Written to `~/.darkrun/<slug>/project.json`, alongside the transient
/// `engine-<pid>.json` descriptors in the SAME slug directory (see
/// `darkrun_mcp::registry`). Where an `EngineDescriptor` is the LIVE record of a
/// running engine, a `ProjectRecord` is the DURABLE record of a registered
/// working tree — it persists when no engine is running, so the home can show
/// registered-but-idle projects.
///
/// `path` is stored absolute at write time and is NOT portable across machines
/// (a project copied to another host carries a stale path); the desktop treats
/// it as a local-filesystem pointer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRecord {
    /// The registry slug for this project — matches the `<slug>` directory name
    /// the record lives under (derived from `path` via the registry's slug
    /// logic).
    pub slug: String,
    /// Absolute repo root of the registered working tree.
    pub path: PathBuf,
    /// Optional human display name; falls back to the slug when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// RFC3339 timestamp the project was registered at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<String>,
}


#[cfg(test)]
mod domain_accessor_tests {
    use super::*;

    #[test]
    fn seal_kind_as_str() {
        assert_eq!(SealKind::External.as_str(), "external");
        assert_eq!(SealKind::Await.as_str(), "await");
    }

    #[test]
    fn unit_iteration_is_complete_tracks_result() {
        let mut it = UnitIteration::default();
        assert!(!it.is_complete());
        it.result = Some(IterationResult::Advance);
        assert!(it.is_complete());
    }

    #[test]
    fn unit_last_worker_reads_the_most_recent_beat() {
        let mut u = Unit {
            slug: "u".into(),
            frontmatter: UnitFrontmatter::default(),
            title: "u".into(),
            body: String::new(),
        };
        assert_eq!(u.last_worker(), None);
        u.frontmatter.iterations.push(UnitIteration { worker: "make".into(), ..Default::default() });
        u.frontmatter.iterations.push(UnitIteration { worker: "challenge".into(), ..Default::default() });
        assert_eq!(u.last_worker(), Some("challenge"));
    }

    #[test]
    fn external_refs_set_well_known_and_other_and_clear() {
        let mut e = ExternalRefs::default();
        e.set("ticket", "JIRA-1");
        e.set("pr", "https://example/pr/1");
        e.set("design", "fig://x");
        e.set("slack", "https://slack/x"); // custom -> other map
        assert_eq!(e.ticket.as_deref(), Some("JIRA-1"));
        assert_eq!(e.pr_url.as_deref(), Some("https://example/pr/1"));
        assert_eq!(e.design.as_deref(), Some("fig://x"));
        assert_eq!(e.other.get("slack").map(String::as_str), Some("https://slack/x"));
        // Empty value clears both a well-known slot and an `other` entry.
        e.set("ticket", "");
        e.set("slack", "  ");
        assert!(e.ticket.is_none());
        assert!(!e.other.contains_key("slack"));
    }
}
