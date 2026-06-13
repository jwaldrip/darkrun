//! Comprehensive integration tests for the `darkrun-api` session payload wire
//! contract — the discriminated `SessionPayload` union, its five variants
//! (review / question / direction / picker / view), the load-bearing
//! `ReviewSessionPayload` and all of its nested types, the run-phase taxonomy,
//! gate kinds, approve actions, drift, seal status, and every option/skip field
//! on the wire.
//!
//! Drives the crate's PUBLIC API only. Every test exercises real serde
//! behavior: roundtrip fidelity, the `session_type` / `mode` discriminators,
//! `skip_serializing_if` omission, snake/kebab case rename rules, default
//! materialization, boundary numerics, idempotency and determinism.

use std::collections::BTreeMap;

use darkrun_api::common::{GateType, ReviewAnnotations, SessionStatus, SessionType};
use darkrun_api::direction::{DirectionSelectRequest, PickerSelectRequest};
use darkrun_api::session::{
    ApproveAction, ApproveActionKind, DirectionAnnotations, DirectionArchetype, DirectionPin,
    DirectionSessionPayload, DiscoveredReviewUrl,     KnowledgeFile, MilestoneStatus, OutputArtifact, OutputArtifactType, PendingDecision,
    PickerKind, PickerOption, PickerSelection, PickerSessionPayload, PreviousReviewSnapshot,
    ProgressMilestone, QuestionAnswer, QuestionOption, QuestionSessionPayload,
    ReviewSessionPayload, RunCurrentState, RunPhase, SealStatus, SessionPayload,
    StationArtifact, StationStateInfo, UnitOutputPreview, UnitOutputType, ViewMode,
    ViewScope, ViewSessionPayload, ViewStatus,
};
use darkrun_api::session::{
    ProofSessionPayload, ViewArtifact, ViewArtifactKind, VisualReviewAnnotations, VisualReviewPin,
    VisualReviewSessionPayload,
};
use darkrun_api::{AuditResult, BenchProof, Proof, Surface, WebProof};
use serde_json::{json, Value};

// -----------------------------------------------------------------------------
// helpers
// -----------------------------------------------------------------------------

/// Serialize -> deserialize -> serialize again. Returns the second JSON. A
/// faithful roundtrip means `json1 == json2`.
fn round<T>(value: &T) -> (Value, Value)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let json1 = serde_json::to_value(value).expect("serialize");
    let back: T = serde_json::from_value(json1.clone()).expect("deserialize");
    let json2 = serde_json::to_value(&back).expect("reserialize");
    (json1, json2)
}

/// Assert a value survives a serde roundtrip with byte-identical JSON.
fn assert_stable<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let (a, b) = round(value);
    assert_eq!(a, b, "roundtrip changed the JSON shape");
}

fn review(p: ReviewSessionPayload) -> SessionPayload {
    SessionPayload::Review(p)
}

// -----------------------------------------------------------------------------
// SessionPayload discriminator — the tagged union
// -----------------------------------------------------------------------------

#[test]
fn review_variant_tags_session_type() {
    let p = review(ReviewSessionPayload {
        session_id: "rv".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "review");
}

#[test]
fn question_variant_tags_session_type() {
    let p = SessionPayload::Question(QuestionSessionPayload {
        session_id: "q".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "question");
}

#[test]
fn direction_variant_tags_session_type() {
    let p = SessionPayload::Direction(DirectionSessionPayload {
        session_id: "d".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "direction");
}

#[test]
fn picker_variant_tags_session_type() {
    let p = SessionPayload::Picker(sample_picker("p"));
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "picker");
}

#[test]
fn view_variant_tags_session_type() {
    let p = SessionPayload::View(sample_view("v"));
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "view");
}

#[test]
fn session_type_accessor_matches_tag_for_all_variants() {
    let cases: Vec<(SessionPayload, &str)> = vec![
        (review(ReviewSessionPayload::default()), "review"),
        (
            SessionPayload::Question(QuestionSessionPayload::default()),
            "question",
        ),
        (
            SessionPayload::Direction(DirectionSessionPayload::default()),
            "direction",
        ),
        (SessionPayload::Picker(sample_picker("x")), "picker"),
        (SessionPayload::View(sample_view("x")), "view"),
    ];
    for (payload, tag) in cases {
        assert_eq!(payload.session_type(), tag);
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["session_type"], tag);
    }
}

#[test]
fn session_id_accessor_reads_every_variant() {
    assert_eq!(
        review(ReviewSessionPayload {
            session_id: "a".into(),
            ..Default::default()
        })
        .session_id(),
        "a"
    );
    assert_eq!(
        SessionPayload::Question(QuestionSessionPayload {
            session_id: "b".into(),
            ..Default::default()
        })
        .session_id(),
        "b"
    );
    assert_eq!(
        SessionPayload::Direction(DirectionSessionPayload {
            session_id: "c".into(),
            ..Default::default()
        })
        .session_id(),
        "c"
    );
    assert_eq!(SessionPayload::Picker(sample_picker("d")).session_id(), "d");
    assert_eq!(SessionPayload::View(sample_view("e")).session_id(), "e");
}

#[test]
fn unknown_session_type_is_rejected() {
    let json = json!({ "session_type": "telepathy", "session_id": "x" });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err());
}

#[test]
fn missing_session_type_is_rejected() {
    let json = json!({ "session_id": "x", "status": "pending" });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err());
}

#[test]
fn empty_object_is_rejected_as_session_payload() {
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json!({}));
    assert!(parsed.is_err());
}

#[test]
fn session_type_tag_is_case_sensitive() {
    let json = json!({ "session_type": "Review", "session_id": "x", "status": "pending" });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err(), "Capitalized tag must not parse");
}

#[test]
fn session_type_tag_rejects_numeric() {
    let json = json!({ "session_type": 1, "session_id": "x" });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err());
}

#[test]
fn review_payload_carries_inner_fields_flattened_with_tag() {
    // serde tag = "session_type" means the inner struct fields sit alongside
    // the discriminator, not nested under a key.
    let p = review(ReviewSessionPayload {
        session_id: "s-1".into(),
        run_slug: Some("my-run".into()),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_id"], "s-1");
    assert_eq!(json["run_slug"], "my-run");
    assert!(json.get("Review").is_none(), "must not nest under variant name");
}

#[test]
fn all_variants_roundtrip_preserving_tag() {
    let payloads = vec![
        review(ReviewSessionPayload {
            session_id: "r".into(),
            ..Default::default()
        }),
        SessionPayload::Question(QuestionSessionPayload {
            session_id: "q".into(),
            ..Default::default()
        }),
        SessionPayload::Direction(DirectionSessionPayload {
            session_id: "d".into(),
            ..Default::default()
        }),
        SessionPayload::Picker(sample_picker("p")),
        SessionPayload::View(sample_view("v")),
    ];
    for p in payloads {
        assert_stable(&p);
    }
}

// -----------------------------------------------------------------------------
// sample constructors (picker/view have no Default)
// -----------------------------------------------------------------------------

fn sample_picker(id: &str) -> PickerSessionPayload {
    PickerSessionPayload {
        session_id: id.into(),
        status: SessionStatus::Pending,
        run_slug: None,
        kind: PickerKind::Factory,
        title: "pick".into(),
        prompt: "which?".into(),
        options: vec![PickerOption {
            id: "software".into(),
            label: "Software".into(),
            description: None,
            secondary: None,
        }],
        selection: None,
    }
}

fn sample_view(id: &str) -> ViewSessionPayload {
    ViewSessionPayload {
        session_id: id.into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: None,
        station: None,
        artifact: None,
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    }
}

// -----------------------------------------------------------------------------
// RunPhase — all six phases
// -----------------------------------------------------------------------------

#[test]
fn run_phase_serializes_snake_case() {
    let cases = [
        (RunPhase::Spec, "spec"),
        (RunPhase::Review, "review"),
        (RunPhase::Manufacture, "manufacture"),
        (RunPhase::Audit, "audit"),
        (RunPhase::Reflect, "reflect"),
        (RunPhase::Checkpoint, "checkpoint"),
    ];
    for (phase, wire) in cases {
        assert_eq!(serde_json::to_value(phase).unwrap(), json!(wire));
    }
}

#[test]
fn run_phase_deserializes_each_variant() {
    let cases = [
        ("spec", RunPhase::Spec),
        ("review", RunPhase::Review),
        ("manufacture", RunPhase::Manufacture),
        ("audit", RunPhase::Audit),
        ("reflect", RunPhase::Reflect),
        ("checkpoint", RunPhase::Checkpoint),
    ];
    for (wire, phase) in cases {
        let parsed: RunPhase = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(parsed, phase);
    }
}

#[test]
fn run_phase_roundtrips_each() {
    for phase in [
        RunPhase::Spec,
        RunPhase::Review,
        RunPhase::Manufacture,
        RunPhase::Audit,
        RunPhase::Reflect,
        RunPhase::Checkpoint,
    ] {
        assert_stable(&phase);
    }
}

#[test]
fn run_phase_rejects_unknown() {
    let parsed: Result<RunPhase, _> = serde_json::from_value(json!("deploy"));
    assert!(parsed.is_err());
}

#[test]
fn run_phase_rejects_pascal_case() {
    let parsed: Result<RunPhase, _> = serde_json::from_value(json!("Spec"));
    assert!(parsed.is_err());
}

#[test]
fn run_phase_ordering_taxonomy_is_distinct() {
    // The six phases are distinct values; no two collapse on the wire.
    let mut seen = std::collections::HashSet::new();
    for phase in [
        RunPhase::Spec,
        RunPhase::Review,
        RunPhase::Manufacture,
        RunPhase::Audit,
        RunPhase::Reflect,
        RunPhase::Checkpoint,
    ] {
        let wire = serde_json::to_value(phase).unwrap();
        assert!(seen.insert(wire.as_str().unwrap().to_string()), "duplicate wire repr");
    }
    assert_eq!(seen.len(), 6);
}

#[test]
fn run_phase_in_current_state_each_phase() {
    for phase in [
        RunPhase::Spec,
        RunPhase::Review,
        RunPhase::Manufacture,
        RunPhase::Audit,
        RunPhase::Reflect,
        RunPhase::Checkpoint,
    ] {
        let st = RunCurrentState {
            factory: "software".into(),
            station: "frame".into(),
            phase: Some(phase),
            ..Default::default()
        };
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["phase"], serde_json::to_value(phase).unwrap());
        assert_stable(&st);
    }
}

// -----------------------------------------------------------------------------
// GateType — all four kinds
// -----------------------------------------------------------------------------

#[test]
fn gate_type_serializes_snake_case() {
    let cases = [
        (GateType::Auto, "auto"),
        (GateType::Ask, "ask"),
        (GateType::External, "external"),
        (GateType::Await, "await"),
    ];
    for (gate, wire) in cases {
        assert_eq!(serde_json::to_value(gate).unwrap(), json!(wire));
    }
}

#[test]
fn gate_type_deserializes_each() {
    for (wire, gate) in [
        ("auto", GateType::Auto),
        ("ask", GateType::Ask),
        ("external", GateType::External),
        ("await", GateType::Await),
    ] {
        let parsed: GateType = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(parsed, gate);
    }
}

#[test]
fn gate_type_roundtrips_each() {
    for gate in [GateType::Auto, GateType::Ask, GateType::External, GateType::Await] {
        assert_stable(&gate);
    }
}

#[test]
fn gate_type_rejects_unknown() {
    let parsed: Result<GateType, _> = serde_json::from_value(json!("manual"));
    assert!(parsed.is_err());
}

#[test]
fn gate_type_in_review_payload_each_kind() {
    for gate in [GateType::Auto, GateType::Ask, GateType::External, GateType::Await] {
        let p = review(ReviewSessionPayload {
            session_id: "g".into(),
            gate_type: Some(gate),
            ..Default::default()
        });
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["gate_type"], serde_json::to_value(gate).unwrap());
        assert_stable(&p);
    }
}

