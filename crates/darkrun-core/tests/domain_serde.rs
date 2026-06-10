//! Comprehensive serde + schemars coverage for the entire darkrun-core domain
//! model.
//!
//! Every domain enum and struct is exercised here for: snake_case wire form,
//! default values, `skip_serializing_if` omission, unknown-field tolerance,
//! JSON <-> YAML roundtrip determinism, schemars JSON-Schema generation, and
//! boundary conditions on numeric / string / collection fields. These assert
//! the exact wire contract the rest of the workspace (MCP, API, desktop, web)
//! depends on, so any accidental rename, default change, or skip-rule
//! regression fails loudly here.

use darkrun_core::domain::*;
use schemars::schema_for;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serializes an enum value and asserts it is a bare JSON string, returning it.
fn json_token<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("serialize")
        .as_str()
        .expect("enum serializes to a JSON string")
        .to_string()
}

/// JSON roundtrip: serialize then deserialize, returning the recovered value.
fn json_round<T: Serialize + DeserializeOwned>(value: &T) -> T {
    let s = serde_json::to_string(value).expect("ser json");
    serde_json::from_str(&s).expect("de json")
}

/// YAML roundtrip: serialize then deserialize, returning the recovered value.
fn yaml_round<T: Serialize + DeserializeOwned>(value: &T) -> T {
    let s = serde_yaml::to_string(value).expect("ser yaml");
    serde_yaml::from_str(&s).expect("de yaml")
}

/// Serialize to a JSON object map, panicking if the value is not an object.
fn obj<T: Serialize>(value: &T) -> serde_json::Map<String, Value> {
    serde_json::to_value(value)
        .expect("ser")
        .as_object()
        .expect("object")
        .clone()
}

/// Parse a bare token through JSON enum deserialization.
fn from_token<T: DeserializeOwned>(token: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(&format!("\"{token}\""))
}

/// The generated JSON schema as a `serde_json::Value`.
macro_rules! schema_value {
    ($t:ty) => {
        serde_json::to_value(&schema_for!($t)).expect("schema to value")
    };
}

// ===========================================================================
// Status
// ===========================================================================

#[test]
fn status_pending_token() {
    assert_eq!(json_token(&Status::Pending), "pending");
}

#[test]
fn status_active_token() {
    assert_eq!(json_token(&Status::Active), "active");
}

#[test]
fn status_in_progress_token_is_snake_case() {
    assert_eq!(json_token(&Status::InProgress), "in_progress");
}

#[test]
fn status_completed_token() {
    assert_eq!(json_token(&Status::Completed), "completed");
}

#[test]
fn status_blocked_token() {
    assert_eq!(json_token(&Status::Blocked), "blocked");
}

#[test]
fn status_default_is_pending() {
    assert_eq!(Status::default(), Status::Pending);
}

#[test]
fn status_pending_roundtrips() {
    assert_eq!(json_round(&Status::Pending), Status::Pending);
}

#[test]
fn status_active_roundtrips() {
    assert_eq!(json_round(&Status::Active), Status::Active);
}

#[test]
fn status_in_progress_roundtrips() {
    assert_eq!(json_round(&Status::InProgress), Status::InProgress);
}

#[test]
fn status_completed_roundtrips() {
    assert_eq!(json_round(&Status::Completed), Status::Completed);
}

#[test]
fn status_blocked_roundtrips() {
    assert_eq!(json_round(&Status::Blocked), Status::Blocked);
}

#[test]
fn status_yaml_roundtrips_every_variant() {
    for v in [
        Status::Pending,
        Status::Active,
        Status::InProgress,
        Status::Completed,
        Status::Blocked,
    ] {
        assert_eq!(yaml_round(&v), v);
    }
}

#[test]
fn status_deserializes_from_token() {
    assert_eq!(from_token::<Status>("pending").unwrap(), Status::Pending);
    assert_eq!(from_token::<Status>("active").unwrap(), Status::Active);
    assert_eq!(
        from_token::<Status>("in_progress").unwrap(),
        Status::InProgress
    );
    assert_eq!(
        from_token::<Status>("completed").unwrap(),
        Status::Completed
    );
    assert_eq!(from_token::<Status>("blocked").unwrap(), Status::Blocked);
}

#[test]
fn status_rejects_camel_case_in_progress() {
    // The wire form is snake_case; the Rust variant name must not deserialize.
    assert!(from_token::<Status>("inProgress").is_err());
    assert!(from_token::<Status>("InProgress").is_err());
}

#[test]
fn status_rejects_unknown_token() {
    assert!(from_token::<Status>("done").is_err());
    assert!(from_token::<Status>("").is_err());
    assert!(from_token::<Status>("PENDING").is_err());
}

#[test]
fn status_rejects_numeric_form() {
    // Unit enums with rename_all serialize as strings, never integers.
    assert!(serde_json::from_str::<Status>("0").is_err());
}

#[test]
fn status_equality_is_variant_sensitive() {
    assert_ne!(Status::Pending, Status::Active);
    assert_ne!(Status::Active, Status::InProgress);
    assert_ne!(Status::Completed, Status::Blocked);
}

#[test]
fn status_copy_semantics_hold() {
    let a = Status::Active;
    let b = a; // Copy
    assert_eq!(a, b);
}

#[test]
fn status_schema_lists_all_five_tokens() {
    let s = schema_value!(Status);
    let variants = s["oneOf"].as_array().expect("oneOf array");
    let tokens: Vec<&str> = variants
        .iter()
        .map(|v| v["enum"][0].as_str().expect("enum token"))
        .collect();
    assert_eq!(
        tokens,
        vec!["pending", "active", "in_progress", "completed", "blocked"]
    );
}

#[test]
fn status_schema_has_title_and_description() {
    let s = schema_value!(Status);
    assert_eq!(s["title"], "Status");
    assert!(s["description"].is_string());
}

#[test]
fn status_schema_is_draft07() {
    let s = schema_value!(Status);
    assert_eq!(s["$schema"], "http://json-schema.org/draft-07/schema#");
}

#[test]
fn status_serializes_inside_a_map() {
    let m = json!({ "status": Status::Completed });
    assert_eq!(m["status"], "completed");
}

// ===========================================================================
// StationPhase
// ===========================================================================

const ALL_PHASES: [(StationPhase, &str); 6] = [
    (StationPhase::Spec, "spec"),
    (StationPhase::Review, "review"),
    (StationPhase::Manufacture, "manufacture"),
    (StationPhase::Audit, "audit"),
    (StationPhase::Reflect, "reflect"),
    (StationPhase::Checkpoint, "checkpoint"),
];

#[test]
fn station_phase_has_exactly_six_variants() {
    assert_eq!(ALL_PHASES.len(), 6);
}

#[test]
fn station_phase_spec_token() {
    assert_eq!(json_token(&StationPhase::Spec), "spec");
}

#[test]
fn station_phase_review_token() {
    assert_eq!(json_token(&StationPhase::Review), "review");
}

#[test]
fn station_phase_manufacture_token() {
    assert_eq!(json_token(&StationPhase::Manufacture), "manufacture");
}

#[test]
fn station_phase_audit_token() {
    assert_eq!(json_token(&StationPhase::Audit), "audit");
}

#[test]
fn station_phase_reflect_token() {
    assert_eq!(json_token(&StationPhase::Reflect), "reflect");
}

#[test]
fn station_phase_checkpoint_token() {
    assert_eq!(json_token(&StationPhase::Checkpoint), "checkpoint");
}

#[test]
fn station_phase_every_variant_roundtrips_json() {
    for (phase, token) in ALL_PHASES {
        assert_eq!(json_token(&phase), token);
        assert_eq!(from_token::<StationPhase>(token).unwrap(), phase);
    }
}

#[test]
fn station_phase_every_variant_roundtrips_yaml() {
    for (phase, _) in ALL_PHASES {
        assert_eq!(yaml_round(&phase), phase);
    }
}

#[test]
fn station_phase_rejects_unknown_token() {
    assert!(from_token::<StationPhase>("deploy").is_err());
    assert!(from_token::<StationPhase>("manufacturing").is_err());
    assert!(from_token::<StationPhase>("spec ").is_err());
}

#[test]
fn station_phase_rejects_capitalized() {
    assert!(from_token::<StationPhase>("Spec").is_err());
    assert!(from_token::<StationPhase>("Manufacture").is_err());
}

