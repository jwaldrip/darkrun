//! darkrun-mcp — the MCP server + manager that drives a darkrun Run.
//!
//! This crate is the engine half of darkrun: the manager
//! (`run-tick` -> `derive_position`) and the core MCP tool
//! surface, built around the factory vocabulary
//! (Factory > Station > Unit > Pass).
//!
//! ## Manager
//!
//! The manager is a **pure read** of on-disk `.darkrun/` state that
//! returns ONE structured next-action instruction. It never runs LLM agents —
//! it tells the caller what to do; the caller does it, then re-ticks. See
//! [`position::derive_position`] and [`position::run_tick`].
//!
//! Three-track priority (Track C -> B -> A): **drift -> feedback ->
//! run**. Inside the run track, each Station walks the phase machine
//! `Spec -> Review -> Manufacture -> Audit -> Reflect -> Checkpoint`, where the
//! checkpoint kind (`auto`/`ask`/`external`/`await`) decides whether the station
//! advances automatically or holds for an operator decision. Each phase expands
//! into named sub-step beats in the *rendered prompt*, not in separate ticks.
//!
//! ## MCP surface
//!
//! [`tools::DarkrunServer`] exposes the tool surface over the official Rust
//! MCP SDK (`rmcp`):
//!
//! - **Run:** `darkrun_run_start`, `darkrun_run_next`, `darkrun_run_show`,
//!   `darkrun_run_list`, `darkrun_run_archive`.
//! - **Units:** `darkrun_unit_list`, `darkrun_unit_get`, `darkrun_unit_create`,
//!   `darkrun_unit_update`.
//! - **Feedback:** `darkrun_feedback_create`, `darkrun_feedback_list`,
//!   `darkrun_feedback_resolve`, `darkrun_feedback_reject`,
//!   `darkrun_feedback_move`.
//! - **Checkpoint:** `darkrun_checkpoint_decide`.
//! - **Surface + proof:** `darkrun_run_surface` (classify/read the run's
//!   verification surface) plus `darkrun_proof_attach` / `darkrun_proof_get`
//!   (attach/read the surface-routed objective evidence — the Prove station's
//!   NUMBERS — feeding the view/review).
//! - **Factories:** `darkrun_factory_list`, `darkrun_factory_detail`.
//! - **Visual sessions:** `darkrun_question`, `darkrun_direction`,
//!   `darkrun_picker` (emit a mid-run operator prompt) plus
//!   `darkrun_question_result`, `darkrun_direction_result`,
//!   `darkrun_picker_result` (read the operator's answer/selection back).
//!
//! [`server::serve_stdio`] serves the surface over stdio AND co-hosts the
//! [`darkrun_http`] HTTP/WS review server in-process, sharing one in-memory
//! [`sessions::SessionRegistry`] so interactive sessions reach the desktop app
//! with no on-disk bridge. The typed helpers behind the tools live in [`units`],
//! [`feedback`], [`runs`], [`drift`], [`proof`], and [`sessions`].

pub mod backlog;
pub mod change;
pub mod drift;
pub mod error;
pub mod factory;
pub mod feedback;
pub mod gate;
pub mod meta;
pub mod position;
pub mod proof;
pub mod reflection;
pub mod reset;
pub mod runs;
pub mod scaffold;
pub mod server;
pub mod sessions;
pub mod setup;
pub mod skill_bridge;
pub mod tools;
pub mod units;
pub mod zap;

pub use change::{change_request_intent, ChangeRequestIntent};
pub use error::{McpError, Result};
pub use factory::{list_factories, resolve_factory, FactoryDef, StationDef};
pub use position::{
    checkpoint_decide, derive_position, render_prompt, run_start, run_tick, Position, PromptContext,
    RunAction, TickResult, Track,
};
pub use proof::{
    attach_proof, get_proof, get_surface, route_for, set_surface, SurfaceResult,
};
pub use runs::RunSummary;
pub use server::{serve_stdio, serve_stdio_on, DEFAULT_ADDR};
pub use sessions::{
    create_direction, create_picker, create_question, direction_result, picker_result,
    question_result, ArchetypeSpec, AwaitingSession, PickerOptionSpec, QuestionOptionSpec,
    SessionRegistry,
};
pub use tools::DarkrunServer;
pub use units::UnitUpdate;