// -----------------------------------------------------------------------------
// SessionType discriminator enum (the separate standalone enum)
// -----------------------------------------------------------------------------

#[test]
fn session_type_enum_snake_case() {
    for (variant, wire) in [
        (SessionType::Review, "review"),
        (SessionType::Question, "question"),
        (SessionType::Direction, "direction"),
        (SessionType::Picker, "picker"),
        (SessionType::View, "view"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let parsed: SessionType = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(parsed, variant);
    }
}

#[test]
fn session_type_enum_matches_payload_tags() {
    // The standalone SessionType wire strings line up with SessionPayload tags.
    let pairs: Vec<(SessionType, SessionPayload)> = vec![
        (SessionType::Review, review(ReviewSessionPayload::default())),
        (
            SessionType::Question,
            SessionPayload::Question(QuestionSessionPayload::default()),
        ),
        (
            SessionType::Direction,
            SessionPayload::Direction(DirectionSessionPayload::default()),
        ),
        (SessionType::Picker, SessionPayload::Picker(sample_picker("p"))),
        (SessionType::View, SessionPayload::View(sample_view("v"))),
    ];
    for (ty, payload) in pairs {
        let ty_wire = serde_json::to_value(ty).unwrap();
        assert_eq!(ty_wire.as_str().unwrap(), payload.session_type());
    }
}

#[test]
fn session_type_enum_rejects_unknown() {
    let parsed: Result<SessionType, _> = serde_json::from_value(json!("audit"));
    assert!(parsed.is_err());
}

// -----------------------------------------------------------------------------
// SessionStatus — including Default
// -----------------------------------------------------------------------------

#[test]
fn session_status_default_is_pending() {
    assert_eq!(SessionStatus::default(), SessionStatus::Pending);
}

#[test]
fn session_status_all_variants_snake_case() {
    for (variant, wire) in [
        (SessionStatus::Pending, "pending"),
        (SessionStatus::Decided, "decided"),
        (SessionStatus::Answered, "answered"),
        (SessionStatus::Approved, "approved"),
        (SessionStatus::ChangesRequested, "changes_requested"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let back: SessionStatus = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn session_status_changes_requested_uses_underscore() {
    let json = serde_json::to_value(SessionStatus::ChangesRequested).unwrap();
    assert_eq!(json, json!("changes_requested"));
}

#[test]
fn session_status_rejects_unknown() {
    let parsed: Result<SessionStatus, _> = serde_json::from_value(json!("rejected"));
    assert!(parsed.is_err());
}

#[test]
fn default_review_payload_status_is_pending() {
    let p = ReviewSessionPayload::default();
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["status"], "pending");
}

// -----------------------------------------------------------------------------
// MilestoneStatus
// -----------------------------------------------------------------------------

#[test]
fn milestone_status_snake_case_each() {
    for (variant, wire) in [
        (MilestoneStatus::Done, "done"),
        (MilestoneStatus::Active, "active"),
        (MilestoneStatus::Pending, "pending"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let back: MilestoneStatus = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn milestone_status_rejects_unknown() {
    let parsed: Result<MilestoneStatus, _> = serde_json::from_value(json!("blocked"));
    assert!(parsed.is_err());
}

#[test]
fn progress_milestone_roundtrips() {
    let m = ProgressMilestone {
        key: "review:spec".into(),
        label: "Review spec".into(),
        status: MilestoneStatus::Active,
    };
    assert_stable(&m);
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(json["key"], "review:spec");
    assert_eq!(json["status"], "active");
}

#[test]
fn progress_milestone_requires_all_fields() {
    // No skip attrs on ProgressMilestone — every field is mandatory.
    let parsed: Result<ProgressMilestone, _> =
        serde_json::from_value(json!({ "key": "k", "label": "l" }));
    assert!(parsed.is_err(), "missing status must fail");
}

#[test]
fn empty_drift_vec_is_omitted() {
    let p = review(ReviewSessionPayload {
        session_id: "d".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert!(json.get("drift").is_none(), "empty drift must be omitted");
}

// -----------------------------------------------------------------------------
// ApproveAction / ApproveActionKind — all nine kinds
// -----------------------------------------------------------------------------

#[test]
fn approve_action_kind_snake_case_each() {
    for (variant, wire) in [
        (ApproveActionKind::AdHocDone, "ad_hoc_done"),
        (ApproveActionKind::OpenPr, "open_pr"),
        (ApproveActionKind::SubmitExternal, "submit_external"),
        (ApproveActionKind::StartRun, "start_run"),
        (ApproveActionKind::StartExecution, "start_execution"),
        (ApproveActionKind::CompleteStation, "complete_station"),
        (ApproveActionKind::SubmitRunReview, "submit_run_review"),
        (ApproveActionKind::CompleteRun, "complete_run"),
        (ApproveActionKind::Approve, "approve"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let back: ApproveActionKind = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn approve_action_kind_rejects_unknown() {
    let parsed: Result<ApproveActionKind, _> = serde_json::from_value(json!("merge"));
    assert!(parsed.is_err());
}

#[test]
fn approve_action_kind_all_distinct_wire() {
    let kinds = [
        ApproveActionKind::AdHocDone,
        ApproveActionKind::OpenPr,
        ApproveActionKind::SubmitExternal,
        ApproveActionKind::StartRun,
        ApproveActionKind::StartExecution,
        ApproveActionKind::CompleteStation,
        ApproveActionKind::SubmitRunReview,
        ApproveActionKind::CompleteRun,
        ApproveActionKind::Approve,
    ];
    let mut seen = std::collections::HashSet::new();
    for k in kinds {
        let w = serde_json::to_value(k).unwrap();
        assert!(seen.insert(w.as_str().unwrap().to_string()));
    }
    assert_eq!(seen.len(), 9);
}

#[test]
fn approve_action_roundtrips_each_kind() {
    for kind in [
        ApproveActionKind::AdHocDone,
        ApproveActionKind::OpenPr,
        ApproveActionKind::SubmitExternal,
        ApproveActionKind::StartRun,
        ApproveActionKind::StartExecution,
        ApproveActionKind::CompleteStation,
        ApproveActionKind::SubmitRunReview,
        ApproveActionKind::CompleteRun,
        ApproveActionKind::Approve,
    ] {
        let action = ApproveAction {
            label: "Go".into(),
            kind,
        };
        let json = serde_json::to_value(&action).unwrap();
        assert_eq!(json["kind"], serde_json::to_value(kind).unwrap());
        assert_eq!(json["label"], "Go");
        assert_stable(&action);
    }
}

#[test]
fn approve_action_in_review_payload() {
    let p = review(ReviewSessionPayload {
        session_id: "a".into(),
        approve_action: Some(ApproveAction {
            label: "Complete Frame Station".into(),
            kind: ApproveActionKind::CompleteStation,
        }),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["approve_action"]["label"], "Complete Frame Station");
    assert_eq!(json["approve_action"]["kind"], "complete_station");
    assert_stable(&p);
}

#[test]
fn approve_action_requires_both_fields() {
    let parsed: Result<ApproveAction, _> =
        serde_json::from_value(json!({ "label": "x" }));
    assert!(parsed.is_err(), "missing kind must fail");
    let parsed2: Result<ApproveAction, _> =
        serde_json::from_value(json!({ "kind": "approve" }));
    assert!(parsed2.is_err(), "missing label must fail");
}

// -----------------------------------------------------------------------------
// SealStatus
// -----------------------------------------------------------------------------

#[test]
fn seal_status_snake_case() {
    assert_eq!(serde_json::to_value(SealStatus::Sealed).unwrap(), json!("sealed"));
    assert_eq!(
        serde_json::to_value(SealStatus::PendingSeal).unwrap(),
        json!("pending_seal")
    );
}

#[test]
fn seal_status_deserializes() {
    let a: SealStatus = serde_json::from_value(json!("sealed")).unwrap();
    assert_eq!(a, SealStatus::Sealed);
    let b: SealStatus = serde_json::from_value(json!("pending_seal")).unwrap();
    assert_eq!(b, SealStatus::PendingSeal);
}

#[test]
fn seal_status_rejects_unknown() {
    let parsed: Result<SealStatus, _> = serde_json::from_value(json!("unsealed"));
    assert!(parsed.is_err());
}

#[test]
fn seal_status_in_current_state() {
    let st = RunCurrentState {
        factory: "software".into(),
        station: "seal".into(),
        seal_status: Some(SealStatus::PendingSeal),
        awaiting_merge_into: Some("main".into()),
        ..Default::default()
    };
    let json = serde_json::to_value(&st).unwrap();
    assert_eq!(json["seal_status"], "pending_seal");
    assert_eq!(json["awaiting_merge_into"], "main");
    assert_stable(&st);
}

// -----------------------------------------------------------------------------
// OutputArtifactType / OutputArtifact
// -----------------------------------------------------------------------------

#[test]
fn output_artifact_type_snake_case_each() {
    for (variant, wire) in [
        (OutputArtifactType::Markdown, "markdown"),
        (OutputArtifactType::Html, "html"),
        (OutputArtifactType::Image, "image"),
        (OutputArtifactType::Video, "video"),
        (OutputArtifactType::Code, "code"),
        (OutputArtifactType::File, "file"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let back: OutputArtifactType = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn output_artifact_type_rejects_unknown() {
    let parsed: Result<OutputArtifactType, _> = serde_json::from_value(json!("pdf"));
    assert!(parsed.is_err());
}

#[test]
fn output_artifact_type_serializes_under_renamed_key() {
    let a = OutputArtifact {
        station: "frame".into(),
        name: "out.md".into(),
        artifact_type: OutputArtifactType::Markdown,
        language: None,
        directory: None,
        content: None,
        relative_path: None,
        run_relative_path: None,
    };
    let json = serde_json::to_value(&a).unwrap();
    // `#[serde(rename = "type")]` — wire key is `type`, not `artifact_type`.
    assert_eq!(json["type"], "markdown");
    assert!(json.get("artifact_type").is_none());
}

#[test]
fn output_artifact_minimal_omits_all_optionals() {
    let a = OutputArtifact {
        station: "".into(),
        name: "readme".into(),
        artifact_type: OutputArtifactType::File,
        language: None,
        directory: None,
        content: None,
        relative_path: None,
        run_relative_path: None,
    };
    let json = serde_json::to_value(&a).unwrap();
    for k in ["language", "directory", "content", "relative_path", "run_relative_path"] {
        assert!(json.get(k).is_none(), "{k} should be omitted");
    }
    // station empty string is NOT optional — it serializes.
    assert_eq!(json["station"], "");
    assert_stable(&a);
}

#[test]
fn output_artifact_code_carries_language() {
    let a = OutputArtifact {
        station: "build".into(),
        name: "main.rs".into(),
        artifact_type: OutputArtifactType::Code,
        language: Some("rust".into()),
        directory: Some("src".into()),
        content: Some("fn main() {}".into()),
        relative_path: Some("/api/file/main.rs".into()),
        run_relative_path: Some("build/src/main.rs".into()),
    };
    let json = serde_json::to_value(&a).unwrap();
    assert_eq!(json["type"], "code");
    assert_eq!(json["language"], "rust");
    assert_eq!(json["directory"], "src");
    assert_stable(&a);
}

#[test]
fn output_artifacts_and_other_files_distinguished_in_payload() {
    let p = review(ReviewSessionPayload {
        session_id: "o".into(),
        output_artifacts: vec![OutputArtifact {
            station: "frame".into(),
            name: "declared.md".into(),
            artifact_type: OutputArtifactType::Markdown,
            language: None,
            directory: None,
            content: None,
            relative_path: None,
            run_relative_path: None,
        }],
        other_files: vec![OutputArtifact {
            station: "frame".into(),
            name: "stray.txt".into(),
            artifact_type: OutputArtifactType::File,
            language: None,
            directory: None,
            content: None,
            relative_path: None,
            run_relative_path: None,
        }],
        run_other_files: vec![OutputArtifact {
            station: "".into(),
            name: "root.log".into(),
            artifact_type: OutputArtifactType::File,
            language: None,
            directory: None,
            content: None,
            relative_path: None,
            run_relative_path: None,
        }],
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["output_artifacts"][0]["name"], "declared.md");
    assert_eq!(json["other_files"][0]["name"], "stray.txt");
    assert_eq!(json["run_other_files"][0]["name"], "root.log");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// UnitOutputType / UnitOutputPreview
// -----------------------------------------------------------------------------

#[test]
fn unit_output_type_snake_case_each() {
    for (variant, wire) in [
        (UnitOutputType::Markdown, "markdown"),
        (UnitOutputType::Html, "html"),
        (UnitOutputType::Image, "image"),
        (UnitOutputType::File, "file"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let back: UnitOutputType = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn unit_output_type_has_no_video_variant() {
    // UnitOutputType is a strict subset of OutputArtifactType — no `video`.
    let parsed: Result<UnitOutputType, _> = serde_json::from_value(json!("video"));
    assert!(parsed.is_err());
    let parsed_code: Result<UnitOutputType, _> = serde_json::from_value(json!("code"));
    assert!(parsed_code.is_err());
}

#[test]
fn unit_output_preview_type_renamed_to_type() {
    let u = UnitOutputPreview {
        path: "out/a.md".into(),
        name: "A".into(),
        output_type: UnitOutputType::Markdown,
        url: "/api/file/a.md".into(),
        preview_body: None,
        size_bytes: None,
        exists: true,
    };
    let json = serde_json::to_value(&u).unwrap();
    assert_eq!(json["type"], "markdown");
    assert!(json.get("output_type").is_none());
    // exists is mandatory (no skip) and present.
    assert_eq!(json["exists"], true);
    assert_stable(&u);
}

#[test]
fn unit_output_preview_minimal_omits_preview_body_and_size() {
    let u = UnitOutputPreview {
        path: "p".into(),
        name: "n".into(),
        output_type: UnitOutputType::File,
        url: "u".into(),
        preview_body: None,
        size_bytes: None,
        exists: false,
    };
    let json = serde_json::to_value(&u).unwrap();
    assert!(json.get("preview_body").is_none());
    assert!(json.get("size_bytes").is_none());
    assert_eq!(json["exists"], false);
}

#[test]
fn unit_output_preview_size_bytes_boundaries() {
    for size in [0u64, 1, 1023, 1024, u32::MAX as u64, u64::MAX] {
        let u = UnitOutputPreview {
            path: "p".into(),
            name: "n".into(),
            output_type: UnitOutputType::Image,
            url: "u".into(),
            preview_body: Some("body".into()),
            size_bytes: Some(size),
            exists: true,
        };
        let json = serde_json::to_value(&u).unwrap();
        assert_eq!(json["size_bytes"], json!(size));
        assert_stable(&u);
    }
}

#[test]
fn unit_outputs_map_in_payload_keyed_by_unit() {
    let mut unit_outputs = BTreeMap::new();
    unit_outputs.insert(
        "unit-1".to_string(),
        vec![UnitOutputPreview {
            path: "out/1.md".into(),
            name: "One".into(),
            output_type: UnitOutputType::Markdown,
            url: "/u/1".into(),
            preview_body: Some("# hi".into()),
            size_bytes: Some(42),
            exists: true,
        }],
    );
    let p = review(ReviewSessionPayload {
        session_id: "u".into(),
        unit_outputs,
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["unit_outputs"]["unit-1"][0]["name"], "One");
    assert_eq!(json["unit_outputs"]["unit-1"][0]["exists"], true);
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// StationStateInfo — the only authoritative bool + display shims
// -----------------------------------------------------------------------------

#[test]
fn station_state_info_minimal_only_required() {
    let s = StationStateInfo {
        station: "frame".into(),
        merged_into_main: false,
        status: None,
        phase: None,
        started_at: None,
        completed_at: None,
        gate_entered_at: None,
        gate_outcome: None,
    };
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["station"], "frame");
    assert_eq!(json["merged_into_main"], false);
    for k in ["status", "phase", "started_at", "completed_at", "gate_entered_at", "gate_outcome"] {
        assert!(json.get(k).is_none(), "{k} should be omitted");
    }
    assert_stable(&s);
}

#[test]
fn station_state_info_full_roundtrips() {
    let s = StationStateInfo {
        station: "frame".into(),
        merged_into_main: true,
        status: Some("complete".into()),
        phase: Some("checkpoint".into()),
        started_at: Some("2026-01-01T00:00:00Z".into()),
        completed_at: Some("2026-01-01T01:00:00Z".into()),
        gate_entered_at: Some("2026-01-01T00:30:00Z".into()),
        gate_outcome: Some("approved".into()),
    };
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["merged_into_main"], true);
    assert_eq!(json["gate_outcome"], "approved");
    assert_stable(&s);
}

#[test]
fn station_state_info_merged_flag_is_mandatory() {
    let parsed: Result<StationStateInfo, _> =
        serde_json::from_value(json!({ "station": "frame" }));
    assert!(parsed.is_err(), "merged_into_main is required");
}

#[test]
fn station_states_map_in_payload() {
    // An ORDERED Vec in factory order (frame before build), NOT alphabetical —
    // serializes to a JSON array, and the order is preserved on the wire.
    let states = vec![
        StationStateInfo {
            station: "frame".into(),
            merged_into_main: true,
            status: None,
            phase: None,
            started_at: None,
            completed_at: None,
            gate_entered_at: None,
            gate_outcome: None,
        },
        StationStateInfo {
            station: "build".into(),
            merged_into_main: false,
            status: Some("active".into()),
            phase: None,
            started_at: None,
            completed_at: None,
            gate_entered_at: None,
            gate_outcome: None,
        },
    ];
    let p = review(ReviewSessionPayload {
        session_id: "ss".into(),
        station_states: states,
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["station_states"][0]["station"], "frame");
    assert_eq!(json["station_states"][0]["merged_into_main"], true);
    assert_eq!(json["station_states"][1]["station"], "build");
    assert_eq!(json["station_states"][1]["status"], "active");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// KnowledgeFile / StationArtifact
// -----------------------------------------------------------------------------

#[test]
fn knowledge_file_roundtrips() {
    let k = KnowledgeFile {
        name: "context.md".into(),
        content: "## background\nstuff".into(),
    };
    let json = serde_json::to_value(&k).unwrap();
    assert_eq!(json["name"], "context.md");
    assert_eq!(json["content"], "## background\nstuff");
    assert_stable(&k);
}

#[test]
fn knowledge_file_requires_both_fields() {
    let parsed: Result<KnowledgeFile, _> =
        serde_json::from_value(json!({ "name": "x" }));
    assert!(parsed.is_err());
}

#[test]
fn station_artifact_roundtrips() {
    let a = StationArtifact {
        station: "frame".into(),
        name: "notes.txt".into(),
        content: "line".into(),
    };
    let json = serde_json::to_value(&a).unwrap();
    assert_eq!(json["station"], "frame");
    assert_stable(&a);
}

#[test]
fn knowledge_files_and_station_artifacts_vecs_in_payload() {
    let p = review(ReviewSessionPayload {
        session_id: "kf".into(),
        knowledge_files: vec![KnowledgeFile {
            name: "a".into(),
            content: "x".into(),
        }],
        station_artifacts: vec![StationArtifact {
            station: "frame".into(),
            name: "b".into(),
            content: "y".into(),
        }],
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["knowledge_files"][0]["name"], "a");
    assert_eq!(json["station_artifacts"][0]["station"], "frame");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// PreviousReviewSnapshot
// -----------------------------------------------------------------------------

#[test]
fn previous_review_snapshot_roundtrips() {
    let mut units = BTreeMap::new();
    units.insert("unit-1".to_string(), "raw body 1".to_string());
    units.insert("unit-2".to_string(), "raw body 2".to_string());
    let snap = PreviousReviewSnapshot {
        feedback: "please tighten copy".into(),
        reviewed_at: "2026-01-01T00:00:00Z".into(),
        run_raw_content: "# run".into(),
        unit_raw_contents: units,
    };
    let json = serde_json::to_value(&snap).unwrap();
    assert_eq!(json["feedback"], "please tighten copy");
    assert_eq!(json["unit_raw_contents"]["unit-1"], "raw body 1");
    assert_stable(&snap);
}

#[test]
fn previous_review_snapshot_empty_unit_map_still_serializes() {
    // No skip attr on unit_raw_contents — empty map serializes as `{}`.
    let snap = PreviousReviewSnapshot {
        feedback: "f".into(),
        reviewed_at: "t".into(),
        run_raw_content: "c".into(),
        unit_raw_contents: BTreeMap::new(),
    };
    let json = serde_json::to_value(&snap).unwrap();
    assert_eq!(json["unit_raw_contents"], json!({}));
}

#[test]
fn previous_review_attaches_to_payload() {
    let p = review(ReviewSessionPayload {
        session_id: "pr".into(),
        previous_review: Some(PreviousReviewSnapshot {
            feedback: "redo".into(),
            reviewed_at: "t".into(),
            run_raw_content: "rc".into(),
            unit_raw_contents: BTreeMap::new(),
        }),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["previous_review"]["feedback"], "redo");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// DiscoveredReviewUrl / DiscoveredReviewSource (kebab-case)
// -----------------------------------------------------------------------------

#[test]
fn discovered_review_source_kebab_case() {
    let pr = serde_json::to_value(
        DiscoveredReviewUrl {
            url: "https://x".into(),
            source: darkrun_api::session::DiscoveredReviewSource::GithubPrRef,
            pr_number: 42,
            matched_sha: "abc".into(),
        }
        .source,
    )
    .unwrap();
    assert_eq!(pr, json!("github-pr-ref"));
}

#[test]
fn discovered_review_url_both_sources_roundtrip() {
    use darkrun_api::session::DiscoveredReviewSource;
    for (source, wire) in [
        (DiscoveredReviewSource::GithubPrRef, "github-pr-ref"),
        (DiscoveredReviewSource::GitlabMrRef, "gitlab-mr-ref"),
    ] {
        let d = DiscoveredReviewUrl {
            url: "https://example/pr/1".into(),
            source,
            pr_number: 1,
            matched_sha: "deadbeef".into(),
        };
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["source"], json!(wire));
        assert_stable(&d);
    }
}

#[test]
fn discovered_review_source_rejects_snake_case() {
    use darkrun_api::session::DiscoveredReviewSource;
    let parsed: Result<DiscoveredReviewSource, _> =
        serde_json::from_value(json!("github_pr_ref"));
    assert!(parsed.is_err(), "kebab-case enum must reject snake_case");
}

#[test]
fn discovered_review_url_pr_number_boundaries() {
    use darkrun_api::session::DiscoveredReviewSource;
    for n in [0u64, 1, 9999, u64::MAX] {
        let d = DiscoveredReviewUrl {
            url: "u".into(),
            source: DiscoveredReviewSource::GitlabMrRef,
            pr_number: n,
            matched_sha: "sha".into(),
        };
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["pr_number"], json!(n));
        assert_stable(&d);
    }
}

#[test]
fn discovered_review_url_attaches_to_payload() {
    use darkrun_api::session::DiscoveredReviewSource;
    let p = review(ReviewSessionPayload {
        session_id: "du".into(),
        discovered_review_url: Some(DiscoveredReviewUrl {
            url: "https://github/pr/7".into(),
            source: DiscoveredReviewSource::GithubPrRef,
            pr_number: 7,
            matched_sha: "f00".into(),
        }),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["discovered_review_url"]["pr_number"], 7);
    assert_eq!(json["discovered_review_url"]["source"], "github-pr-ref");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// PendingDecision
// -----------------------------------------------------------------------------

#[test]
fn pending_decision_roundtrips() {
    let pd = PendingDecision {
        decision: "approved".into(),
        feedback: "looks good".into(),
        submitted_at: "2026-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_value(&pd).unwrap();
    assert_eq!(json["decision"], "approved");
    assert_stable(&pd);
}

#[test]
fn pending_decision_requires_all_fields() {
    let parsed: Result<PendingDecision, _> =
        serde_json::from_value(json!({ "decision": "x", "feedback": "y" }));
    assert!(parsed.is_err(), "missing submitted_at must fail");
}

#[test]
fn pending_decision_in_review_payload() {
    let p = review(ReviewSessionPayload {
        session_id: "pd".into(),
        pending_decision: Some(PendingDecision {
            decision: "changes_requested".into(),
            feedback: "redo the header".into(),
            submitted_at: "t".into(),
        }),
        await_active: Some(false),
        await_count: Some(3),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["pending_decision"]["decision"], "changes_requested");
    assert_eq!(json["await_active"], false);
    assert_eq!(json["await_count"], 3);
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// RunCurrentState — milestones, indices, signals
// -----------------------------------------------------------------------------

#[test]
fn run_current_state_default_omits_all_optionals() {
    let st = RunCurrentState::default();
    let json = serde_json::to_value(&st).unwrap();
    // Two mandatory string fields always serialize (default empty).
    assert_eq!(json["factory"], "");
    assert_eq!(json["station"], "");
    for k in [
        "phase",
        "step",
        "pending_signals",
        "milestones",
        "progress_index",
        "progress_total",
        "seal_status",
        "awaiting_merge_into",
    ] {
        assert!(json.get(k).is_none(), "{k} should be omitted by default");
    }
}

#[test]
fn run_current_state_full_roundtrips() {
    let st = RunCurrentState {
        factory: "software".into(),
        station: "frame".into(),
        phase: Some(RunPhase::Manufacture),
        step: Some("pass-2".into()),
        pending_signals: vec!["needs:design".into(), "needs:data".into()],
        milestones: vec![
            ProgressMilestone {
                key: "spec".into(),
                label: "Spec".into(),
                status: MilestoneStatus::Done,
            },
            ProgressMilestone {
                key: "manufacture".into(),
                label: "Manufacture".into(),
                status: MilestoneStatus::Active,
            },
        ],
        progress_index: Some(1),
        progress_total: Some(2),
        seal_status: None,
        awaiting_merge_into: None,
    };
    let json = serde_json::to_value(&st).unwrap();
    assert_eq!(json["phase"], "manufacture");
    assert_eq!(json["step"], "pass-2");
    assert_eq!(json["pending_signals"].as_array().unwrap().len(), 2);
    assert_eq!(json["milestones"][1]["status"], "active");
    assert_eq!(json["progress_index"], 1);
    assert_eq!(json["progress_total"], 2);
    assert_stable(&st);
}

#[test]
fn run_current_state_progress_index_boundaries() {
    for idx in [0u32, 1, u32::MAX] {
        let st = RunCurrentState {
            factory: "f".into(),
            station: "s".into(),
            progress_index: Some(idx),
            progress_total: Some(idx),
            ..Default::default()
        };
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["progress_index"], json!(idx));
        assert_stable(&st);
    }
}

#[test]
fn run_current_state_empty_signals_omitted() {
    let st = RunCurrentState {
        factory: "f".into(),
        station: "s".into(),
        pending_signals: vec![],
        milestones: vec![],
        ..Default::default()
    };
    let json = serde_json::to_value(&st).unwrap();
    assert!(json.get("pending_signals").is_none());
    assert!(json.get("milestones").is_none());
}

#[test]
fn run_current_state_in_review_payload() {
    let p = review(ReviewSessionPayload {
        session_id: "cs".into(),
        current_state: Some(RunCurrentState {
            factory: "software".into(),
            station: "frame".into(),
            phase: Some(RunPhase::Checkpoint),
            ..Default::default()
        }),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["current_state"]["factory"], "software");
    assert_eq!(json["current_state"]["phase"], "checkpoint");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// ReviewSessionPayload — opaque Value fields, maps, big shape
// -----------------------------------------------------------------------------

#[test]
fn review_payload_default_is_minimal() {
    let p = ReviewSessionPayload::default();
    let json = serde_json::to_value(&p).unwrap();
    // Only the two mandatory fields show up.
    assert_eq!(json["session_id"], "");
    assert_eq!(json["status"], "pending");
    // None of the optional collections / fields appear.
    for k in [
        "run_slug", "run_dir", "gate_type", "target", "decision", "feedback",
        "annotations", "run", "units", "criteria", "mermaid", "station_states",
        "drift", "approve_action", "current_state", "previous_review",
    ] {
        assert!(json.get(k).is_none(), "default review must omit {k}");
    }
}

#[test]
fn review_payload_opaque_run_value_passthrough() {
    let run = json!({ "title": "My Run", "nested": { "a": [1, 2, 3] } });
    let p = review(ReviewSessionPayload {
        session_id: "r".into(),
        run: Some(run.clone()),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["run"], run);
    assert_stable(&p);
}

#[test]
fn review_payload_opaque_units_and_criteria_vecs() {
    let p = review(ReviewSessionPayload {
        session_id: "r".into(),
        units: vec![json!({ "slug": "u1" }), json!({ "slug": "u2" })],
        criteria: vec![json!({ "text": "must pass" })],
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["units"].as_array().unwrap().len(), 2);
    assert_eq!(json["units"][0]["slug"], "u1");
    assert_eq!(json["criteria"][0]["text"], "must pass");
    assert_stable(&p);
}

#[test]
fn review_payload_opaque_units_accept_arbitrary_json() {
    // Opaque Value fields must accept any JSON shape, including scalars/null.
    let p = review(ReviewSessionPayload {
        session_id: "r".into(),
        units: vec![json!(null), json!(42), json!("text"), json!([1, 2])],
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["units"], json!([null, 42, "text", [1, 2]]));
    assert_stable(&p);
}

#[test]
fn review_payload_empty_units_criteria_omitted() {
    let p = review(ReviewSessionPayload {
        session_id: "r".into(),
        units: vec![],
        criteria: vec![],
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert!(json.get("units").is_none());
    assert!(json.get("criteria").is_none());
}

#[test]
fn review_payload_string_map_fields() {
    let mut summaries = BTreeMap::new();
    summaries.insert("frame".to_string(), "the frame station".to_string());
    let mut briefs = BTreeMap::new();
    briefs.insert("frame".to_string(), "user-facing brief".to_string());
    let mut observations = BTreeMap::new();
    observations.insert("frame".to_string(), "saw X".to_string());
    let mut elaborations = BTreeMap::new();
    elaborations.insert("frame".to_string(), "elaborated narrative".to_string());
    let p = review(ReviewSessionPayload {
        session_id: "m".into(),
        station_summaries: summaries,
        station_briefs: briefs,
        station_observations: observations,
        station_elaborations: elaborations,
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["station_summaries"]["frame"], "the frame station");
    assert_eq!(json["station_briefs"]["frame"], "user-facing brief");
    assert_eq!(json["station_observations"]["frame"], "saw X");
    assert_eq!(json["station_elaborations"]["frame"], "elaborated narrative");
    assert_stable(&p);
}

#[test]
fn review_payload_empty_maps_omitted() {
    let p = review(ReviewSessionPayload {
        session_id: "m".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    for k in [
        "station_states",
        "station_summaries",
        "station_briefs",
        "station_observations",
        "station_elaborations",
        "station_milestones",
        "unit_outputs",
        "output_declared_by",
    ] {
        assert!(json.get(k).is_none(), "empty map {k} must be omitted");
    }
}

#[test]
fn review_payload_station_milestones_map() {
    let mut milestones = BTreeMap::new();
    milestones.insert(
        "frame".to_string(),
        vec![
            ProgressMilestone {
                key: "spec".into(),
                label: "Spec".into(),
                status: MilestoneStatus::Done,
            },
            ProgressMilestone {
                key: "reflect".into(),
                label: "Reflect".into(),
                status: MilestoneStatus::Pending,
            },
        ],
    );
    let p = review(ReviewSessionPayload {
        session_id: "sm".into(),
        station_milestones: milestones,
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["station_milestones"]["frame"][0]["status"], "done");
    assert_eq!(json["station_milestones"]["frame"][1]["status"], "pending");
    assert_stable(&p);
}

#[test]
fn review_payload_output_declared_by_inverse_map() {
    let mut declared = BTreeMap::new();
    declared.insert(
        "out/shared.md".to_string(),
        vec!["unit-1".to_string(), "unit-2".to_string()],
    );
    let p = review(ReviewSessionPayload {
        session_id: "od".into(),
        output_declared_by: declared,
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(
        json["output_declared_by"]["out/shared.md"],
        json!(["unit-1", "unit-2"])
    );
    assert_stable(&p);
}

#[test]
fn review_payload_annotations_attach() {
    let p = review(ReviewSessionPayload {
        session_id: "an".into(),
        annotations: Some(ReviewAnnotations {
            screenshot: Some("data:image/png;base64,AA".into()),
            pins: vec![darkrun_api::common::Pin {
                x: 0.5,
                y: 0.25,
                text: "look here".into(),
            }],
            comments: vec![darkrun_api::common::InlineComment {
                selected_text: "foo".into(),
                comment: "rename".into(),
                paragraph: 3,
                location: Some("frame/spec.md".into()),
            }],
        }),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["annotations"]["pins"][0]["x"], 0.5);
    assert_eq!(json["annotations"]["comments"][0]["paragraph"], 3);
    assert_stable(&p);
}

#[test]
fn review_payload_decision_and_feedback_strings() {
    let p = review(ReviewSessionPayload {
        session_id: "df".into(),
        decision: Some("approved".into()),
        feedback: Some("ship it".into()),
        target: Some("frame/spec.md".into()),
        run_dir: Some("/runs/my-run".into()),
        run_slug: Some("my-run".into()),
        mermaid: Some("graph TD; A-->B".into()),
        reflection: Some("the run did X".into()),
        gate_context: Some("post-manufacture".into()),
        next_station: Some("build".into()),
        next_phase: Some("spec".into()),
        ad_hoc: Some(true),
        station: Some("frame".into()),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["decision"], "approved");
    assert_eq!(json["feedback"], "ship it");
    assert_eq!(json["mermaid"], "graph TD; A-->B");
    assert_eq!(json["ad_hoc"], true);
    assert_eq!(json["next_station"], "build");
    assert_stable(&p);
}

#[test]
fn review_payload_await_timestamps() {
    let p = review(ReviewSessionPayload {
        session_id: "aw".into(),
        await_active: Some(true),
        await_count: Some(0),
        last_await_started_at: Some("2026-01-01T00:00:00Z".into()),
        last_await_ended_at: Some("2026-01-01T00:05:00Z".into()),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["await_active"], true);
    assert_eq!(json["await_count"], 0);
    assert_eq!(json["last_await_started_at"], "2026-01-01T00:00:00Z");
    assert_stable(&p);
}

#[test]
fn review_payload_await_count_boundaries() {
    for c in [0u32, 1, 100, u32::MAX] {
        let p = review(ReviewSessionPayload {
            session_id: "c".into(),
            await_count: Some(c),
            ..Default::default()
        });
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["await_count"], json!(c));
        assert_stable(&p);
    }
}

#[test]
fn review_payload_ad_hoc_false_still_serializes() {
    // ad_hoc is Option<bool>; Some(false) is present, only None is skipped.
    let p = review(ReviewSessionPayload {
        session_id: "ah".into(),
        ad_hoc: Some(false),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["ad_hoc"], false);
}

// -----------------------------------------------------------------------------
// QuestionSessionPayload + QuestionDef + QuestionAnswer
// -----------------------------------------------------------------------------

#[test]
fn question_payload_default_minimal() {
    let p = QuestionSessionPayload::default();
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_id"], "");
    assert_eq!(json["status"], "pending");
    // multi_select is a plain bool that defaults to false and always serializes.
    assert_eq!(json["multi_select"], false);
    for k in ["title", "prompt", "context", "options", "answer", "image_urls"] {
        assert!(json.get(k).is_none(), "{k} omitted by default");
    }
}

#[test]
fn question_option_minimal_and_full() {
    let minimal = QuestionOption {
        id: "a".into(),
        label: "Option A".into(),
        image_url: None,
        image_url_light: None,
        description: None,
    };
    let json = serde_json::to_value(&minimal).unwrap();
    assert!(json.get("image_url").is_none());
    assert!(json.get("description").is_none());
    assert_eq!(json["id"], "a");
    assert_eq!(json["label"], "Option A");
    assert_stable(&minimal);

    let full = QuestionOption {
        id: "b".into(),
        label: "Option B".into(),
        image_url: Some("/mockups/b.png".into()),
        image_url_light: None,
        description: Some("the bold one".into()),
    };
    let json2 = serde_json::to_value(&full).unwrap();
    assert_eq!(json2["image_url"], "/mockups/b.png");
    assert_eq!(json2["description"], "the bold one");
    assert_stable(&full);
}

#[test]
fn question_answer_minimal_and_text() {
    let minimal = QuestionAnswer {
        selected: vec!["a".into()],
        text: None,
    };
    let json = serde_json::to_value(&minimal).unwrap();
    assert!(json.get("text").is_none());
    assert_eq!(json["selected"], json!(["a"]));
    assert_stable(&minimal);

    let with_text = QuestionAnswer {
        selected: vec![],
        text: Some("custom".into()),
    };
    let json2 = serde_json::to_value(&with_text).unwrap();
    assert_eq!(json2["text"], "custom");
    // empty selected vec is omitted.
    assert!(json2.get("selected").is_none());
    assert_stable(&with_text);
}

#[test]
fn question_answer_default_is_empty() {
    let a = QuestionAnswer::default();
    let json = serde_json::to_value(&a).unwrap();
    assert_eq!(json, json!({}));
}

#[test]
fn question_payload_full_roundtrips() {
    let p = SessionPayload::Question(QuestionSessionPayload {
        session_id: "q".into(),
        status: SessionStatus::Answered,
        run_slug: None,
        title: Some("Onboarding".into()),
        prompt: "Which mockup feels right?".into(),
        context: Some("## context".into()),
        options: vec![
            QuestionOption {
                id: "red".into(),
                label: "Crimson".into(),
                image_url: Some("/mock/red.png".into()),
                image_url_light: None,
                description: Some("bold + warm".into()),
            },
            QuestionOption {
                id: "blue".into(),
                label: "Cobalt".into(),
                image_url: Some("/mock/blue.png".into()),
                image_url_light: None,
                description: None,
            },
        ],
        multi_select: false,
        answer: Some(QuestionAnswer {
            selected: vec!["red".into()],
            text: Some("love the warmth".into()),
        }),
        image_urls: vec!["/img/1.png".into(), "/img/2.png".into()],
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "question");
    assert_eq!(json["status"], "answered");
    assert_eq!(json["prompt"], "Which mockup feels right?");
    assert_eq!(json["options"][0]["id"], "red");
    assert_eq!(json["options"][0]["image_url"], "/mock/red.png");
    assert_eq!(json["multi_select"], false);
    assert_eq!(json["answer"]["selected"][0], "red");
    assert_eq!(json["answer"]["text"], "love the warmth");
    assert_eq!(json["image_urls"].as_array().unwrap().len(), 2);
    assert_stable(&p);
}

#[test]
fn question_payload_multi_select_true() {
    let p = QuestionSessionPayload {
        session_id: "q".into(),
        prompt: "Pick all that apply".into(),
        multi_select: true,
        options: vec![QuestionOption {
            id: "x".into(),
            label: "X".into(),
            image_url: None,
            image_url_light: None,
            description: None,
        }],
        ..Default::default()
    };
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["multi_select"], true);
    assert_stable(&p);
}

#[test]
fn question_payload_image_urls_empty_omitted() {
    let p = QuestionSessionPayload {
        session_id: "q".into(),
        image_urls: vec![],
        ..Default::default()
    };
    let json = serde_json::to_value(&p).unwrap();
    assert!(json.get("image_urls").is_none());
}

// -----------------------------------------------------------------------------
// DirectionSessionPayload + archetypes + selection
// -----------------------------------------------------------------------------

#[test]
fn direction_payload_default_minimal() {
    let p = DirectionSessionPayload::default();
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_id"], "");
    assert_eq!(json["status"], "pending");
    for k in [
        "title",
        "run_slug",
        "prompt",
        "context",
        "archetypes",
        "chosen_archetype",
        "annotations",
    ] {
        assert!(json.get(k).is_none(), "{k} omitted by default");
    }
}

#[test]
fn direction_archetype_roundtrips() {
    let a = DirectionArchetype {
        id: "brutalist".into(),
        label: "Brutalist".into(),
        image_url: "/mock/brutalist.png".into(),
        image_url_light: None,
        description: "raw concrete".into(),
    };
    let json = serde_json::to_value(&a).unwrap();
    assert_eq!(json["id"], "brutalist");
    assert_eq!(json["label"], "Brutalist");
    assert_eq!(json["image_url"], "/mock/brutalist.png");
    assert_eq!(json["description"], "raw concrete");
    assert_stable(&a);
}

#[test]
fn direction_archetype_requires_all_fields() {
    // No skip attrs — every field is mandatory.
    let parsed: Result<DirectionArchetype, _> = serde_json::from_value(json!({
        "id": "x",
        "label": "X",
        "description": "d"
        // missing image_url
    }));
    assert!(parsed.is_err(), "image_url is required");
}

#[test]
fn direction_annotations_minimal_and_full() {
    let minimal = DirectionAnnotations::default();
    let json = serde_json::to_value(&minimal).unwrap();
    assert!(json.get("pins").is_none());
    assert!(json.get("screenshot").is_none());
    assert!(json.get("comments").is_none());
    assert_eq!(json, json!({}));

    let full = DirectionAnnotations {
        pins: vec![DirectionPin {
            x: 0.5,
            y: 0.25,
            note: "tighten the header".into(),
        }],
        screenshot: Some("data:image/png;base64,AA".into()),
        comments: vec!["love the palette".into(), "loosen the grid".into()],
    };
    let json2 = serde_json::to_value(&full).unwrap();
    assert_eq!(json2["pins"][0]["x"], 0.5);
    assert_eq!(json2["pins"][0]["note"], "tighten the header");
    assert_eq!(json2["screenshot"], "data:image/png;base64,AA");
    assert_eq!(json2["comments"].as_array().unwrap().len(), 2);
    assert_stable(&full);
}

#[test]
fn direction_pin_uses_note_field() {
    let pin = DirectionPin {
        x: 0.1,
        y: 0.2,
        note: "here".into(),
    };
    let json = serde_json::to_value(&pin).unwrap();
    assert_eq!(json["note"], "here");
    // The legacy `text` key is gone.
    assert!(json.get("text").is_none());
    assert_stable(&pin);
}

#[test]
fn direction_payload_full_roundtrips() {
    let p = SessionPayload::Direction(DirectionSessionPayload {
        session_id: "d".into(),
        status: SessionStatus::Decided,
        title: Some("Pick a vibe".into()),
        run_slug: Some("run".into()),
        prompt: "Which design direction?".into(),
        context: Some("## options".into()),
        archetypes: vec![
            DirectionArchetype {
                id: "a".into(),
                label: "A".into(),
                image_url: "/mock/a.png".into(),
                image_url_light: None,
                description: "da".into(),
            },
            DirectionArchetype {
                id: "b".into(),
                label: "B".into(),
                image_url: "/mock/b.png".into(),
                image_url_light: None,
                description: "db".into(),
            },
        ],
        chosen_archetype: Some("a".into()),
        annotations: Some(DirectionAnnotations {
            pins: vec![DirectionPin {
                x: 0.5,
                y: 0.5,
                note: "more of this".into(),
            }],
            screenshot: None,
            comments: vec!["good".into()],
        }),
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "direction");
    assert_eq!(json["prompt"], "Which design direction?");
    assert_eq!(json["archetypes"].as_array().unwrap().len(), 2);
    assert_eq!(json["archetypes"][0]["image_url"], "/mock/a.png");
    assert_eq!(json["chosen_archetype"], "a");
    assert_eq!(json["annotations"]["pins"][0]["note"], "more of this");
    assert_eq!(json["annotations"]["comments"][0], "good");
    assert_stable(&p);
}

// -----------------------------------------------------------------------------
// PickerSessionPayload + PickerKind + options
// -----------------------------------------------------------------------------

#[test]
fn picker_kind_snake_case_each() {
    for (variant, wire) in [
        (PickerKind::Factory, "factory"),
        (PickerKind::Mode, "mode"),
        (PickerKind::Station, "station"),
        (PickerKind::Confirm, "confirm"),
        (PickerKind::UrlInput, "url_input"),
    ] {
        assert_eq!(serde_json::to_value(variant).unwrap(), json!(wire));
        let back: PickerKind = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn picker_kind_rejects_unknown() {
    let parsed: Result<PickerKind, _> = serde_json::from_value(json!("dropdown"));
    assert!(parsed.is_err());
}

#[test]
fn picker_option_minimal_and_full() {
    let minimal = PickerOption {
        id: "software".into(),
        label: "Software".into(),
        description: None,
        secondary: None,
    };
    let json = serde_json::to_value(&minimal).unwrap();
    assert!(json.get("description").is_none());
    assert!(json.get("secondary").is_none());
    assert_stable(&minimal);

    let full = PickerOption {
        id: "design".into(),
        label: "Design".into(),
        description: Some("for visual work".into()),
        secondary: Some(true),
    };
    let json2 = serde_json::to_value(&full).unwrap();
    assert_eq!(json2["description"], "for visual work");
    assert_eq!(json2["secondary"], true);
    assert_stable(&full);
}

#[test]
fn picker_option_secondary_false_still_present() {
    let o = PickerOption {
        id: "x".into(),
        label: "X".into(),
        description: None,
        secondary: Some(false),
    };
    let json = serde_json::to_value(&o).unwrap();
    assert_eq!(json["secondary"], false);
}

#[test]
fn picker_selection_roundtrips() {
    let s = PickerSelection { id: "software".into() };
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["id"], "software");
    assert_stable(&s);
}

#[test]
fn picker_payload_full_roundtrips_each_kind() {
    for kind in [
        PickerKind::Factory,
        PickerKind::Mode,
        PickerKind::Station,
        PickerKind::Confirm,
        PickerKind::UrlInput,
    ] {
        let p = SessionPayload::Picker(PickerSessionPayload {
            session_id: "p".into(),
            status: SessionStatus::Pending,
            run_slug: Some("run".into()),
            kind,
            title: "Pick".into(),
            prompt: "Which?".into(),
            options: vec![
                PickerOption {
                    id: "a".into(),
                    label: "A".into(),
                    description: Some("first".into()),
                    secondary: Some(false),
                },
                PickerOption {
                    id: "b".into(),
                    label: "B".into(),
                    description: None,
                    secondary: Some(true),
                },
            ],
            selection: Some(PickerSelection { id: "a".into() }),
        });
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["session_type"], "picker");
        assert_eq!(json["kind"], serde_json::to_value(kind).unwrap());
        assert_eq!(json["selection"]["id"], "a");
        assert_stable(&p);
    }
}

#[test]
fn picker_payload_required_fields_must_be_present() {
    // title, prompt, options, kind have no defaults/skip — all required.
    let parsed: Result<PickerSessionPayload, _> = serde_json::from_value(json!({
        "session_id": "p",
        "status": "pending",
        "kind": "factory",
        "title": "t"
        // missing prompt + options
    }));
    assert!(parsed.is_err());
}

#[test]
fn picker_payload_no_run_slug_omitted() {
    let p = sample_picker("p");
    let json = serde_json::to_value(&p).unwrap();
    assert!(json.get("run_slug").is_none());
}

// -----------------------------------------------------------------------------
// ViewSessionPayload + ViewMode + ViewStatus
// -----------------------------------------------------------------------------

#[test]
fn view_mode_snake_case() {
    assert_eq!(serde_json::to_value(ViewMode::Viewer).unwrap(), json!("viewer"));
    assert_eq!(serde_json::to_value(ViewMode::Boot).unwrap(), json!("boot"));
    let v: ViewMode = serde_json::from_value(json!("viewer")).unwrap();
    assert_eq!(v, ViewMode::Viewer);
    let b: ViewMode = serde_json::from_value(json!("boot")).unwrap();
    assert_eq!(b, ViewMode::Boot);
}

#[test]
fn view_mode_rejects_unknown() {
    let parsed: Result<ViewMode, _> = serde_json::from_value(json!("edit"));
    assert!(parsed.is_err());
}

#[test]
fn view_status_snake_case() {
    assert_eq!(serde_json::to_value(ViewStatus::Open).unwrap(), json!("open"));
    assert_eq!(serde_json::to_value(ViewStatus::Closed).unwrap(), json!("closed"));
    let o: ViewStatus = serde_json::from_value(json!("open")).unwrap();
    assert_eq!(o, ViewStatus::Open);
}

#[test]
fn view_status_rejects_unknown() {
    let parsed: Result<ViewStatus, _> = serde_json::from_value(json!("hidden"));
    assert!(parsed.is_err());
}

#[test]
fn view_payload_viewer_minimal_omits_boot_fields() {
    let p = sample_view("v");
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["mode"], "viewer");
    assert_eq!(json["status"], "open");
    for k in ["factory", "station", "artifact", "boot_port", "boot_command"] {
        assert!(json.get(k).is_none(), "{k} omitted on minimal view");
    }
    assert_stable(&SessionPayload::View(p));
}

#[test]
fn view_payload_boot_mode_full_roundtrips() {
    let p = SessionPayload::View(ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: Some("software".into()),
        station: Some("frame".into()),
        artifact: Some("index.html".into()),
        mode: ViewMode::Boot,
        boot_port: Some(3000),
        boot_command: Some("npm run dev".into()),
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["mode"], "boot");
    assert_eq!(json["boot_port"], 3000);
    assert_eq!(json["boot_command"], "npm run dev");
    assert_stable(&p);
}

#[test]
fn view_payload_boot_port_u16_boundaries() {
    for port in [0u16, 1, 8080, u16::MAX] {
        let p = ViewSessionPayload {
            session_id: "v".into(),
            status: ViewStatus::Open,
            run_slug: "run".into(),
            scope: ViewScope::Run,
            artifacts: vec![],
            factory: None,
            station: None,
            artifact: None,
            mode: ViewMode::Boot,
            boot_port: Some(port),
            boot_command: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["boot_port"], json!(port));
        assert_stable(&p);
    }
}

#[test]
fn view_payload_boot_port_rejects_over_u16() {
    // boot_port is u16 — 65536 overflows and must not deserialize.
    let json = json!({
        "session_id": "v",
        "status": "open",
        "run_slug": "run",
        "mode": "boot",
        "boot_port": 65536
    });
    let parsed: Result<ViewSessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err(), "port over u16 max must fail");
}

#[test]
fn view_payload_closed_status() {
    let p = ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Closed,
        run_slug: "run".into(),
        scope: ViewScope::Run,
        artifacts: vec![],
        factory: None,
        station: None,
        artifact: None,
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    };
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["status"], "closed");
    assert_stable(&p);
}

#[test]
fn view_payload_run_slug_is_mandatory() {
    // Unlike other payloads, view's run_slug is a non-optional String.
    let parsed: Result<ViewSessionPayload, _> = serde_json::from_value(json!({
        "session_id": "v",
        "status": "open",
        "mode": "viewer"
    }));
    assert!(parsed.is_err(), "missing run_slug must fail");
}

// -----------------------------------------------------------------------------
// DirectionSelectRequest / PickerSelectRequest — the decision request bodies
// -----------------------------------------------------------------------------

#[test]
fn direction_select_request_minimal() {
    let r = DirectionSelectRequest {
        archetype: "brutalist".into(),
        annotations: None,
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["archetype"], "brutalist");
    assert!(json.get("annotations").is_none());
    assert_stable(&r);
}

#[test]
fn direction_select_request_with_annotations() {
    let r = DirectionSelectRequest {
        archetype: "x".into(),
        annotations: Some(DirectionAnnotations {
            pins: vec![DirectionPin {
                x: 0.1,
                y: 0.2,
                note: "note".into(),
            }],
            screenshot: Some("data:image/png;base64,AA".into()),
            comments: vec!["nice".into()],
        }),
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["archetype"], "x");
    assert_eq!(json["annotations"]["pins"][0]["note"], "note");
    assert_eq!(json["annotations"]["screenshot"], "data:image/png;base64,AA");
    assert_stable(&r);
}

#[test]
fn direction_select_request_deserializes_from_text() {
    let r: DirectionSelectRequest =
        serde_json::from_value(json!({ "archetype": "bold" })).unwrap();
    assert_eq!(r.archetype, "bold");
    assert!(r.annotations.is_none());
}

#[test]
fn direction_select_request_missing_archetype_rejected() {
    let parsed: Result<DirectionSelectRequest, _> =
        serde_json::from_value(json!({ "annotations": {} }));
    assert!(parsed.is_err(), "archetype is required");
}

#[test]
fn picker_select_request_roundtrips() {
    let r = PickerSelectRequest { id: "software".into() };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["id"], "software");
    assert_stable(&r);
}

#[test]
fn picker_select_request_missing_id_rejected() {
    let parsed: Result<PickerSelectRequest, _> = serde_json::from_value(json!({}));
    assert!(parsed.is_err(), "id is required");
}

// -----------------------------------------------------------------------------
// Determinism & idempotency
// -----------------------------------------------------------------------------

#[test]
fn serialization_is_deterministic_across_runs() {
    let p = review(ReviewSessionPayload {
        session_id: "det".into(),
        gate_type: Some(GateType::Ask),
        approve_action: Some(ApproveAction {
            label: "Go".into(),
            kind: ApproveActionKind::Approve,
        }),
        ..Default::default()
    });
    let a = serde_json::to_string(&p).unwrap();
    let b = serde_json::to_string(&p).unwrap();
    let c = serde_json::to_string(&p).unwrap();
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn btreemap_keys_serialize_in_sorted_order() {
    let mut summaries = BTreeMap::new();
    summaries.insert("zebra".to_string(), "z".to_string());
    summaries.insert("alpha".to_string(), "a".to_string());
    summaries.insert("mike".to_string(), "m".to_string());
    let p = review(ReviewSessionPayload {
        session_id: "ord".into(),
        station_summaries: summaries,
        ..Default::default()
    });
    let text = serde_json::to_string(&p).unwrap();
    let alpha = text.find("alpha").unwrap();
    let mike = text.find("mike").unwrap();
    let zebra = text.find("zebra").unwrap();
    assert!(alpha < mike && mike < zebra, "BTreeMap must emit sorted keys");
}

#[test]
fn pretty_and_compact_json_deserialize_equally() {
    let p = SessionPayload::Picker(sample_picker("eq"));
    let compact = serde_json::to_string(&p).unwrap();
    let pretty = serde_json::to_string_pretty(&p).unwrap();
    let from_compact: SessionPayload = serde_json::from_str(&compact).unwrap();
    let from_pretty: SessionPayload = serde_json::from_str(&pretty).unwrap();
    assert_eq!(
        serde_json::to_value(&from_compact).unwrap(),
        serde_json::to_value(&from_pretty).unwrap()
    );
}

// -----------------------------------------------------------------------------
// Float coordinate fidelity (pins)
// -----------------------------------------------------------------------------

#[test]
fn pin_float_coordinates_survive_roundtrip() {
    for (x, y) in [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0), (0.123456789, 0.987654321)] {
        let pin = darkrun_api::common::Pin {
            x,
            y,
            text: "p".into(),
        };
        let json = serde_json::to_value(&pin).unwrap();
        let back: darkrun_api::common::Pin = serde_json::from_value(json).unwrap();
        assert_eq!(back.x, x);
        assert_eq!(back.y, y);
    }
}

#[test]
fn direction_pin_negative_and_large_floats() {
    for (x, y) in [(-1.0, -2.0), (1000.5, 2000.25)] {
        let pin = DirectionPin {
            x,
            y,
            note: "p".into(),
        };
        assert_stable(&pin);
    }
}

// -----------------------------------------------------------------------------
// Unicode & special characters in string fields
// -----------------------------------------------------------------------------

#[test]
fn unicode_strings_survive_roundtrip() {
    let p = review(ReviewSessionPayload {
        session_id: "u-✓".into(),
        feedback: Some("café — naïve \"quoted\" \n newline \t tab".into()),
        mermaid: Some("graph TD;\n  A-->B".into()),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_id"], "u-✓");
    assert!(json["feedback"].as_str().unwrap().contains("café"));
    assert_stable(&p);
}

#[test]
fn empty_string_fields_serialize_not_omitted() {
    // Mandatory String fields serialize even when empty; only Option/Vec/Map
    // collapse under their skip guards.
    let p = review(ReviewSessionPayload {
        session_id: "".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_id"], "");
}

// -----------------------------------------------------------------------------
// Cross-variant negative: payload of wrong shape under a tag
// -----------------------------------------------------------------------------

#[test]
fn picker_tag_with_review_body_fails() {
    // session_type=picker but missing the picker-required kind/title/prompt.
    let json = json!({
        "session_type": "picker",
        "session_id": "x",
        "status": "pending"
    });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err(), "picker needs kind/title/prompt/options");
}

#[test]
fn view_tag_with_missing_mode_fails() {
    let json = json!({
        "session_type": "view",
        "session_id": "x",
        "status": "open",
        "run_slug": "run"
    });
    let parsed: Result<SessionPayload, _> = serde_json::from_value(json);
    assert!(parsed.is_err(), "view needs mode");
}

#[test]
fn review_tag_tolerates_unknown_extra_fields() {
    // Review payload has no deny_unknown_fields, so extra keys are ignored.
    let json = json!({
        "session_type": "review",
        "session_id": "x",
        "status": "pending",
        "totally_made_up_field": 123
    });
    let parsed: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.session_type(), "review");
    assert_eq!(parsed.session_id(), "x");
}

// -----------------------------------------------------------------------------
// Wire-string parsing from raw JSON text (not just Value)
// -----------------------------------------------------------------------------

#[test]
fn review_payload_parses_from_raw_text() {
    let text = r#"{
        "session_type": "review",
        "session_id": "rt",
        "status": "approved",
        "gate_type": "ask",
        "await_active": true,
        "await_count": 5
    }"#;
    let p: SessionPayload = serde_json::from_str(text).unwrap();
    match p {
        SessionPayload::Review(r) => {
            assert_eq!(r.session_id, "rt");
            assert_eq!(r.status, SessionStatus::Approved);
            assert_eq!(r.gate_type, Some(GateType::Ask));
            assert_eq!(r.await_active, Some(true));
            assert_eq!(r.await_count, Some(5));
        }
        _ => panic!("expected review"),
    }
}

#[test]
fn picker_payload_parses_from_raw_text() {
    let text = r#"{
        "session_type": "picker",
        "session_id": "pk",
        "status": "pending",
        "kind": "station",
        "title": "Pick a station",
        "prompt": "Which station?",
        "options": [
            { "id": "frame", "label": "Frame" },
            { "id": "build", "label": "Build", "secondary": true }
        ]
    }"#;
    let p: SessionPayload = serde_json::from_str(text).unwrap();
    match p {
        SessionPayload::Picker(pk) => {
            assert_eq!(pk.kind, PickerKind::Station);
            assert_eq!(pk.options.len(), 2);
            assert_eq!(pk.options[1].secondary, Some(true));
            assert_eq!(pk.options[0].secondary, None);
        }
        _ => panic!("expected picker"),
    }
}

#[test]
fn malformed_json_text_is_rejected() {
    let text = r#"{ "session_type": "review", "session_id": "#;
    let parsed: Result<SessionPayload, _> = serde_json::from_str(text);
    assert!(parsed.is_err());
}

// =============================================================================
// View artifact browser — the real `view` payload (artifacts + scope)
// =============================================================================

#[test]
fn view_artifact_browser_lists_artifacts_with_kinds() {
    let p = ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Station,
        artifacts: vec![
            ViewArtifact {
                id: "a1".into(),
                path: "build/index.html".into(),
                kind: ViewArtifactKind::File,
                label: "index.html".into(),
                thumbnail_url: None,
                url: Some("/view/v/a1".into()),
            },
            ViewArtifact {
                id: "a2".into(),
                path: "build/home.png".into(),
                kind: ViewArtifactKind::Screenshot,
                label: "Home screenshot".into(),
                thumbnail_url: Some("/thumb/a2.png".into()),
                url: Some("/view/v/a2".into()),
            },
        ],
        factory: None,
        station: Some("build".into()),
        artifact: Some("a2".into()),
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    };
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["scope"], "station");
    assert_eq!(json["artifacts"][0]["id"], "a1");
    assert_eq!(json["artifacts"][0]["kind"], "file");
    assert_eq!(json["artifacts"][1]["kind"], "screenshot");
    assert_eq!(json["artifacts"][1]["thumbnail_url"], "/thumb/a2.png");

    let back: ViewSessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.artifacts.len(), 2);
    assert_eq!(back.artifact_by_id("a2").unwrap().label, "Home screenshot");
    assert!(back.artifact_by_id("ghost").is_none());
}

#[test]
fn view_artifact_kind_tokens_are_snake_case() {
    for (k, s) in [
        (ViewArtifactKind::File, "file"),
        (ViewArtifactKind::Image, "image"),
        (ViewArtifactKind::Screenshot, "screenshot"),
        (ViewArtifactKind::Markdown, "markdown"),
        (ViewArtifactKind::Json, "json"),
    ] {
        assert_eq!(serde_json::to_value(k).unwrap(), json!(s));
    }
}

#[test]
fn view_scope_defaults_to_run_when_absent() {
    // Legacy payloads omitting `scope` still parse, defaulting to `run`.
    let back: ViewSessionPayload = serde_json::from_value(json!({
        "session_id": "v",
        "status": "open",
        "run_slug": "run",
        "mode": "viewer"
    }))
    .unwrap();
    assert_eq!(back.scope, ViewScope::Run);
    assert!(back.artifacts.is_empty());
}

#[test]
fn view_payload_through_union_carries_artifacts() {
    let p = SessionPayload::View(ViewSessionPayload {
        session_id: "v".into(),
        status: ViewStatus::Open,
        run_slug: "run".into(),
        scope: ViewScope::Run,
        artifacts: vec![ViewArtifact {
            id: "md".into(),
            path: "README.md".into(),
            kind: ViewArtifactKind::Markdown,
            label: "Readme".into(),
            thumbnail_url: None,
            url: None,
        }],
        factory: None,
        station: None,
        artifact: None,
        mode: ViewMode::Viewer,
        boot_port: None,
        boot_command: None,
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "view");
    assert_eq!(json["artifacts"][0]["kind"], "markdown");
    let back: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.session_type(), "view");
}

// =============================================================================
// Visual-review payload — pin annotations over an output screenshot
// =============================================================================

#[test]
fn visual_review_payload_roundtrips() {
    let p = VisualReviewSessionPayload {
        session_id: "vr".into(),
        status: SessionStatus::Pending,
        run_slug: Some("run".into()),
        station: Some("build".into()),
        artifact_id: Some("a2".into()),
        artifact_path: Some("build/home.png".into()),
        screenshot_url: Some("/shot/home.png".into()),
        prompt: Some("Review the home page".into()),
        annotations: Some(VisualReviewAnnotations {
            pins: vec![VisualReviewPin {
                x: 0.1,
                y: 0.2,
                note: "spacing".into(),
            }],
            comments: vec!["looks off".into()],
        }),
    };
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["artifact_id"], "a2");
    assert_eq!(json["annotations"]["pins"][0]["note"], "spacing");
    let back: VisualReviewSessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.annotations.unwrap().comments[0], "looks off");
}

#[test]
fn visual_review_through_union_tags_correctly() {
    let p = SessionPayload::VisualReview(VisualReviewSessionPayload {
        session_id: "vr".into(),
        ..Default::default()
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "visual_review");
    let back: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.session_type(), "visual_review");
    assert_eq!(back.session_id(), "vr");
}

#[test]
fn visual_review_annotations_emptiness() {
    assert!(VisualReviewAnnotations::default().is_empty());
    assert!(!VisualReviewAnnotations {
        comments: vec!["x".into()],
        ..Default::default()
    }
    .is_empty());
}

// =============================================================================
// Proof payload — the Prove station's objective evidence
// =============================================================================

#[test]
fn proof_payload_web_through_union() {
    let mut vitals = BTreeMap::new();
    vitals.insert("lcp".to_string(), 980.0);
    let p = SessionPayload::Proof(ProofSessionPayload {
        session_id: "pf".into(),
        status: SessionStatus::Pending,
        run_slug: Some("run".into()),
        station: Some("prove".into()),
        proof: Proof::web(
            Surface::WebUi,
            WebProof {
                vitals,
                audits: vec![AuditResult {
                    name: "reduced-motion".into(),
                    value: "ok".into(),
                    pass: true,
                }],
                screenshot_url: Some("/shot.png".into()),
            },
        ),
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["session_type"], "proof");
    assert_eq!(json["proof"]["surface"], "web_ui");
    assert_eq!(json["proof"]["web"]["vitals"]["lcp"], 980.0);
    let back: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.session_type(), "proof");
}

#[test]
fn proof_payload_bench_through_union() {
    let p = SessionPayload::Proof(ProofSessionPayload {
        session_id: "pf".into(),
        status: SessionStatus::Pending,
        run_slug: Some("run".into()),
        station: None,
        proof: Proof::bench(
            Surface::Data,
            BenchProof {
                p50: Some(0.3),
                p95: Some(0.9),
                p99: Some(1.5),
                throughput: Some(12000.0),
                samples: Some(500),
            },
        ),
    });
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["proof"]["surface"], "data");
    assert_eq!(json["proof"]["bench"]["throughput"], 12000.0);
    assert!(json["proof"].get("web").is_none());
    let back: SessionPayload = serde_json::from_value(json).unwrap();
    assert_eq!(back.session_type(), "proof");
}