#[test]
fn station_phase_schema_lists_six_in_order() {
    let s = schema_value!(StationPhase);
    let tokens: Vec<&str> = s["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .map(|v| v["enum"][0].as_str().expect("token"))
        .collect();
    assert_eq!(
        tokens,
        vec!["spec", "review", "user_gate", "manufacture", "audit", "reflect", "checkpoint"]
    );
}

#[test]
fn station_phase_variants_are_distinct() {
    use std::collections::HashSet;
    let tokens: HashSet<&str> = ALL_PHASES.iter().map(|(_, t)| *t).collect();
    assert_eq!(tokens.len(), 6, "all phase tokens must be unique");
}

#[test]
fn station_phase_equality_distinguishes_spec_from_review() {
    assert_ne!(StationPhase::Spec, StationPhase::Review);
    assert_ne!(StationPhase::Audit, StationPhase::Reflect);
}

// ===========================================================================
// CheckpointKind
// ===========================================================================

const ALL_KINDS: [(CheckpointKind, &str); 4] = [
    (CheckpointKind::Auto, "auto"),
    (CheckpointKind::Ask, "ask"),
    (CheckpointKind::External, "external"),
    (CheckpointKind::Await, "await"),
];

#[test]
fn checkpoint_kind_has_four_variants() {
    assert_eq!(ALL_KINDS.len(), 4);
}

#[test]
fn checkpoint_kind_auto_token() {
    assert_eq!(json_token(&CheckpointKind::Auto), "auto");
}

#[test]
fn checkpoint_kind_ask_token() {
    assert_eq!(json_token(&CheckpointKind::Ask), "ask");
}

#[test]
fn checkpoint_kind_external_token() {
    assert_eq!(json_token(&CheckpointKind::External), "external");
}

#[test]
fn checkpoint_kind_await_token() {
    // `await` is a reserved word but the snake_case wire form is still "await".
    assert_eq!(json_token(&CheckpointKind::Await), "await");
}

#[test]
fn checkpoint_kind_every_variant_roundtrips() {
    for (kind, token) in ALL_KINDS {
        assert_eq!(json_token(&kind), token);
        assert_eq!(from_token::<CheckpointKind>(token).unwrap(), kind);
        assert_eq!(yaml_round(&kind), kind);
    }
}

#[test]
fn checkpoint_kind_rejects_unknown() {
    assert!(from_token::<CheckpointKind>("manual").is_err());
    assert!(from_token::<CheckpointKind>("automatic").is_err());
}

#[test]
fn checkpoint_kind_schema_tokens() {
    let s = schema_value!(CheckpointKind);
    let tokens: Vec<&str> = s["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["enum"][0].as_str().unwrap())
        .collect();
    assert_eq!(tokens, vec!["auto", "ask", "external", "await"]);
}

// ===========================================================================
// CheckpointOutcome
// ===========================================================================

const ALL_OUTCOMES: [(CheckpointOutcome, &str); 4] = [
    (CheckpointOutcome::Advanced, "advanced"),
    (CheckpointOutcome::Paused, "paused"),
    (CheckpointOutcome::Blocked, "blocked"),
    (CheckpointOutcome::Awaiting, "awaiting"),
];

#[test]
fn checkpoint_outcome_advanced_token() {
    assert_eq!(json_token(&CheckpointOutcome::Advanced), "advanced");
}

#[test]
fn checkpoint_outcome_paused_token() {
    assert_eq!(json_token(&CheckpointOutcome::Paused), "paused");
}

#[test]
fn checkpoint_outcome_blocked_token() {
    assert_eq!(json_token(&CheckpointOutcome::Blocked), "blocked");
}

#[test]
fn checkpoint_outcome_awaiting_token() {
    assert_eq!(json_token(&CheckpointOutcome::Awaiting), "awaiting");
}

#[test]
fn checkpoint_outcome_every_variant_roundtrips() {
    for (outcome, token) in ALL_OUTCOMES {
        assert_eq!(json_token(&outcome), token);
        assert_eq!(from_token::<CheckpointOutcome>(token).unwrap(), outcome);
        assert_eq!(yaml_round(&outcome), outcome);
    }
}

#[test]
fn checkpoint_outcome_blocked_shares_token_with_status_blocked() {
    // Both Status::Blocked and CheckpointOutcome::Blocked use "blocked"; they
    // are independent enums but the wire token must match for shared UI labels.
    assert_eq!(
        json_token(&CheckpointOutcome::Blocked),
        json_token(&Status::Blocked)
    );
}

#[test]
fn checkpoint_outcome_rejects_unknown() {
    assert!(from_token::<CheckpointOutcome>("advance").is_err());
    assert!(from_token::<CheckpointOutcome>("done").is_err());
}

#[test]
fn checkpoint_outcome_schema_tokens() {
    let s = schema_value!(CheckpointOutcome);
    let tokens: Vec<&str> = s["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["enum"][0].as_str().unwrap())
        .collect();
    assert_eq!(tokens, vec!["advanced", "paused", "blocked", "awaiting"]);
}

// ===========================================================================
// PassBeat
// ===========================================================================

const ALL_BEATS: [(PassBeat, &str); 3] = [
    (PassBeat::Make, "make"),
    (PassBeat::Challenge, "challenge"),
    (PassBeat::Resolve, "resolve"),
];

#[test]
fn pass_beat_make_token() {
    assert_eq!(json_token(&PassBeat::Make), "make");
}

#[test]
fn pass_beat_challenge_token() {
    assert_eq!(json_token(&PassBeat::Challenge), "challenge");
}

#[test]
fn pass_beat_resolve_token() {
    assert_eq!(json_token(&PassBeat::Resolve), "resolve");
}

#[test]
fn pass_beat_every_variant_roundtrips() {
    for (beat, token) in ALL_BEATS {
        assert_eq!(json_token(&beat), token);
        assert_eq!(from_token::<PassBeat>(token).unwrap(), beat);
        assert_eq!(yaml_round(&beat), beat);
    }
}

#[test]
fn pass_beat_rejects_unknown() {
    assert!(from_token::<PassBeat>("build").is_err());
    assert!(from_token::<PassBeat>("attack").is_err());
}

#[test]
fn pass_beat_three_beats_are_distinct() {
    assert_ne!(PassBeat::Make, PassBeat::Challenge);
    assert_ne!(PassBeat::Challenge, PassBeat::Resolve);
    assert_ne!(PassBeat::Make, PassBeat::Resolve);
}

// ===========================================================================
// FeedbackSeverity
// ===========================================================================

const ALL_SEVERITIES: [(FeedbackSeverity, &str); 4] = [
    (FeedbackSeverity::Blocker, "blocker"),
    (FeedbackSeverity::High, "high"),
    (FeedbackSeverity::Medium, "medium"),
    (FeedbackSeverity::Low, "low"),
];

#[test]
fn feedback_severity_blocker_token() {
    assert_eq!(json_token(&FeedbackSeverity::Blocker), "blocker");
}

#[test]
fn feedback_severity_high_token() {
    assert_eq!(json_token(&FeedbackSeverity::High), "high");
}

#[test]
fn feedback_severity_medium_token() {
    assert_eq!(json_token(&FeedbackSeverity::Medium), "medium");
}

#[test]
fn feedback_severity_low_token() {
    assert_eq!(json_token(&FeedbackSeverity::Low), "low");
}

#[test]
fn feedback_severity_every_variant_roundtrips() {
    for (sev, token) in ALL_SEVERITIES {
        assert_eq!(json_token(&sev), token);
        assert_eq!(from_token::<FeedbackSeverity>(token).unwrap(), sev);
        assert_eq!(yaml_round(&sev), sev);
    }
}

#[test]
fn feedback_severity_rejects_unknown() {
    assert!(from_token::<FeedbackSeverity>("critical").is_err());
    assert!(from_token::<FeedbackSeverity>("trivial").is_err());
}

#[test]
fn feedback_severity_schema_tokens() {
    let s = schema_value!(FeedbackSeverity);
    let tokens: Vec<&str> = s["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["enum"][0].as_str().unwrap())
        .collect();
    assert_eq!(tokens, vec!["blocker", "high", "medium", "low"]);
}

// ===========================================================================
// FeedbackStatus
// ===========================================================================

const ALL_FEEDBACK_STATUSES: [(FeedbackStatus, &str); 8] = [
    (FeedbackStatus::Pending, "pending"),
    (FeedbackStatus::Fixing, "fixing"),
    (FeedbackStatus::Addressed, "addressed"),
    (FeedbackStatus::Answered, "answered"),
    (FeedbackStatus::NonActionable, "non_actionable"),
    (FeedbackStatus::Escalated, "escalated"),
    (FeedbackStatus::Closed, "closed"),
    (FeedbackStatus::Rejected, "rejected"),
];

#[test]
fn feedback_status_has_eight_variants() {
    assert_eq!(ALL_FEEDBACK_STATUSES.len(), 8);
}

#[test]
fn feedback_status_pending_token() {
    assert_eq!(json_token(&FeedbackStatus::Pending), "pending");
}

#[test]
fn feedback_status_fixing_token() {
    assert_eq!(json_token(&FeedbackStatus::Fixing), "fixing");
}

#[test]
fn feedback_status_addressed_token() {
    assert_eq!(json_token(&FeedbackStatus::Addressed), "addressed");
}

#[test]
fn feedback_status_answered_token() {
    assert_eq!(json_token(&FeedbackStatus::Answered), "answered");
}

#[test]
fn feedback_status_non_actionable_token_is_snake_case() {
    assert_eq!(json_token(&FeedbackStatus::NonActionable), "non_actionable");
}

#[test]
fn feedback_status_escalated_token() {
    assert_eq!(json_token(&FeedbackStatus::Escalated), "escalated");
}

#[test]
fn feedback_status_closed_token() {
    assert_eq!(json_token(&FeedbackStatus::Closed), "closed");
}

#[test]
fn feedback_status_rejected_token() {
    assert_eq!(json_token(&FeedbackStatus::Rejected), "rejected");
}

#[test]
fn feedback_status_every_variant_roundtrips() {
    for (status, token) in ALL_FEEDBACK_STATUSES {
        assert_eq!(json_token(&status), token);
        assert_eq!(from_token::<FeedbackStatus>(token).unwrap(), status);
        assert_eq!(yaml_round(&status), status);
    }
}

#[test]
fn feedback_status_non_actionable_rejects_camel() {
    assert!(from_token::<FeedbackStatus>("nonActionable").is_err());
    assert!(from_token::<FeedbackStatus>("NonActionable").is_err());
}

#[test]
fn feedback_status_rejects_unknown() {
    assert!(from_token::<FeedbackStatus>("open").is_err());
    assert!(from_token::<FeedbackStatus>("resolved").is_err());
}

#[test]
fn feedback_status_schema_lists_eight() {
    let s = schema_value!(FeedbackStatus);
    let tokens: Vec<&str> = s["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["enum"][0].as_str().unwrap())
        .collect();
    assert_eq!(tokens.len(), 8);
    assert!(tokens.contains(&"non_actionable"));
}

#[test]
fn feedback_status_tokens_are_unique() {
    use std::collections::HashSet;
    let set: HashSet<&str> = ALL_FEEDBACK_STATUSES.iter().map(|(_, t)| *t).collect();
    assert_eq!(set.len(), 8);
}

// ===========================================================================
// RunGit
// ===========================================================================

#[test]
fn run_git_default_is_all_falsey() {
    let g = RunGit::default();
    assert_eq!(g.change_strategy, "");
    assert!(!g.auto_merge);
    assert!(!g.auto_squash);
}

#[test]
fn run_git_default_serializes_all_three_fields() {
    // RunGit has no skip rules, so every field is always present.
    let o = obj(&RunGit::default());
    assert!(o.contains_key("change_strategy"));
    assert!(o.contains_key("auto_merge"));
    assert!(o.contains_key("auto_squash"));
}

#[test]
fn run_git_default_field_values_on_wire() {
    let o = obj(&RunGit::default());
    assert_eq!(o["change_strategy"], "");
    assert_eq!(o["auto_merge"], false);
    assert_eq!(o["auto_squash"], false);
}

#[test]
fn run_git_roundtrips_json() {
    let g = RunGit {
        change_strategy: "worktree-per-unit".into(),
        auto_merge: true,
        auto_squash: true,
    };
    let back = json_round(&g);
    assert_eq!(back.change_strategy, "worktree-per-unit");
    assert!(back.auto_merge);
    assert!(back.auto_squash);
}

#[test]
fn run_git_roundtrips_yaml() {
    let g = RunGit {
        change_strategy: "single-branch".into(),
        auto_merge: false,
        auto_squash: true,
    };
    let back = yaml_round(&g);
    assert_eq!(back.change_strategy, "single-branch");
    assert!(!back.auto_merge);
    assert!(back.auto_squash);
}

#[test]
fn run_git_fills_defaults_from_empty_object() {
    let g: RunGit = serde_json::from_str("{}").expect("de");
    assert_eq!(g.change_strategy, "");
    assert!(!g.auto_merge);
    assert!(!g.auto_squash);
}

#[test]
fn run_git_fills_defaults_from_partial() {
    let g: RunGit = serde_json::from_str(r#"{"auto_merge":true}"#).expect("de");
    assert!(g.auto_merge);
    assert!(!g.auto_squash);
    assert_eq!(g.change_strategy, "");
}

#[test]
fn run_git_tolerates_unknown_field() {
    let g: RunGit =
        serde_json::from_str(r#"{"change_strategy":"x","extra":42}"#).expect("de");
    assert_eq!(g.change_strategy, "x");
}

#[test]
fn run_git_independent_bool_flags() {
    let g: RunGit =
        serde_json::from_str(r#"{"auto_merge":false,"auto_squash":true}"#).expect("de");
    assert!(!g.auto_merge);
    assert!(g.auto_squash);
}

#[test]
fn run_git_schema_marks_no_required_fields() {
    // Every field has a serde default, so none are required.
    let s = schema_value!(RunGit);
    assert!(s.get("required").is_none() || s["required"].as_array().unwrap().is_empty());
}

#[test]
fn run_git_schema_lists_all_properties() {
    let s = schema_value!(RunGit);
    let props = s["properties"].as_object().expect("properties");
    assert!(props.contains_key("change_strategy"));
    assert!(props.contains_key("auto_merge"));
    assert!(props.contains_key("auto_squash"));
}

// ===========================================================================
// RunFrontmatter
// ===========================================================================

fn minimal_run_fm() -> RunFrontmatter {
    RunFrontmatter {
        factory: "software".into(),
        ..Default::default()
    }
}

#[test]
fn run_frontmatter_default_has_empty_factory() {
    assert_eq!(RunFrontmatter::default().factory, "");
}

#[test]
fn run_frontmatter_default_status_is_pending() {
    assert_eq!(RunFrontmatter::default().status, Status::Pending);
}

#[test]
fn run_frontmatter_skips_none_title() {
    assert!(!obj(&minimal_run_fm()).contains_key("title"));
}

#[test]
fn run_frontmatter_skips_none_archived() {
    assert!(!obj(&minimal_run_fm()).contains_key("archived"));
}

#[test]
fn run_frontmatter_skips_none_started_at() {
    assert!(!obj(&minimal_run_fm()).contains_key("started_at"));
}

#[test]
fn run_frontmatter_skips_none_completed_at() {
    assert!(!obj(&minimal_run_fm()).contains_key("completed_at"));
}

#[test]
fn run_frontmatter_skips_none_git() {
    assert!(!obj(&minimal_run_fm()).contains_key("git"));
}

#[test]
fn run_frontmatter_always_emits_factory() {
    assert_eq!(obj(&minimal_run_fm())["factory"], "software");
}

#[test]
fn run_frontmatter_always_emits_mode_even_when_empty() {
    // mode has #[serde(default)] but no skip rule -> always present as its token.
    assert_eq!(obj(&minimal_run_fm())["mode"], "solo");
}

#[test]
fn run_frontmatter_always_emits_active_station() {
    assert_eq!(obj(&minimal_run_fm())["active_station"], "");
}

#[test]
fn run_frontmatter_always_emits_status() {
    assert_eq!(obj(&minimal_run_fm())["status"], "pending");
}

#[test]
fn run_frontmatter_emits_title_when_present() {
    let fm = RunFrontmatter {
        title: Some("Ship It".into()),
        ..minimal_run_fm()
    };
    assert_eq!(obj(&fm)["title"], "Ship It");
}

#[test]
fn run_frontmatter_emits_archived_when_true() {
    let fm = RunFrontmatter {
        archived: Some(true),
        ..minimal_run_fm()
    };
    assert_eq!(obj(&fm)["archived"], true);
}

#[test]
fn run_frontmatter_emits_archived_when_false() {
    // archived: Some(false) is still Some, so it must be emitted.
    let fm = RunFrontmatter {
        archived: Some(false),
        ..minimal_run_fm()
    };
    assert_eq!(obj(&fm)["archived"], false);
}

#[test]
fn run_frontmatter_emits_git_when_present() {
    let fm = RunFrontmatter {
        git: Some(RunGit::default()),
        ..minimal_run_fm()
    };
    assert!(obj(&fm).contains_key("git"));
}

#[test]
fn run_frontmatter_defaults_fill_missing_fields_from_yaml() {
    let fm: RunFrontmatter = serde_yaml::from_str("factory: software\n").expect("de");
    assert_eq!(fm.factory, "software");
    assert_eq!(fm.mode, Mode::Solo);
    assert_eq!(fm.active_station, "");
    assert_eq!(fm.status, Status::Pending);
    assert_eq!(fm.archived, None);
    assert!(fm.git.is_none());
    assert!(fm.title.is_none());
}

#[test]
fn run_frontmatter_missing_factory_is_an_error() {
    // factory is the one field with no serde default -> required.
    assert!(serde_yaml::from_str::<RunFrontmatter>("mode: continuous\n").is_err());
}

#[test]
fn run_frontmatter_full_roundtrip_json() {
    let fm = RunFrontmatter {
        title: Some("Big Run".into()),
        factory: "software".into(),
        mode: Mode::Solo,
        active_station: "frame".into(),
        status: Status::Active,
        surface: Some(Surface::WebUi),
        archived: Some(false),
        started_at: Some("2026-05-30T00:00:00Z".into()),
        completed_at: None,
        git: Some(RunGit {
            change_strategy: "wt".into(),
            auto_merge: true,
            auto_squash: false,
        }),
        seal: None,
        external_refs: ExternalRefs {
            ticket: Some("JIRA-1".into()),
            pr_url: Some("https://x/pr/1".into()),
            ..Default::default()
        },
        created_by: Some("jason@example.com".into()),
        composite: None,
        sync: vec![],
        composite_state: Default::default(),
    };
    let back = json_round(&fm);
    assert_eq!(back.created_by.as_deref(), Some("jason@example.com"));
    assert_eq!(back.surface, Some(Surface::WebUi));
    assert_eq!(back.title.as_deref(), Some("Big Run"));
    assert_eq!(back.factory, "software");
    assert_eq!(back.mode, Mode::Solo);
    assert_eq!(back.active_station, "frame");
    assert_eq!(back.status, Status::Active);
    assert_eq!(back.archived, Some(false));
    assert_eq!(back.started_at.as_deref(), Some("2026-05-30T00:00:00Z"));
    assert!(back.completed_at.is_none());
    assert!(back.git.unwrap().auto_merge);
    assert_eq!(back.external_refs.ticket.as_deref(), Some("JIRA-1"));
}

#[test]
fn run_frontmatter_full_roundtrip_yaml() {
    let fm = RunFrontmatter {
        title: Some("Y".into()),
        factory: "f".into(),
        mode: Mode::Team,
        active_station: "s".into(),
        status: Status::Completed,
        surface: Some(Surface::Cli),
        archived: Some(true),
        started_at: Some("t1".into()),
        completed_at: Some("t2".into()),
        git: Some(RunGit::default()),
        seal: None,
        external_refs: Default::default(),
        created_by: None,
        composite: None,
        sync: vec![],
        composite_state: Default::default(),
    };
    let back = yaml_round(&fm);
    assert_eq!(back.status, Status::Completed);
    assert_eq!(back.archived, Some(true));
    assert_eq!(back.completed_at.as_deref(), Some("t2"));
}

#[test]
fn run_frontmatter_status_active_parses_from_yaml() {
    let fm: RunFrontmatter =
        serde_yaml::from_str("factory: f\nstatus: active\n").expect("de");
    assert_eq!(fm.status, Status::Active);
}

#[test]
fn run_frontmatter_status_in_progress_parses() {
    let fm: RunFrontmatter =
        serde_yaml::from_str("factory: f\nstatus: in_progress\n").expect("de");
    assert_eq!(fm.status, Status::InProgress);
}

#[test]
fn run_frontmatter_tolerates_unknown_field() {
    let fm: RunFrontmatter =
        serde_yaml::from_str("factory: f\nmystery: 7\n").expect("de");
    assert_eq!(fm.factory, "f");
}

#[test]
fn run_frontmatter_nested_git_roundtrips_through_yaml() {
    let fm = RunFrontmatter {
        factory: "software".into(),
        git: Some(RunGit {
            change_strategy: "worktree-per-unit".into(),
            auto_merge: true,
            auto_squash: false,
        }),
        ..Default::default()
    };
    let back = yaml_round(&fm);
    let git = back.git.expect("git");
    assert_eq!(git.change_strategy, "worktree-per-unit");
    assert!(git.auto_merge);
    assert!(!git.auto_squash);
}

#[test]
fn run_frontmatter_schema_requires_only_factory() {
    let s = schema_value!(RunFrontmatter);
    let required: Vec<&str> = s["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["factory"]);
}

#[test]
fn run_frontmatter_schema_has_all_properties() {
    let s = schema_value!(RunFrontmatter);
    let props = s["properties"].as_object().unwrap();
    for key in [
        "title",
        "factory",
        "mode",
        "active_station",
        "status",
        "archived",
        "started_at",
        "completed_at",
        "git",
    ] {
        assert!(props.contains_key(key), "schema missing property {key}");
    }
}

#[test]
fn run_frontmatter_clone_is_independent() {
    let fm = minimal_run_fm();
    let mut clone = fm.clone();
    clone.factory = "other".into();
    assert_eq!(fm.factory, "software");
    assert_eq!(clone.factory, "other");
}

// ===========================================================================
// Run
// ===========================================================================

fn sample_run() -> Run {
    Run {
        slug: "my-run".into(),
        frontmatter: minimal_run_fm(),
        title: "My Run".into(),
        body: "# My Run\n\nbody\n".into(),
    }
}

#[test]
fn run_roundtrips_json() {
    let back = json_round(&sample_run());
    assert_eq!(back.slug, "my-run");
    assert_eq!(back.title, "My Run");
    assert_eq!(back.frontmatter.factory, "software");
    assert!(back.body.contains("body"));
}

#[test]
fn run_roundtrips_yaml() {
    let back = yaml_round(&sample_run());
    assert_eq!(back.slug, "my-run");
    assert_eq!(back.frontmatter.factory, "software");
}

#[test]
fn run_all_four_fields_required_in_schema() {
    let s = schema_value!(Run);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for key in ["slug", "frontmatter", "title", "body"] {
        assert!(required.contains(&key), "schema must require {key}");
    }
}

#[test]
fn run_missing_slug_is_an_error() {
    let v = json!({
        "frontmatter": minimal_run_fm(),
        "title": "t",
        "body": "b"
    });
    assert!(serde_json::from_value::<Run>(v).is_err());
}

#[test]
fn run_empty_body_roundtrips() {
    let run = Run {
        body: String::new(),
        ..sample_run()
    };
    assert_eq!(json_round(&run).body, "");
}

#[test]
fn run_preserves_multiline_body() {
    let run = Run {
        body: "line1\nline2\n\nline4\n".into(),
        ..sample_run()
    };
    assert_eq!(json_round(&run).body, "line1\nline2\n\nline4\n");
}

#[test]
fn run_preserves_unicode_in_title() {
    let run = Run {
        title: "Café — déjà vu 🚀".into(),
        ..sample_run()
    };
    assert_eq!(json_round(&run).title, "Café — déjà vu 🚀");
}

#[test]
fn run_nested_frontmatter_status_roundtrips() {
    let run = Run {
        frontmatter: RunFrontmatter {
            factory: "f".into(),
            status: Status::Blocked,
            ..Default::default()
        },
        ..sample_run()
    };
    assert_eq!(json_round(&run).frontmatter.status, Status::Blocked);
}

#[test]
fn run_schema_references_run_frontmatter_definition() {
    let s = schema_value!(Run);
    assert!(s["definitions"].get("RunFrontmatter").is_some());
}

// ===========================================================================
// UnitFrontmatter
// ===========================================================================

#[test]
fn unit_default_pass_is_zero_derived_from_empty_iterations() {
    use darkrun_core::domain::Unit;
    let u = Unit {
        slug: "u".into(),
        frontmatter: UnitFrontmatter::default(),
        title: "u".into(),
        body: String::new(),
    };
    assert_eq!(u.pass(), 0);
}

#[test]
fn unit_frontmatter_default_worker_is_empty() {
    assert_eq!(UnitFrontmatter::default().worker, "");
}

#[test]
fn unit_frontmatter_default_status_is_pending() {
    assert_eq!(UnitFrontmatter::default().status, Status::Pending);
}

#[test]
fn unit_frontmatter_default_depends_on_is_empty() {
    assert!(UnitFrontmatter::default().depends_on.is_empty());
}

#[test]
fn unit_frontmatter_skips_empty_inputs() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("inputs"));
}

#[test]
fn unit_frontmatter_skips_empty_outputs() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("outputs"));
}

#[test]
fn unit_frontmatter_skips_none_name() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("name"));
}

#[test]
fn unit_frontmatter_skips_none_model() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("model"));
}

#[test]
fn unit_frontmatter_skips_none_station() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("station"));
}

