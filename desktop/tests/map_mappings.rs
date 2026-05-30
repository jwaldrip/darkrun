//! Boundary-mapping tests: wire enums -> UI kinds.
//!
//! Covers every `RunPhase`, `GateType`, and `SessionStatus` variant plus the
//! free-form `label_tone` token table.

use darkrun_api::common::{GateType, SessionStatus};
use darkrun_api::session::RunPhase;
use darkrun_api::{RunSummary, StationProgress};
use darkrun_desktop::map::{checkpoint_kind, label_tone, phase, run_card, status_tone};
use darkrun_ui::components::factory::CheckpointKind;
use darkrun_ui::kinds::{Phase, Tone};

// ---- phase(): all six variants ----

#[test]
fn phase_spec() {
    assert_eq!(phase(RunPhase::Spec), Phase::Spec);
}

#[test]
fn phase_review() {
    assert_eq!(phase(RunPhase::Review), Phase::Review);
}

#[test]
fn phase_manufacture() {
    assert_eq!(phase(RunPhase::Manufacture), Phase::Manufacture);
}

#[test]
fn phase_audit() {
    assert_eq!(phase(RunPhase::Audit), Phase::Audit);
}

#[test]
fn phase_reflect() {
    assert_eq!(phase(RunPhase::Reflect), Phase::Reflect);
}

#[test]
fn phase_checkpoint() {
    assert_eq!(phase(RunPhase::Checkpoint), Phase::Checkpoint);
}

#[test]
fn phase_is_total_and_distinct() {
    // Every wire phase maps to a distinct UI phase (no collapsing).
    let all = [
        phase(RunPhase::Spec),
        phase(RunPhase::Review),
        phase(RunPhase::Manufacture),
        phase(RunPhase::Audit),
        phase(RunPhase::Reflect),
        phase(RunPhase::Checkpoint),
    ];
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "phases {i} and {j} collapsed");
            }
        }
    }
}

// ---- checkpoint_kind(): all four gate types ----

#[test]
fn gate_auto() {
    assert_eq!(checkpoint_kind(GateType::Auto), CheckpointKind::Auto);
}

#[test]
fn gate_ask() {
    assert_eq!(checkpoint_kind(GateType::Ask), CheckpointKind::Ask);
}

#[test]
fn gate_external() {
    assert_eq!(checkpoint_kind(GateType::External), CheckpointKind::External);
}

#[test]
fn gate_await() {
    assert_eq!(checkpoint_kind(GateType::Await), CheckpointKind::Await);
}

#[test]
fn gate_kinds_distinct() {
    let all = [
        checkpoint_kind(GateType::Auto),
        checkpoint_kind(GateType::Ask),
        checkpoint_kind(GateType::External),
        checkpoint_kind(GateType::Await),
    ];
    for (i, a) in all.iter().enumerate() {
        for (j, b) in all.iter().enumerate() {
            if i != j {
                assert_ne!(a, b);
            }
        }
    }
}

// ---- status_tone(): all five lifecycle statuses ----

#[test]
fn status_pending_warns() {
    assert_eq!(status_tone(SessionStatus::Pending), Tone::Warn);
}

#[test]
fn status_decided_info() {
    assert_eq!(status_tone(SessionStatus::Decided), Tone::Info);
}

#[test]
fn status_answered_info() {
    assert_eq!(status_tone(SessionStatus::Answered), Tone::Info);
}

#[test]
fn status_approved_ok() {
    assert_eq!(status_tone(SessionStatus::Approved), Tone::Ok);
}

#[test]
fn status_changes_requested_danger() {
    assert_eq!(status_tone(SessionStatus::ChangesRequested), Tone::Danger);
}

#[test]
fn status_decided_and_answered_share_tone() {
    assert_eq!(
        status_tone(SessionStatus::Decided),
        status_tone(SessionStatus::Answered)
    );
}

// ---- label_tone(): the free-form token table ----

#[test]
fn label_ok_tokens() {
    for t in ["done", "complete", "completed", "merged", "passed", "approved"] {
        assert_eq!(label_tone(t), Tone::Ok, "token {t}");
    }
}

#[test]
fn label_info_tokens() {
    for t in [
        "active",
        "in_progress",
        "in-progress",
        "running",
        "manufacturing",
    ] {
        assert_eq!(label_tone(t), Tone::Info, "token {t}");
    }
}

#[test]
fn label_danger_tokens() {
    for t in ["blocked", "failed", "error", "rejected", "changes_requested"] {
        assert_eq!(label_tone(t), Tone::Danger, "token {t}");
    }
}

#[test]
fn label_warn_tokens() {
    for t in ["pending", "queued", "waiting", "review"] {
        assert_eq!(label_tone(t), Tone::Warn, "token {t}");
    }
}

#[test]
fn label_unknown_is_neutral() {
    for t in ["", "weird", "xyzzy", "42", "n/a"] {
        assert_eq!(label_tone(t), Tone::Neutral, "token {t:?}");
    }
}

#[test]
fn label_tone_is_case_insensitive() {
    assert_eq!(label_tone("DONE"), Tone::Ok);
    assert_eq!(label_tone("Active"), Tone::Info);
    assert_eq!(label_tone("FAILED"), Tone::Danger);
    assert_eq!(label_tone("Pending"), Tone::Warn);
    assert_eq!(label_tone("MeRgEd"), Tone::Ok);
}

#[test]
fn label_tone_trims_whitespace() {
    assert_eq!(label_tone("  done  "), Tone::Ok);
    assert_eq!(label_tone("\tactive\n"), Tone::Info);
    assert_eq!(label_tone(" failed"), Tone::Danger);
    assert_eq!(label_tone("review  "), Tone::Warn);
}

#[test]
fn label_tone_internal_space_not_trimmed_to_token() {
    // Trimming is outer-only; internal spacing makes it an unknown token.
    assert_eq!(label_tone("in progress"), Tone::Neutral);
    assert_eq!(label_tone("not done"), Tone::Neutral);
}

#[test]
fn label_tone_mixed_case_with_padding() {
    assert_eq!(label_tone("  ChAnGeS_ReQuEsTeD "), Tone::Danger);
}

// ---- run_card(): RunSummary -> RunCardData ----

fn summary() -> RunSummary {
    RunSummary {
        slug: "rate-limit".into(),
        title: "Rate limit the public API".into(),
        factory: "software".into(),
        active_station: "build".into(),
        phase: Some("manufacture".into()),
        status: "active".into(),
        progress: StationProgress { completed: 3, total: 6 },
        started_at: Some("2026-05-30T00:00:00Z".into()),
    }
}

#[test]
fn run_card_carries_identity_and_progress() {
    let card = run_card(&summary());
    assert_eq!(card.slug, "rate-limit");
    assert_eq!(card.title, "Rate limit the public API");
    assert_eq!(card.factory, "software");
    assert_eq!(card.active_station, "build");
    assert_eq!(card.status, "active");
    assert_eq!(card.completed, 3);
    assert_eq!(card.total, 6);
}

#[test]
fn run_card_parses_known_phase_string() {
    assert_eq!(run_card(&summary()).phase, Some(Phase::Manufacture));
}

#[test]
fn run_card_unknown_or_absent_phase_is_none() {
    let mut s = summary();
    s.phase = None;
    assert_eq!(run_card(&s).phase, None);
    s.phase = Some("between-stations".into());
    assert_eq!(run_card(&s).phase, None);
}