#[test]
fn unit_frontmatter_skips_none_started_at() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("started_at"));
}

#[test]
fn unit_frontmatter_skips_none_completed_at() {
    assert!(!obj(&UnitFrontmatter::default()).contains_key("completed_at"));
}

#[test]
fn unit_frontmatter_depends_on_always_present_as_empty_array() {
    // depends_on has #[serde(default)] but no skip rule -> always an array.
    let o = obj(&UnitFrontmatter::default());
    assert_eq!(o["depends_on"].as_array().unwrap().len(), 0);
}

#[test]
fn unit_frontmatter_always_emits_unit_type() {
    assert_eq!(obj(&UnitFrontmatter::default())["unit_type"], "");
}

#[test]
fn unit_frontmatter_always_emits_status() {
    assert_eq!(obj(&UnitFrontmatter::default())["status"], "pending");
}

#[test]
fn unit_frontmatter_does_not_emit_a_stored_pass() {
    // `pass` is derived, never serialized.
    assert!(obj(&UnitFrontmatter::default()).get("pass").is_none());
}

#[test]
fn unit_frontmatter_always_emits_worker() {
    assert_eq!(obj(&UnitFrontmatter::default())["worker"], "");
}

#[test]
fn unit_frontmatter_emits_inputs_when_nonempty() {
    let fm = UnitFrontmatter {
        inputs: vec!["a.md".into(), "b.md".into()],
        ..Default::default()
    };
    let o = obj(&fm);
    assert_eq!(o["inputs"].as_array().unwrap().len(), 2);
}

#[test]
fn unit_frontmatter_emits_outputs_when_nonempty() {
    let fm = UnitFrontmatter {
        outputs: vec!["out.md".into()],
        ..Default::default()
    };
    assert!(obj(&fm).contains_key("outputs"));
}

#[test]
fn unit_frontmatter_emits_name_when_present() {
    let fm = UnitFrontmatter {
        name: Some("Auth".into()),
        ..Default::default()
    };
    assert_eq!(obj(&fm)["name"], "Auth");
}

#[test]
fn unit_frontmatter_emits_station_when_present() {
    let fm = UnitFrontmatter {
        station: Some("build".into()),
        ..Default::default()
    };
    assert_eq!(obj(&fm)["station"], "build");
}

#[test]
fn unit_frontmatter_depends_on_order_preserved() {
    let fm = UnitFrontmatter {
        depends_on: vec!["c".into(), "a".into(), "b".into()],
        ..Default::default()
    };
    let back = json_round(&fm);
    assert_eq!(back.depends_on, vec!["c", "a", "b"]);
}

#[test]
fn unit_frontmatter_full_roundtrip_json() {
    let fm = UnitFrontmatter {
        name: Some("Login".into()),
        unit_type: "feature".into(),
        status: Status::InProgress,
        depends_on: vec!["dep1".into()],
        worker: "builder".into(),
        model: Some("opus".into()),
        station: Some("build".into()),
        revise: false,
        inputs: vec!["spec.md".into()],
        outputs: vec!["impl.rs".into()],
        started_at: Some("t1".into()),
        completed_at: Some("t2".into()),
        ..Default::default()
    };
    let back = json_round(&fm);
    assert_eq!(back.name.as_deref(), Some("Login"));
    assert_eq!(back.unit_type, "feature");
    assert_eq!(back.status, Status::InProgress);
    assert_eq!(back.depends_on, vec!["dep1"]);
    assert_eq!(back.worker, "builder");
    assert_eq!(back.model.as_deref(), Some("opus"));
    assert_eq!(back.station.as_deref(), Some("build"));
    assert_eq!(back.inputs, vec!["spec.md"]);
    assert_eq!(back.outputs, vec!["impl.rs"]);
    assert_eq!(back.started_at.as_deref(), Some("t1"));
    assert_eq!(back.completed_at.as_deref(), Some("t2"));
}

#[test]
fn unit_frontmatter_full_roundtrip_yaml() {
    let fm = UnitFrontmatter {
        unit_type: "task".into(),
        status: Status::Completed,
        worker: "challenger".into(),
        ..Default::default()
    };
    let back = yaml_round(&fm);
    assert_eq!(back.unit_type, "task");
    assert_eq!(back.status, Status::Completed);
    assert_eq!(back.worker, "challenger");
}

#[test]
fn unit_frontmatter_empty_object_yields_default() {
    let fm: UnitFrontmatter = serde_json::from_str("{}").expect("de");
    assert_eq!(fm.worker, "");
    assert_eq!(fm.status, Status::Pending);
    assert!(fm.depends_on.is_empty());
    assert!(fm.name.is_none());
}

#[test]
fn unit_frontmatter_tolerates_unknown_field() {
    let fm: UnitFrontmatter =
        serde_json::from_str(r#"{"unit_type":"x","legacy_bolt":2}"#).expect("de");
    assert_eq!(fm.unit_type, "x");
}

#[test]
fn unit_frontmatter_schema_has_no_required_fields() {
    // Every field defaults, so `required` is absent or empty.
    let s = schema_value!(UnitFrontmatter);
    assert!(s.get("required").is_none() || s["required"].as_array().unwrap().is_empty());
}

#[test]
fn unit_frontmatter_schema_lists_all_properties() {
    let s = schema_value!(UnitFrontmatter);
    let props = s["properties"].as_object().unwrap();
    for key in [
        "name",
        "unit_type",
        "status",
        "depends_on",
        "worker",
        "model",
        "station",
        "inputs",
        "outputs",
        "started_at",
        "completed_at",
        "iterations",
    ] {
        assert!(props.contains_key(key), "missing property {key}");
    }
    assert!(!props.contains_key("pass"), "pass is derived, not a schema property");
}

#[test]
fn unit_frontmatter_inputs_outputs_roundtrip_through_yaml() {
    let fm = UnitFrontmatter {
        inputs: vec!["a".into(), "b".into(), "c".into()],
        outputs: vec!["x".into()],
        ..Default::default()
    };
    let back = yaml_round(&fm);
    assert_eq!(back.inputs.len(), 3);
    assert_eq!(back.outputs, vec!["x"]);
}

// ===========================================================================
// Unit (+ helpers)
// ===========================================================================

fn sample_unit() -> Unit {
    Unit {
        slug: "u1".into(),
        frontmatter: UnitFrontmatter::default(),
        title: "U1".into(),
        body: "body".into(),
    }
}

#[test]
fn unit_roundtrips_json() {
    let back = json_round(&sample_unit());
    assert_eq!(back.slug, "u1");
    assert_eq!(back.title, "U1");
    assert_eq!(back.body, "body");
}

#[test]
fn unit_status_helper_reflects_frontmatter() {
    let unit = Unit {
        frontmatter: UnitFrontmatter {
            status: Status::Active,
            ..Default::default()
        },
        ..sample_unit()
    };
    assert_eq!(unit.status(), Status::Active);
}

#[test]
fn unit_status_helper_default_is_pending() {
    assert_eq!(sample_unit().status(), Status::Pending);
}

#[test]
fn unit_station_helper_defaults_to_root() {
    assert_eq!(sample_unit().station(), "_root");
}

#[test]
fn unit_station_helper_uses_explicit_value() {
    let unit = Unit {
        frontmatter: UnitFrontmatter {
            station: Some("build".into()),
            ..Default::default()
        },
        ..sample_unit()
    };
    assert_eq!(unit.station(), "build");
}

#[test]
fn unit_station_helper_handles_empty_string_station() {
    // An explicit empty string is still Some(""), not the synthetic root.
    let unit = Unit {
        frontmatter: UnitFrontmatter {
            station: Some(String::new()),
            ..Default::default()
        },
        ..sample_unit()
    };
    assert_eq!(unit.station(), "");
}

#[test]
fn unit_status_helper_survives_roundtrip() {
    let unit = Unit {
        frontmatter: UnitFrontmatter {
            status: Status::Completed,
            ..Default::default()
        },
        ..sample_unit()
    };
    assert_eq!(json_round(&unit).status(), Status::Completed);
}

#[test]
fn unit_all_four_fields_required_in_schema() {
    let s = schema_value!(Unit);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for key in ["slug", "frontmatter", "title", "body"] {
        assert!(required.contains(&key));
    }
}

#[test]
fn unit_missing_title_is_an_error() {
    let v = json!({ "slug": "u", "frontmatter": UnitFrontmatter::default(), "body": "b" });
    assert!(serde_json::from_value::<Unit>(v).is_err());
}

#[test]
fn unit_preserves_empty_body() {
    let unit = Unit {
        body: String::new(),
        ..sample_unit()
    };
    assert_eq!(json_round(&unit).body, "");
}

#[test]
fn unit_nested_frontmatter_depends_on_roundtrips() {
    let unit = Unit {
        frontmatter: UnitFrontmatter {
            depends_on: vec!["a".into(), "b".into()],
            ..Default::default()
        },
        ..sample_unit()
    };
    assert_eq!(json_round(&unit).frontmatter.depends_on, vec!["a", "b"]);
}

// ===========================================================================
// Pass
// ===========================================================================

#[test]
fn pass_roundtrips_json() {
    let pass = Pass {
        index: 2,
        unit: "u1".into(),
        beat: PassBeat::Challenge,
    };
    let back = json_round(&pass);
    assert_eq!(back.index, 2);
    assert_eq!(back.unit, "u1");
    assert_eq!(back.beat, PassBeat::Challenge);
}

#[test]
fn pass_index_zero_roundtrips() {
    let pass = Pass {
        index: 0,
        unit: "u".into(),
        beat: PassBeat::Make,
    };
    assert_eq!(json_round(&pass).index, 0);
}

#[test]
fn pass_index_max_u32_roundtrips() {
    let pass = Pass {
        index: u32::MAX,
        unit: "u".into(),
        beat: PassBeat::Resolve,
    };
    assert_eq!(json_round(&pass).index, u32::MAX);
}

#[test]
fn pass_all_beats_roundtrip_in_struct() {
    for (beat, _) in ALL_BEATS {
        let pass = Pass {
            index: 1,
            unit: "u".into(),
            beat,
        };
        assert_eq!(json_round(&pass).beat, beat);
    }
}

#[test]
fn pass_all_three_fields_required() {
    let s = schema_value!(Pass);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for key in ["index", "unit", "beat"] {
        assert!(required.contains(&key));
    }
}

#[test]
fn pass_missing_beat_is_an_error() {
    assert!(serde_json::from_str::<Pass>(r#"{"index":0,"unit":"u"}"#).is_err());
}

#[test]
fn pass_wire_form_uses_snake_case_beat() {
    let pass = Pass {
        index: 0,
        unit: "u".into(),
        beat: PassBeat::Challenge,
    };
    assert_eq!(obj(&pass)["beat"], "challenge");
}

#[test]
fn pass_yaml_roundtrips() {
    let pass = Pass {
        index: 5,
        unit: "auth".into(),
        beat: PassBeat::Make,
    };
    let back = yaml_round(&pass);
    assert_eq!(back.index, 5);
    assert_eq!(back.unit, "auth");
    assert_eq!(back.beat, PassBeat::Make);
}

// ===========================================================================
// Worker
// ===========================================================================

#[test]
fn worker_full_roundtrips() {
    let w = Worker {
        name: "builder".into(),
        model: Some("opus".into()),
        terminal: true,
    };
    let back = json_round(&w);
    assert_eq!(back.name, "builder");
    assert_eq!(back.model.as_deref(), Some("opus"));
    assert!(back.terminal);
}

#[test]
fn worker_minimal_defaults_terminal_false() {
    let w: Worker = serde_json::from_str(r#"{"name":"x"}"#).expect("de");
    assert!(!w.terminal);
    assert!(w.model.is_none());
}

#[test]
fn worker_skips_none_model() {
    let w = Worker {
        name: "x".into(),
        model: None,
        terminal: false,
    };
    assert!(!obj(&w).contains_key("model"));
}

#[test]
fn worker_emits_model_when_present() {
    let w = Worker {
        name: "x".into(),
        model: Some("sonnet".into()),
        terminal: false,
    };
    assert_eq!(obj(&w)["model"], "sonnet");
}

#[test]
fn worker_always_emits_terminal() {
    // terminal has #[serde(default)] but no skip -> always present.
    let w = Worker {
        name: "x".into(),
        model: None,
        terminal: false,
    };
    assert_eq!(obj(&w)["terminal"], false);
}

#[test]
fn worker_terminal_true_on_wire() {
    let w = Worker {
        name: "x".into(),
        model: None,
        terminal: true,
    };
    assert_eq!(obj(&w)["terminal"], true);
}

#[test]
fn worker_name_required_in_schema() {
    let s = schema_value!(Worker);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["name"]);
}

#[test]
fn worker_missing_name_is_an_error() {
    assert!(serde_json::from_str::<Worker>(r#"{"terminal":true}"#).is_err());
}

#[test]
fn worker_tolerates_unknown_field() {
    let w: Worker = serde_json::from_str(r#"{"name":"x","hat":"old"}"#).expect("de");
    assert_eq!(w.name, "x");
}

#[test]
fn worker_yaml_roundtrips() {
    let w = Worker {
        name: "reviewer".into(),
        model: Some("sonnet-4".into()),
        terminal: true,
    };
    let back = yaml_round(&w);
    assert_eq!(back.name, "reviewer");
    assert!(back.terminal);
}

// ===========================================================================
// Explorer
// ===========================================================================

#[test]
fn explorer_roundtrips() {
    let e = Explorer {
        name: "context".into(),
        mandate: "gather constraints".into(),
    };
    let back = json_round(&e);
    assert_eq!(back.name, "context");
    assert_eq!(back.mandate, "gather constraints");
}

#[test]
fn explorer_mandate_defaults_to_empty() {
    let e: Explorer = serde_json::from_str(r#"{"name":"v"}"#).expect("de");
    assert_eq!(e.mandate, "");
}

#[test]
fn explorer_always_emits_mandate() {
    let e = Explorer {
        name: "x".into(),
        mandate: String::new(),
    };
    assert_eq!(obj(&e)["mandate"], "");
}

#[test]
fn explorer_name_required() {
    let s = schema_value!(Explorer);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["name"]);
}

#[test]
fn explorer_missing_name_is_an_error() {
    assert!(serde_json::from_str::<Explorer>(r#"{"mandate":"x"}"#).is_err());
}

#[test]
fn explorer_yaml_roundtrips() {
    let e = Explorer {
        name: "value".into(),
        mandate: "find the user value".into(),
    };
    assert_eq!(yaml_round(&e).mandate, "find the user value");
}

#[test]
fn explorer_preserves_multiline_mandate() {
    let e = Explorer {
        name: "x".into(),
        mandate: "line1\nline2".into(),
    };
    assert_eq!(json_round(&e).mandate, "line1\nline2");
}

// ===========================================================================
// Reviewer
// ===========================================================================

#[test]
fn reviewer_roundtrips() {
    let r = Reviewer {
        name: "value".into(),
        dimension: "user-value".into(),
    };
    let back = json_round(&r);
    assert_eq!(back.name, "value");
    assert_eq!(back.dimension, "user-value");
}

#[test]
fn reviewer_dimension_defaults_to_empty() {
    let r: Reviewer = serde_json::from_str(r#"{"name":"feasibility"}"#).expect("de");
    assert_eq!(r.dimension, "");
}

#[test]
fn reviewer_always_emits_dimension() {
    let r = Reviewer {
        name: "x".into(),
        dimension: String::new(),
    };
    assert_eq!(obj(&r)["dimension"], "");
}

#[test]
fn reviewer_name_required() {
    let s = schema_value!(Reviewer);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["name"]);
}

#[test]
fn reviewer_missing_name_is_an_error() {
    assert!(serde_json::from_str::<Reviewer>(r#"{"dimension":"x"}"#).is_err());
}

#[test]
fn reviewer_yaml_roundtrips() {
    let r = Reviewer {
        name: "feasibility".into(),
        dimension: "can we build it".into(),
    };
    assert_eq!(yaml_round(&r).dimension, "can we build it");
}

// ===========================================================================
// Checkpoint
// ===========================================================================

#[test]
fn checkpoint_minimal_roundtrips() {
    let cp = Checkpoint {
        kind: CheckpointKind::Auto,
        entered_at: None,
        outcome: None,
    };
    let back = json_round(&cp);
    assert_eq!(back.kind, CheckpointKind::Auto);
    assert!(back.entered_at.is_none());
    assert!(back.outcome.is_none());
}

#[test]
fn checkpoint_full_roundtrips() {
    let cp = Checkpoint {
        kind: CheckpointKind::Await,
        entered_at: Some("2026-05-30T00:00:00Z".into()),
        outcome: Some(CheckpointOutcome::Awaiting),
    };
    let back = json_round(&cp);
    assert_eq!(back.kind, CheckpointKind::Await);
    assert_eq!(back.entered_at.as_deref(), Some("2026-05-30T00:00:00Z"));
    assert_eq!(back.outcome, Some(CheckpointOutcome::Awaiting));
}

#[test]
fn checkpoint_skips_none_entered_at() {
    let cp = Checkpoint {
        kind: CheckpointKind::Ask,
        entered_at: None,
        outcome: None,
    };
    assert!(!obj(&cp).contains_key("entered_at"));
}

#[test]
fn checkpoint_skips_none_outcome() {
    let cp = Checkpoint {
        kind: CheckpointKind::Ask,
        entered_at: None,
        outcome: None,
    };
    assert!(!obj(&cp).contains_key("outcome"));
}

#[test]
fn checkpoint_emits_outcome_when_present() {
    let cp = Checkpoint {
        kind: CheckpointKind::Auto,
        entered_at: None,
        outcome: Some(CheckpointOutcome::Advanced),
    };
    assert_eq!(obj(&cp)["outcome"], "advanced");
}

#[test]
fn checkpoint_always_emits_kind() {
    let cp = Checkpoint {
        kind: CheckpointKind::External,
        entered_at: None,
        outcome: None,
    };
    assert_eq!(obj(&cp)["kind"], "external");
}

#[test]
fn checkpoint_kind_required_in_schema() {
    let s = schema_value!(Checkpoint);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["kind"]);
}

#[test]
fn checkpoint_missing_kind_is_an_error() {
    assert!(serde_json::from_str::<Checkpoint>(r#"{"outcome":"paused"}"#).is_err());
}

#[test]
fn checkpoint_every_kind_outcome_pair_roundtrips() {
    for (kind, _) in ALL_KINDS {
        for (outcome, _) in ALL_OUTCOMES {
            let cp = Checkpoint {
                kind,
                entered_at: Some("t".into()),
                outcome: Some(outcome),
            };
            let back = json_round(&cp);
            assert_eq!(back.kind, kind);
            assert_eq!(back.outcome, Some(outcome));
        }
    }
}

#[test]
fn checkpoint_schema_references_kind_and_outcome_definitions() {
    let s = schema_value!(Checkpoint);
    let defs = s["definitions"].as_object().expect("definitions");
    assert!(defs.contains_key("CheckpointKind"));
    assert!(defs.contains_key("CheckpointOutcome"));
}

#[test]
fn checkpoint_yaml_roundtrips() {
    let cp = Checkpoint {
        kind: CheckpointKind::Await,
        entered_at: Some("t".into()),
        outcome: Some(CheckpointOutcome::Blocked),
    };
    let back = yaml_round(&cp);
    assert_eq!(back.kind, CheckpointKind::Await);
    assert_eq!(back.outcome, Some(CheckpointOutcome::Blocked));
}

// ===========================================================================
// Station
// ===========================================================================

fn sample_station() -> Station {
    Station {
        station: "build".into(),
        status: Status::Active,
        phase: StationPhase::Manufacture,
            elaborated: false,
        checkpoint: None,
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: None,
        completed_at: None,
    }
}

#[test]
fn station_minimal_roundtrips() {
    let back = json_round(&sample_station());
    assert_eq!(back.station, "build");
    assert_eq!(back.status, Status::Active);
    assert_eq!(back.phase, StationPhase::Manufacture);
}

#[test]
fn station_skips_none_checkpoint() {
    assert!(!obj(&sample_station()).contains_key("checkpoint"));
}

#[test]
fn station_skips_none_started_at() {
    assert!(!obj(&sample_station()).contains_key("started_at"));
}

#[test]
fn station_skips_none_completed_at() {
    assert!(!obj(&sample_station()).contains_key("completed_at"));
}

#[test]
fn station_always_emits_status() {
    assert_eq!(obj(&sample_station())["status"], "active");
}

#[test]
fn station_always_emits_phase() {
    assert_eq!(obj(&sample_station())["phase"], "manufacture");
}

#[test]
fn station_default_status_when_omitted() {
    // status has #[serde(default)]; phase + station are required.
    let s: Station =
        serde_json::from_str(r#"{"station":"x","phase":"spec"}"#).expect("de");
    assert_eq!(s.status, Status::Pending);
}

#[test]
fn station_with_checkpoint_roundtrips() {
    let station = Station {
        checkpoint: Some(Checkpoint {
            kind: CheckpointKind::Ask,
            entered_at: None,
            outcome: None,
        }),
        ..sample_station()
    };
    let back = json_round(&station);
    assert_eq!(back.checkpoint.unwrap().kind, CheckpointKind::Ask);
}

#[test]
fn station_full_roundtrips() {
    let station = Station {
        station: "frame".into(),
        status: Status::Completed,
        phase: StationPhase::Checkpoint,
            elaborated: false,
        checkpoint: Some(Checkpoint {
            kind: CheckpointKind::Auto,
            entered_at: Some("t1".into()),
            outcome: Some(CheckpointOutcome::Advanced),
        }),
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: Some("t0".into()),
        completed_at: Some("t2".into()),
    };
    let back = json_round(&station);
    assert_eq!(back.status, Status::Completed);
    assert_eq!(back.phase, StationPhase::Checkpoint);
    assert_eq!(
        back.checkpoint.as_ref().unwrap().outcome,
        Some(CheckpointOutcome::Advanced)
    );
    assert_eq!(back.completed_at.as_deref(), Some("t2"));
}

#[test]
fn station_every_phase_roundtrips_in_struct() {
    for (phase, _) in ALL_PHASES {
        let station = Station {
            phase,
            ..sample_station()
        };
        assert_eq!(json_round(&station).phase, phase);
    }
}

#[test]
fn station_required_fields_are_station_and_phase() {
    let s = schema_value!(Station);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(required.contains(&"station"));
    assert!(required.contains(&"phase"));
    // status defaults, so it is not required.
    assert!(!required.contains(&"status"));
}

#[test]
fn station_missing_phase_is_an_error() {
    assert!(serde_json::from_str::<Station>(r#"{"station":"x"}"#).is_err());
}

#[test]
fn station_missing_station_is_an_error() {
    assert!(serde_json::from_str::<Station>(r#"{"phase":"spec"}"#).is_err());
}

#[test]
fn station_schema_references_definitions() {
    let s = schema_value!(Station);
    let defs = s["definitions"].as_object().unwrap();
    assert!(defs.contains_key("Status"));
    assert!(defs.contains_key("StationPhase"));
    assert!(defs.contains_key("Checkpoint"));
}

#[test]
fn station_yaml_roundtrips() {
    let station = Station {
        station: "audit".into(),
        status: Status::Blocked,
        phase: StationPhase::Audit,
            elaborated: false,
        checkpoint: None,
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: Some("t".into()),
        completed_at: None,
    };
    let back = yaml_round(&station);
    assert_eq!(back.status, Status::Blocked);
    assert_eq!(back.phase, StationPhase::Audit);
}

// ===========================================================================
// Feedback
// ===========================================================================

fn sample_feedback() -> Feedback {
    Feedback {
        id: "fb-1".into(),
        run: "r".into(),
        station: "prove".into(),
        status: FeedbackStatus::Pending,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: None,
        body: String::new(),
        created_at: None,
    }
}

#[test]
fn feedback_minimal_roundtrips() {
    let back = json_round(&sample_feedback());
    assert_eq!(back.id, "fb-1");
    assert_eq!(back.run, "r");
    assert_eq!(back.station, "prove");
    assert_eq!(back.status, FeedbackStatus::Pending);
    assert!(back.severity.is_none());
}

#[test]
fn feedback_skips_none_severity() {
    assert!(!obj(&sample_feedback()).contains_key("severity"));
}

#[test]
fn feedback_skips_none_created_at() {
    assert!(!obj(&sample_feedback()).contains_key("created_at"));
}

#[test]
fn feedback_always_emits_body_even_when_empty() {
    // body has #[serde(default)] but no skip rule.
    assert_eq!(obj(&sample_feedback())["body"], "");
}

#[test]
fn feedback_always_emits_status() {
    assert_eq!(obj(&sample_feedback())["status"], "pending");
}

#[test]
fn feedback_emits_severity_when_classified() {
    let fb = Feedback {
        severity: Some(FeedbackSeverity::Blocker),
        ..sample_feedback()
    };
    assert_eq!(obj(&fb)["severity"], "blocker");
}

#[test]
fn feedback_classified_roundtrips() {
    let fb = Feedback {
        id: "fb-9".into(),
        run: "run-x".into(),
        station: "build".into(),
        status: FeedbackStatus::Escalated,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: Some(FeedbackSeverity::High),
        body: "needs work".into(),
        created_at: Some("2026-05-30T00:00:00Z".into()),
    };
    let back = json_round(&fb);
    assert_eq!(back.status, FeedbackStatus::Escalated);
    assert_eq!(back.severity, Some(FeedbackSeverity::High));
    assert_eq!(back.body, "needs work");
    assert_eq!(back.created_at.as_deref(), Some("2026-05-30T00:00:00Z"));
}

#[test]
fn feedback_every_status_roundtrips_in_struct() {
    for (status, _) in ALL_FEEDBACK_STATUSES {
        let fb = Feedback {
            status,
            ..sample_feedback()
        };
        assert_eq!(json_round(&fb).status, status);
    }
}

#[test]
fn feedback_every_severity_roundtrips_in_struct() {
    for (sev, _) in ALL_SEVERITIES {
        let fb = Feedback {
            severity: Some(sev),
            ..sample_feedback()
        };
        assert_eq!(json_round(&fb).severity, Some(sev));
    }
}

#[test]
fn feedback_required_fields_in_schema() {
    let s = schema_value!(Feedback);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for key in ["id", "run", "station", "status"] {
        assert!(required.contains(&key), "must require {key}");
    }
    // body, severity, created_at default/skip -> not required.
    assert!(!required.contains(&"body"));
    assert!(!required.contains(&"severity"));
}

#[test]
fn feedback_missing_status_is_an_error() {
    let v = json!({ "id": "x", "run": "r", "station": "s" });
    assert!(serde_json::from_value::<Feedback>(v).is_err());
}

#[test]
fn feedback_missing_id_is_an_error() {
    let v = json!({ "run": "r", "station": "s", "status": "pending" });
    assert!(serde_json::from_value::<Feedback>(v).is_err());
}

#[test]
fn feedback_body_defaults_when_omitted() {
    let v = json!({ "id": "x", "run": "r", "station": "s", "status": "pending" });
    let fb: Feedback = serde_json::from_value(v).expect("de");
    assert_eq!(fb.body, "");
}

#[test]
fn feedback_schema_references_status_and_severity() {
    let s = schema_value!(Feedback);
    let defs = s["definitions"].as_object().unwrap();
    assert!(defs.contains_key("FeedbackStatus"));
    assert!(defs.contains_key("FeedbackSeverity"));
}

#[test]
fn feedback_yaml_roundtrips() {
    let fb = Feedback {
        id: "fb".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::Answered,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: Some(FeedbackSeverity::Low),
        body: "reply".into(),
        created_at: None,
    };
    let back = yaml_round(&fb);
    assert_eq!(back.status, FeedbackStatus::Answered);
    assert_eq!(back.severity, Some(FeedbackSeverity::Low));
}

#[test]
fn feedback_preserves_multiline_body() {
    let fb = Feedback {
        body: "issue:\n- one\n- two\n".into(),
        ..sample_feedback()
    };
    assert_eq!(json_round(&fb).body, "issue:\n- one\n- two\n");
}

// ===========================================================================
// Cross-cutting: determinism, idempotency, nesting
// ===========================================================================

#[test]
fn json_serialization_is_deterministic_for_run_frontmatter() {
    let fm = RunFrontmatter {
        title: Some("T".into()),
        factory: "f".into(),
        status: Status::Active,
        archived: Some(true),
        ..Default::default()
    };
    let a = serde_json::to_string(&fm).expect("ser");
    let b = serde_json::to_string(&fm).expect("ser");
    assert_eq!(a, b, "serialization must be byte-stable");
}

#[test]
fn double_roundtrip_is_idempotent_for_unit_frontmatter() {
    let fm = UnitFrontmatter {
        unit_type: "feature".into(),
        status: Status::InProgress,
        depends_on: vec!["a".into(), "b".into()],
        worker: "builder".into(),
        inputs: vec!["i.md".into()],
        outputs: vec!["o.md".into()],
        ..Default::default()
    };
    let once = json_round(&fm);
    let twice = json_round(&once);
    assert_eq!(
        serde_json::to_string(&once).unwrap(),
        serde_json::to_string(&twice).unwrap()
    );
}

#[test]
fn double_roundtrip_is_idempotent_for_station() {
    let station = Station {
        station: "build".into(),
        status: Status::Active,
        phase: StationPhase::Reflect,
            elaborated: false,
        checkpoint: Some(Checkpoint {
            kind: CheckpointKind::External,
            entered_at: Some("t".into()),
            outcome: Some(CheckpointOutcome::Awaiting),
        }),
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: Some("t0".into()),
        completed_at: None,
    };
    let once = json_round(&station);
    let twice = json_round(&once);
    assert_eq!(
        serde_json::to_string(&once).unwrap(),
        serde_json::to_string(&twice).unwrap()
    );
}

#[test]
fn json_to_yaml_to_json_preserves_run() {
    let run = sample_run();
    let yaml = serde_yaml::to_string(&run).expect("yaml");
    let from_yaml: Run = serde_yaml::from_str(&yaml).expect("de yaml");
    let json = serde_json::to_string(&from_yaml).expect("json");
    let back: Run = serde_json::from_str(&json).expect("de json");
    assert_eq!(back.slug, run.slug);
    assert_eq!(back.frontmatter.factory, run.frontmatter.factory);
}

#[test]
fn every_enum_schema_is_valid_draft07() {
    // Each enum schema must declare the draft-07 meta-schema.
    let draft = "http://json-schema.org/draft-07/schema#";
    assert_eq!(schema_value!(Status)["$schema"], draft);
    assert_eq!(schema_value!(StationPhase)["$schema"], draft);
    assert_eq!(schema_value!(CheckpointKind)["$schema"], draft);
    assert_eq!(schema_value!(CheckpointOutcome)["$schema"], draft);
    assert_eq!(schema_value!(PassBeat)["$schema"], draft);
    assert_eq!(schema_value!(FeedbackSeverity)["$schema"], draft);
    assert_eq!(schema_value!(FeedbackStatus)["$schema"], draft);
}

#[test]
fn every_struct_schema_declares_object_type() {
    assert_eq!(schema_value!(RunGit)["type"], "object");
    assert_eq!(schema_value!(RunFrontmatter)["type"], "object");
    assert_eq!(schema_value!(Run)["type"], "object");
    assert_eq!(schema_value!(UnitFrontmatter)["type"], "object");
    assert_eq!(schema_value!(Unit)["type"], "object");
    assert_eq!(schema_value!(Pass)["type"], "object");
    assert_eq!(schema_value!(Worker)["type"], "object");
    assert_eq!(schema_value!(Explorer)["type"], "object");
    assert_eq!(schema_value!(Reviewer)["type"], "object");
    assert_eq!(schema_value!(Checkpoint)["type"], "object");
    assert_eq!(schema_value!(Station)["type"], "object");
    assert_eq!(schema_value!(Feedback)["type"], "object");
}

#[test]
fn every_struct_schema_has_title() {
    assert_eq!(schema_value!(RunGit)["title"], "RunGit");
    assert_eq!(schema_value!(RunFrontmatter)["title"], "RunFrontmatter");
    assert_eq!(schema_value!(Run)["title"], "Run");
    assert_eq!(schema_value!(UnitFrontmatter)["title"], "UnitFrontmatter");
    assert_eq!(schema_value!(Unit)["title"], "Unit");
    assert_eq!(schema_value!(Pass)["title"], "Pass");
    assert_eq!(schema_value!(Worker)["title"], "Worker");
    assert_eq!(schema_value!(Explorer)["title"], "Explorer");
    assert_eq!(schema_value!(Reviewer)["title"], "Reviewer");
    assert_eq!(schema_value!(Checkpoint)["title"], "Checkpoint");
    assert_eq!(schema_value!(Station)["title"], "Station");
    assert_eq!(schema_value!(Feedback)["title"], "Feedback");
}

#[test]
fn every_enum_schema_has_title() {
    assert_eq!(schema_value!(Status)["title"], "Status");
    assert_eq!(schema_value!(StationPhase)["title"], "StationPhase");
    assert_eq!(schema_value!(CheckpointKind)["title"], "CheckpointKind");
    assert_eq!(
        schema_value!(CheckpointOutcome)["title"],
        "CheckpointOutcome"
    );
    assert_eq!(schema_value!(PassBeat)["title"], "PassBeat");
    assert_eq!(schema_value!(FeedbackSeverity)["title"], "FeedbackSeverity");
    assert_eq!(schema_value!(FeedbackStatus)["title"], "FeedbackStatus");
}

#[test]
fn every_enum_schema_carries_a_description() {
    // The doc-comment on each enum becomes the schema description; missing
    // docs would silently drop tooltips in generated clients.
    assert!(schema_value!(Status)["description"].is_string());
    assert!(schema_value!(StationPhase)["description"].is_string());
    assert!(schema_value!(CheckpointKind)["description"].is_string());
    assert!(schema_value!(CheckpointOutcome)["description"].is_string());
    assert!(schema_value!(PassBeat)["description"].is_string());
    assert!(schema_value!(FeedbackSeverity)["description"].is_string());
    assert!(schema_value!(FeedbackStatus)["description"].is_string());
}

#[test]
fn every_enum_schema_variant_carries_a_description() {
    // Per-variant doc-comments become per-variant descriptions in the oneOf.
    for ty in [
        schema_value!(Status),
        schema_value!(StationPhase),
        schema_value!(CheckpointKind),
        schema_value!(CheckpointOutcome),
        schema_value!(PassBeat),
        schema_value!(FeedbackSeverity),
        schema_value!(FeedbackStatus),
    ] {
        for variant in ty["oneOf"].as_array().expect("oneOf") {
            assert!(
                variant["description"].is_string(),
                "each variant must carry a description"
            );
            // Exactly one token per variant.
            assert_eq!(variant["enum"].as_array().unwrap().len(), 1);
            assert_eq!(variant["type"], "string");
        }
    }
}

// ===========================================================================
// Cross-type wire-token contract
// ===========================================================================

#[test]
fn status_and_feedback_status_share_pending_token() {
    // Shared UI label across both lifecycles.
    assert_eq!(
        json_token(&Status::Pending),
        json_token(&FeedbackStatus::Pending)
    );
}

#[test]
fn checkpoint_kind_await_and_outcome_awaiting_are_distinct_tokens() {
    // The gate kind ("await") differs from the in-flight outcome ("awaiting").
    assert_ne!(
        json_token(&CheckpointKind::Await),
        json_token(&CheckpointOutcome::Awaiting)
    );
}

// ===========================================================================
// Numeric & string boundaries
// ===========================================================================

#[test]
fn unit_frontmatter_empty_strings_roundtrip() {
    let fm = UnitFrontmatter {
        unit_type: String::new(),
        worker: String::new(),
        name: Some(String::new()),
        ..Default::default()
    };
    let back = json_round(&fm);
    assert_eq!(back.unit_type, "");
    assert_eq!(back.worker, "");
    // Some("") is preserved distinctly from None.
    assert_eq!(back.name.as_deref(), Some(""));
}

#[test]
fn unit_frontmatter_large_depends_on_list_roundtrips() {
    let deps: Vec<String> = (0..200).map(|i| format!("dep-{i}")).collect();
    let fm = UnitFrontmatter {
        depends_on: deps.clone(),
        ..Default::default()
    };
    let back = json_round(&fm);
    assert_eq!(back.depends_on.len(), 200);
    assert_eq!(back.depends_on, deps);
}

#[test]
fn unit_frontmatter_duplicate_depends_on_preserved() {
    // Domain layer does not dedupe; the wire form must preserve duplicates.
    let fm = UnitFrontmatter {
        depends_on: vec!["a".into(), "a".into(), "b".into()],
        ..Default::default()
    };
    assert_eq!(json_round(&fm).depends_on, vec!["a", "a", "b"]);
}

#[test]
fn pass_index_one_roundtrips() {
    let pass = Pass {
        index: 1,
        unit: "u".into(),
        beat: PassBeat::Make,
    };
    assert_eq!(json_round(&pass).index, 1);
}

#[test]
fn unit_iterations_roundtrip_json() {
    use darkrun_core::domain::{IterationResult, UnitIteration};
    let fm = UnitFrontmatter {
        iterations: vec![UnitIteration {
            worker: "make".into(),
            result: Some(IterationResult::Advance),
            note: Some("handoff".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let back = json_round(&fm);
    assert_eq!(back.iterations.len(), 1);
    assert_eq!(back.iterations[0].note.as_deref(), Some("handoff"));
}

#[test]
fn run_frontmatter_unicode_factory_roundtrips() {
    let fm = RunFrontmatter {
        factory: "工厂-software".into(),
        ..Default::default()
    };
    assert_eq!(json_round(&fm).factory, "工厂-software");
}

#[test]
fn feedback_unicode_id_and_body_roundtrip() {
    let fb = Feedback {
        id: "fb-✓".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::Pending,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: None,
        body: "找到一个 bug 🐛".into(),
        created_at: None,
    };
    let back = json_round(&fb);
    assert_eq!(back.id, "fb-✓");
    assert_eq!(back.body, "找到一个 bug 🐛");
}

#[test]
fn worker_name_with_hyphens_roundtrips() {
    let w = Worker {
        name: "fix-worker-loop".into(),
        model: None,
        terminal: false,
    };
    assert_eq!(json_round(&w).name, "fix-worker-loop");
}

// ===========================================================================
// Option None vs absent symmetry
// ===========================================================================

#[test]
fn run_frontmatter_absent_optionals_deserialize_as_none() {
    // A serialized minimal frontmatter must round-trip to all-None optionals.
    let fm = minimal_run_fm();
    let s = serde_json::to_string(&fm).expect("ser");
    let back: RunFrontmatter = serde_json::from_str(&s).expect("de");
    assert!(back.title.is_none());
    assert!(back.archived.is_none());
    assert!(back.started_at.is_none());
    assert!(back.completed_at.is_none());
    assert!(back.git.is_none());
}

#[test]
fn checkpoint_explicit_null_outcome_deserializes_as_none() {
    let cp: Checkpoint =
        serde_json::from_str(r#"{"kind":"auto","outcome":null}"#).expect("de");
    assert!(cp.outcome.is_none());
}

#[test]
fn run_frontmatter_explicit_null_git_deserializes_as_none() {
    let fm: RunFrontmatter =
        serde_json::from_str(r#"{"factory":"f","git":null}"#).expect("de");
    assert!(fm.git.is_none());
}

#[test]
fn unit_frontmatter_explicit_null_optionals_are_none() {
    let fm: UnitFrontmatter =
        serde_json::from_str(r#"{"name":null,"model":null,"station":null}"#).expect("de");
    assert!(fm.name.is_none());
    assert!(fm.model.is_none());
    assert!(fm.station.is_none());
}

// ===========================================================================
// Full-document nesting roundtrips
// ===========================================================================

#[test]
fn run_with_full_git_policy_yaml_roundtrips() {
    let run = Run {
        slug: "ship".into(),
        frontmatter: RunFrontmatter {
            title: Some("Ship".into()),
            factory: "software".into(),
            mode: Mode::Solo,
            active_station: "frame".into(),
            status: Status::Active,
            surface: None,
            archived: Some(false),
            started_at: Some("2026-05-30T00:00:00Z".into()),
            completed_at: None,
            git: Some(RunGit {
                change_strategy: "worktree-per-unit".into(),
                auto_merge: true,
                auto_squash: true,
            }),
            seal: None,
            external_refs: Default::default(),
            created_by: None,
            composite: None,
            sync: vec![],
            composite_state: Default::default(),
        },
        title: "Ship".into(),
        body: "# Ship\n".into(),
    };
    let back = yaml_round(&run);
    assert_eq!(back.frontmatter.mode, Mode::Solo);
    let git = back.frontmatter.git.expect("git");
    assert!(git.auto_merge && git.auto_squash);
}

#[test]
fn unit_with_full_frontmatter_yaml_roundtrips() {
    let unit = Unit {
        slug: "auth".into(),
        frontmatter: UnitFrontmatter {
            name: Some("Auth".into()),
            unit_type: "feature".into(),
            status: Status::InProgress,
            depends_on: vec!["db".into()],
            worker: "builder".into(),
            model: Some("opus".into()),
            station: Some("build".into()),
            revise: false,
            inputs: vec!["spec.md".into()],
            outputs: vec!["auth.rs".into()],
            started_at: Some("t".into()),
            completed_at: None,
            ..Default::default()
        },
        title: "Auth".into(),
        body: "impl".into(),
    };
    let back = yaml_round(&unit);
    assert_eq!(back.station(), "build");
    assert_eq!(back.status(), Status::InProgress);
}

#[test]
fn station_with_each_checkpoint_kind_roundtrips() {
    for (kind, _) in ALL_KINDS {
        let station = Station {
            station: "s".into(),
            status: Status::Active,
            phase: StationPhase::Checkpoint,
            elaborated: false,
            checkpoint: Some(Checkpoint {
                kind,
                entered_at: None,
                outcome: None,
            }),
            branch: None,
            pr_ref: None,
            pr_status: None,
            pr_ready_at: None,
            pr_merged_at: None,
            verifier_nonce: None,
            started_at: None,
            completed_at: None,
        };
        assert_eq!(json_round(&station).checkpoint.unwrap().kind, kind);
    }
}

// ===========================================================================
// Schema structural invariants
// ===========================================================================

#[test]
fn nested_struct_schemas_reference_inner_definitions() {
    // RunFrontmatter embeds RunGit and Status.
    let s = schema_value!(RunFrontmatter);
    let defs = s["definitions"].as_object().expect("definitions");
    assert!(defs.contains_key("RunGit"));
    assert!(defs.contains_key("Status"));
}

#[test]
fn unit_frontmatter_schema_references_status_definition() {
    let s = schema_value!(UnitFrontmatter);
    assert!(s["definitions"]
        .as_object()
        .unwrap()
        .contains_key("Status"));
}

#[test]
fn unit_schema_references_unit_frontmatter_definition() {
    let s = schema_value!(Unit);
    assert!(s["definitions"]
        .as_object()
        .unwrap()
        .contains_key("UnitFrontmatter"));
}

#[test]
fn pass_schema_references_pass_beat_definition() {
    let s = schema_value!(Pass);
    assert!(s["definitions"]
        .as_object()
        .unwrap()
        .contains_key("PassBeat"));
}

#[test]
fn enum_schemas_have_no_properties_block() {
    // Unit enums are string oneOf; they must not carry an object properties map.
    assert!(schema_value!(Status).get("properties").is_none());
    assert!(schema_value!(StationPhase).get("properties").is_none());
}

#[test]
fn struct_schemas_have_no_oneof_block() {
    // Plain structs serialize as objects, never as a oneOf union.
    assert!(schema_value!(Worker).get("oneOf").is_none());
    assert!(schema_value!(Station).get("oneOf").is_none());
    assert!(schema_value!(Feedback).get("oneOf").is_none());
}

#[test]
fn run_git_bool_properties_typed_as_boolean() {
    let s = schema_value!(RunGit);
    let props = s["properties"].as_object().unwrap();
    assert_eq!(props["auto_merge"]["type"], "boolean");
    assert_eq!(props["auto_squash"]["type"], "boolean");
    assert_eq!(props["change_strategy"]["type"], "string");
}

#[test]
fn unit_frontmatter_iterations_typed_as_array() {
    let s = schema_value!(UnitFrontmatter);
    let iters = &s["properties"]["iterations"];
    assert_eq!(iters["type"], "array");
}

#[test]
fn unit_frontmatter_depends_on_typed_as_array_of_strings() {
    let s = schema_value!(UnitFrontmatter);
    let deps = &s["properties"]["depends_on"];
    assert_eq!(deps["type"], "array");
    assert_eq!(deps["items"]["type"], "string");
}

#[test]
fn checkpoint_optional_fields_admit_null_in_schema() {
    let s = schema_value!(Checkpoint);
    // entered_at is Option<String> -> ["string","null"].
    let entered = &s["properties"]["entered_at"]["type"];
    let arr = entered.as_array().expect("type union array");
    assert!(arr.contains(&Value::from("string")));
    assert!(arr.contains(&Value::from("null")));
}

// ===========================================================================
// Variant-count guards (catch accidental add/remove)
// ===========================================================================

#[test]
fn status_has_exactly_five_variants_in_schema() {
    assert_eq!(schema_value!(Status)["oneOf"].as_array().unwrap().len(), 5);
}

#[test]
fn station_phase_has_exactly_seven_variants_in_schema() {
    assert_eq!(
        schema_value!(StationPhase)["oneOf"].as_array().unwrap().len(),
        7
    );
}

#[test]
fn checkpoint_kind_has_exactly_four_variants_in_schema() {
    assert_eq!(
        schema_value!(CheckpointKind)["oneOf"].as_array().unwrap().len(),
        4
    );
}

#[test]
fn checkpoint_outcome_has_exactly_four_variants_in_schema() {
    assert_eq!(
        schema_value!(CheckpointOutcome)["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn pass_beat_has_exactly_three_variants_in_schema() {
    assert_eq!(
        schema_value!(PassBeat)["oneOf"].as_array().unwrap().len(),
        3
    );
}

#[test]
fn feedback_severity_has_exactly_four_variants_in_schema() {
    assert_eq!(
        schema_value!(FeedbackSeverity)["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn feedback_status_has_exactly_eight_variants_in_schema() {
    assert_eq!(
        schema_value!(FeedbackStatus)["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
}

// ===========================================================================
// Pretty-print stability and whitespace tolerance
// ===========================================================================

#[test]
fn pretty_json_roundtrips_for_station() {
    let station = sample_station();
    let pretty = serde_json::to_string_pretty(&station).expect("pretty");
    let back: Station = serde_json::from_str(&pretty).expect("de pretty");
    assert_eq!(back.phase, StationPhase::Manufacture);
}

#[test]
fn whitespace_padded_json_deserializes_for_worker() {
    let w: Worker =
        serde_json::from_str("  {  \"name\" : \"x\" , \"terminal\" : true }  ").expect("de");
    assert_eq!(w.name, "x");
    assert!(w.terminal);
}

#[test]
fn feedback_field_order_does_not_matter() {
    let v = r#"{"status":"closed","station":"s","id":"i","run":"r","body":"b"}"#;
    let fb: Feedback = serde_json::from_str(v).expect("de");
    assert_eq!(fb.id, "i");
    assert_eq!(fb.status, FeedbackStatus::Closed);
    assert_eq!(fb.body, "b");
}

#[test]
fn run_frontmatter_extra_then_known_field_order_parses() {
    let fm: RunFrontmatter =
        serde_json::from_str(r#"{"extra":1,"factory":"f","mode":"team"}"#).expect("de");
    assert_eq!(fm.factory, "f");
    assert_eq!(fm.mode, Mode::Team);
}

#[test]
fn feedback_tolerates_unknown_yaml_field() {
    let fb: Feedback = serde_yaml::from_str(
        "id: i\nrun: r\nstation: s\nstatus: pending\nlegacy: drop\n",
    )
    .expect("de");
    assert_eq!(fb.id, "i");
}

#[test]
fn checkpoint_tolerates_unknown_field() {
    let cp: Checkpoint =
        serde_json::from_str(r#"{"kind":"auto","legacy":"x"}"#).expect("de");
    assert_eq!(cp.kind, CheckpointKind::Auto);
}

#[test]
fn station_tolerates_unknown_field() {
    let s: Station =
        serde_json::from_str(r#"{"station":"x","phase":"spec","gate":"old"}"#).expect("de");
    assert_eq!(s.phase, StationPhase::Spec);
}

#[test]
fn every_struct_schema_carries_a_description() {
    // Each struct's doc-comment must survive into the schema for tooling.
    assert!(schema_value!(RunGit)["description"].is_string());
    assert!(schema_value!(RunFrontmatter)["description"].is_string());
    assert!(schema_value!(Run)["description"].is_string());
    assert!(schema_value!(UnitFrontmatter)["description"].is_string());
    assert!(schema_value!(Unit)["description"].is_string());
    assert!(schema_value!(Pass)["description"].is_string());
    assert!(schema_value!(Worker)["description"].is_string());
    assert!(schema_value!(Explorer)["description"].is_string());
    assert!(schema_value!(Reviewer)["description"].is_string());
    assert!(schema_value!(Checkpoint)["description"].is_string());
    assert!(schema_value!(Station)["description"].is_string());
    assert!(schema_value!(Feedback)["description"].is_string());
}

#[test]
fn worker_terminal_field_typed_as_boolean_in_schema() {
    let s = schema_value!(Worker);
    assert_eq!(s["properties"]["terminal"]["type"], "boolean");
}

#[test]
fn pass_index_typed_as_uint32_in_schema() {
    let s = schema_value!(Pass);
    assert_eq!(s["properties"]["index"]["type"], "integer");
    assert_eq!(s["properties"]["index"]["format"], "uint32");
}

#[test]
fn run_frontmatter_property_descriptions_present() {
    // Field doc-comments map to per-property descriptions.
    let s = schema_value!(RunFrontmatter);
    assert!(s["properties"]["factory"]["description"].is_string());
}

#[test]
fn status_token_set_matches_const_table() {
    // The schema's token set must exactly match the hand-maintained table the
    // rest of these tests rely on.
    use std::collections::BTreeSet;
    let schema_tokens: BTreeSet<String> = schema_value!(Status)["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["enum"][0].as_str().unwrap().to_string())
        .collect();
    let table: BTreeSet<String> = ["pending", "active", "in_progress", "completed", "blocked"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(schema_tokens, table);
}

#[test]
fn feedback_status_token_set_matches_const_table() {
    use std::collections::BTreeSet;
    let schema_tokens: BTreeSet<String> = schema_value!(FeedbackStatus)["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["enum"][0].as_str().unwrap().to_string())
        .collect();
    let table: BTreeSet<String> = ALL_FEEDBACK_STATUSES
        .iter()
        .map(|(_, t)| t.to_string())
        .collect();
    assert_eq!(schema_tokens, table);
}

// ===========================================================================
// Position — the fixed FSSBPH flow invariant
// ===========================================================================

#[test]
fn flow_is_the_six_fssbph_positions_in_order() {
    let dirs: Vec<&str> = Position::FLOW.iter().map(|p| p.dir()).collect();
    assert_eq!(
        dirs,
        vec!["frame", "specify", "shape", "build", "prove", "harden"]
    );
}

#[test]
fn position_parse_round_trips_and_rejects_unknown() {
    for p in Position::FLOW {
        assert_eq!(Position::parse(p.dir()), Some(p));
        assert_eq!(p.index(), Position::FLOW.iter().position(|&q| q == p).unwrap());
    }
    assert_eq!(Position::parse("operations"), None);
}

#[test]
fn position_serializes_snake_case() {
    assert_eq!(serde_json::to_string(&Position::Frame).unwrap(), "\"frame\"");
    assert_eq!(serde_json::to_string(&Position::Harden).unwrap(), "\"harden\"");
}
