//! Comprehensive coverage of the `frontmatter` module's public surface:
//! `split`, `parse`, `serialize`, `first_heading`, and the `Document` type.
//!
//! Exercises the awkward inputs (BOM, CRLF, unterminated fences, trailing
//! fences with no newline, empty bodies, body-only docs, inline dashes,
//! nested YAML, lists, quotes, unicode) and round-trips every domain
//! frontmatter type through the YAML envelope the module produces.
//!
//! The tests deliberately construct a `default()` value and then mutate one
//! field at a time so each case isolates a single attribute; that pattern
//! trips `clippy::field_reassign_with_default`, which we allow here.
#![allow(clippy::field_reassign_with_default)]

use darkrun_core::domain::{
    Checkpoint, CheckpointKind, CheckpointOutcome, Explorer, Feedback,
    FeedbackSeverity, FeedbackStatus, Mode, Pass, PassBeat, Reviewer, RunFrontmatter, RunGit,
    Status, Station, StationPhase, UnitFrontmatter, Worker,
};
use darkrun_core::error::CoreError;
use darkrun_core::frontmatter::{self, Document};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Build a minimal valid RunFrontmatter (factory is the one required field).
fn run_fm() -> RunFrontmatter {
    RunFrontmatter {
        factory: "software".into(),
        ..Default::default()
    }
}

/// Parse must succeed; return the typed frontmatter + body.
fn parse_run(raw: &str) -> (RunFrontmatter, String) {
    frontmatter::parse::<RunFrontmatter>(raw).expect("parse run frontmatter")
}

// ===========================================================================
// split — fence detection
// ===========================================================================

#[test]
fn split_extracts_frontmatter_and_body() {
    let doc = frontmatter::split("---\nfactory: software\n---\n# Title\nbody\n");
    assert_eq!(doc.frontmatter, "factory: software\n");
    assert_eq!(doc.body, "# Title\nbody\n");
}

#[test]
fn split_frontmatter_keeps_trailing_newline() {
    // The captured frontmatter retains the newline of its last line.
    let doc = frontmatter::split("---\na: 1\nb: 2\n---\nbody\n");
    assert_eq!(doc.frontmatter, "a: 1\nb: 2\n");
}

#[test]
fn split_multiline_frontmatter_preserved_verbatim() {
    let doc = frontmatter::split("---\nfactory: software\nmode: continuous\n---\nbody\n");
    assert_eq!(doc.frontmatter, "factory: software\nmode: continuous\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_body_only_has_empty_frontmatter() {
    let doc = frontmatter::split("# Just a body\nno fence\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "# Just a body\nno fence\n");
}

#[test]
fn split_body_only_single_line() {
    let doc = frontmatter::split("just one line");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "just one line");
}

#[test]
fn split_empty_string() {
    let doc = frontmatter::split("");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "");
}

#[test]
fn split_only_newline_is_body() {
    let doc = frontmatter::split("\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "\n");
}

#[test]
fn split_only_whitespace_is_body() {
    let doc = frontmatter::split("   \n  \n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "   \n  \n");
}

#[test]
fn split_empty_frontmatter_block() {
    // Immediate closing fence -> empty YAML, body follows.
    let doc = frontmatter::split("---\n---\nbody only\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "body only\n");
}

#[test]
fn split_empty_frontmatter_block_empty_body() {
    let doc = frontmatter::split("---\n---\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "");
}

#[test]
fn split_handles_trailing_fence_without_newline() {
    // Closing `---` at EOF with no trailing newline -> empty body.
    let doc = frontmatter::split("---\nfactory: software\n---");
    assert_eq!(doc.frontmatter, "factory: software\n");
    assert_eq!(doc.body, "");
}

#[test]
fn split_trailing_fence_immediately_after_open_no_newline() {
    // `---\n---` (no trailing newline): open then immediate close at EOF.
    let doc = frontmatter::split("---\n---");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "");
}

#[test]
fn split_unterminated_fence_falls_back_to_body() {
    let doc = frontmatter::split("---\nfactory: software\nno close here\n");
    assert_eq!(doc.frontmatter, "");
    assert!(doc.body.contains("factory: software"));
    assert!(doc.body.contains("no close here"));
}

#[test]
fn split_unterminated_fence_keeps_open_fence_in_body() {
    // When the fence never closes, the whole normalized doc is the body,
    // including the opening `---`.
    let doc = frontmatter::split("---\nstuff\n");
    assert_eq!(doc.frontmatter, "");
    assert!(doc.body.starts_with("---\n"));
}

#[test]
fn split_lone_open_fence_no_newline_is_body() {
    // `---` with no newline after it does not match the `---\n` open prefix.
    let doc = frontmatter::split("---");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "---");
}

#[test]
fn split_does_not_treat_inline_dashes_as_fence() {
    // A `---` that is not on its own line must not close the block.
    let doc = frontmatter::split("---\nnote: a --- b\n---\nbody\n");
    assert_eq!(doc.frontmatter, "note: a --- b\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_does_not_treat_four_dashes_as_fence() {
    // `----` is not exactly `---`, so it does not close.
    let doc = frontmatter::split("---\nx: 1\n----\nstill fm\n---\nbody\n");
    assert_eq!(doc.frontmatter, "x: 1\n----\nstill fm\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_does_not_treat_indented_dashes_as_fence() {
    // An indented `  ---` is not exactly `---` on its own line.
    let doc = frontmatter::split("---\nx: 1\n  ---\n---\nbody\n");
    assert_eq!(doc.frontmatter, "x: 1\n  ---\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_first_closing_fence_wins() {
    // The body itself may contain `---`; only the first closer is honored.
    let doc = frontmatter::split("---\nx: 1\n---\nbody\n---\nmore\n");
    assert_eq!(doc.frontmatter, "x: 1\n");
    assert_eq!(doc.body, "body\n---\nmore\n");
}

#[test]
fn split_closing_fence_with_trailing_spaces_is_not_fence() {
    // `--- ` (trailing space) != `---`.
    let doc = frontmatter::split("---\nx: 1\n--- \n---\nbody\n");
    assert_eq!(doc.frontmatter, "x: 1\n--- \n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_body_with_no_trailing_newline() {
    let doc = frontmatter::split("---\nx: 1\n---\nbody no newline");
    assert_eq!(doc.frontmatter, "x: 1\n");
    assert_eq!(doc.body, "body no newline");
}

#[test]
fn split_body_can_be_multiparagraph() {
    let doc = frontmatter::split("---\nx: 1\n---\npara one\n\npara two\n");
    assert_eq!(doc.body, "para one\n\npara two\n");
}

// ---- BOM handling -------------------------------------------------------

#[test]
fn split_strips_leading_bom() {
    let doc = frontmatter::split("\u{feff}---\nfactory: software\n---\nbody\n");
    assert_eq!(doc.frontmatter, "factory: software\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_strips_bom_before_body_only() {
    let doc = frontmatter::split("\u{feff}plain body\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "plain body\n");
}

#[test]
fn split_only_bom_is_empty() {
    let doc = frontmatter::split("\u{feff}");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "");
}

#[test]
fn split_bom_inside_body_not_stripped() {
    // Only a *leading* BOM is stripped; one inside the body is preserved.
    let doc = frontmatter::split("body \u{feff} mid\n");
    assert!(doc.body.contains('\u{feff}'));
}

// ---- CRLF normalization -------------------------------------------------

#[test]
fn split_normalizes_crlf() {
    let doc = frontmatter::split("---\r\nfactory: software\r\n---\r\nbody\r\n");
    assert_eq!(doc.frontmatter, "factory: software\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_crlf_body_only() {
    let doc = frontmatter::split("line one\r\nline two\r\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "line one\nline two\n");
}

#[test]
fn split_crlf_no_lf_remains() {
    let doc = frontmatter::split("---\r\nx: 1\r\n---\r\na\r\nb\r\n");
    assert!(!doc.frontmatter.contains('\r'));
    assert!(!doc.body.contains('\r'));
}

#[test]
fn split_mixed_crlf_and_lf() {
    // CRLF normalized to LF; bare LF left intact.
    let doc = frontmatter::split("---\r\nx: 1\n---\r\nbody\n");
    assert_eq!(doc.frontmatter, "x: 1\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_bom_then_crlf() {
    let doc = frontmatter::split("\u{feff}---\r\nfactory: software\r\n---\r\nbody\r\n");
    assert_eq!(doc.frontmatter, "factory: software\n");
    assert_eq!(doc.body, "body\n");
}

// ---- Document type behaviors -------------------------------------------

#[test]
fn document_equality_same_inputs() {
    let a = frontmatter::split("---\nfactory: x\n---\nbody\n");
    let b = frontmatter::split("---\nfactory: x\n---\nbody\n");
    assert_eq!(a, b);
}

#[test]
fn document_inequality_different_body() {
    let a = frontmatter::split("---\nfactory: x\n---\nbody a\n");
    let b = frontmatter::split("---\nfactory: x\n---\nbody b\n");
    assert_ne!(a, b);
}

#[test]
fn document_inequality_different_frontmatter() {
    let a = frontmatter::split("---\nfactory: x\n---\nbody\n");
    let b = frontmatter::split("---\nfactory: y\n---\nbody\n");
    assert_ne!(a, b);
}

#[test]
fn document_clone_equals_original() {
    let a = frontmatter::split("---\nfactory: x\n---\nbody\n");
    let cloned = a.clone();
    assert_eq!(a, cloned);
}

#[test]
fn document_can_be_constructed_directly() {
    let d = Document {
        frontmatter: "k: v\n".into(),
        body: "hi\n".into(),
    };
    assert_eq!(d.frontmatter, "k: v\n");
    assert_eq!(d.body, "hi\n");
}

#[test]
fn document_debug_is_nonempty() {
    let d = frontmatter::split("---\nfactory: x\n---\nbody\n");
    let s = format!("{d:?}");
    assert!(s.contains("Document"));
    assert!(s.contains("frontmatter"));
}

// ===========================================================================
// parse — typed deserialization + error paths
// ===========================================================================

#[test]
fn parse_typed_roundtrip() {
    let fm = RunFrontmatter {
        title: Some("Ship it".into()),
        factory: "software".into(),
        active_station: "frame".into(),
        ..Default::default()
    };
    let doc = frontmatter::serialize(&fm, "# Ship it\n\nbody\n").expect("ser");
    let (parsed, body) = parse_run(&doc);
    assert_eq!(parsed.title.as_deref(), Some("Ship it"));
    assert_eq!(parsed.factory, "software");
    assert_eq!(parsed.active_station, "frame");
    assert!(body.contains("body"));
}

#[test]
fn parse_returns_body_after_fence() {
    let (_, body) = parse_run("---\nfactory: software\n---\n# H\nline\n");
    assert_eq!(body, "# H\nline\n");
}

#[test]
fn parse_empty_body_is_empty_string() {
    let (_, body) = parse_run("---\nfactory: software\n---\n");
    assert_eq!(body, "");
}

#[test]
fn parse_missing_frontmatter_errors() {
    let err = frontmatter::parse::<RunFrontmatter>("plain body, no fence").unwrap_err();
    assert!(matches!(err, CoreError::MissingFrontmatter));
}

#[test]
fn parse_missing_frontmatter_on_empty_string() {
    let err = frontmatter::parse::<RunFrontmatter>("").unwrap_err();
    assert!(matches!(err, CoreError::MissingFrontmatter));
}

#[test]
fn parse_missing_frontmatter_on_heading_only() {
    let err = frontmatter::parse::<RunFrontmatter>("# Title only\n").unwrap_err();
    assert!(matches!(err, CoreError::MissingFrontmatter));
}

#[test]
fn parse_leading_whitespace_before_fence_still_missing() {
    // The fence must be at the very start (after BOM/CRLF normalization).
    // A doc that only *contains* `---` later is body-only -> MissingFrontmatter.
    let err = frontmatter::parse::<RunFrontmatter>("text\n---\nfactory: x\n---\n").unwrap_err();
    assert!(matches!(err, CoreError::MissingFrontmatter));
}

#[test]
fn parse_unterminated_fence_is_missing_frontmatter() {
    // The raw starts with `---`, but split finds no closer so frontmatter is
    // empty; parse then sees `raw.trim_start().starts_with("---")` is true,
    // so it tries to deserialize an empty YAML -> a Yaml error for a required
    // field rather than MissingFrontmatter. Confirm it's a Yaml error.
    let err =
        frontmatter::parse::<RunFrontmatter>("---\nfactory: software\nno close\n").unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_malformed_yaml_errors() {
    let raw = "---\nfactory: [unterminated\n---\nbody\n";
    let err = frontmatter::parse::<RunFrontmatter>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_missing_required_field_errors() {
    // `factory` has no serde default.
    let raw = "---\nmode: continuous\n---\nbody\n";
    let err = frontmatter::parse::<RunFrontmatter>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_empty_frontmatter_block_errors_for_required_field() {
    // `---\n---\n` yields empty YAML -> required `factory` missing.
    let err = frontmatter::parse::<RunFrontmatter>("---\n---\nbody\n").unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_wrong_type_for_field_errors() {
    // `factory` is a String; a mapping value should be rejected.
    let raw = "---\nfactory:\n  nested: true\n---\nbody\n";
    let err = frontmatter::parse::<RunFrontmatter>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_unknown_status_value_errors() {
    let raw = "---\nfactory: software\nstatus: exploded\n---\nbody\n";
    let err = frontmatter::parse::<RunFrontmatter>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_fills_defaults_for_absent_optional_fields() {
    let (fm, _) = parse_run("---\nfactory: software\n---\nbody\n");
    assert_eq!(fm.factory, "software");
    assert_eq!(fm.mode, Mode::Solo);
    assert_eq!(fm.active_station, "");
    assert_eq!(fm.status, Status::Pending);
    assert!(fm.title.is_none());
    assert!(fm.archived.is_none());
    assert!(fm.git.is_none());
}

#[test]
fn parse_crlf_document() {
    let (fm, body) = parse_run("---\r\nfactory: software\r\n---\r\nbody\r\n");
    assert_eq!(fm.factory, "software");
    assert_eq!(body, "body\n");
}

#[test]
fn parse_bom_document() {
    let (fm, _) = parse_run("\u{feff}---\nfactory: software\n---\nbody\n");
    assert_eq!(fm.factory, "software");
}

#[test]
fn parse_status_each_variant() {
    for (tok, want) in [
        ("pending", Status::Pending),
        ("active", Status::Active),
        ("in_progress", Status::InProgress),
        ("completed", Status::Completed),
        ("blocked", Status::Blocked),
    ] {
        let raw = format!("---\nfactory: software\nstatus: {tok}\n---\nbody\n");
        let (fm, _) = parse_run(&raw);
        assert_eq!(fm.status, want, "token {tok}");
    }
}

// ---- nested YAML, lists, quotes, unicode through parse -------------------

#[test]
fn parse_nested_git_mapping() {
    let raw = "---\nfactory: software\ngit:\n  change_strategy: worktree-per-unit\n  auto_merge: true\n  auto_squash: false\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    let git = fm.git.expect("git present");
    assert_eq!(git.change_strategy, "worktree-per-unit");
    assert!(git.auto_merge);
    assert!(!git.auto_squash);
}

#[test]
fn parse_git_partial_mapping_uses_defaults() {
    let raw = "---\nfactory: software\ngit:\n  auto_merge: true\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    let git = fm.git.expect("git present");
    assert!(git.auto_merge);
    assert_eq!(git.change_strategy, "");
    assert!(!git.auto_squash);
}

#[test]
fn parse_unit_depends_on_list_block_style() {
    let raw = "---\nstatus: pending\ndepends_on:\n  - a\n  - b\n  - c\n---\nbody\n";
    let (fm, _) =
        frontmatter::parse::<UnitFrontmatter>(raw).expect("parse unit");
    assert_eq!(fm.depends_on, vec!["a", "b", "c"]);
}

#[test]
fn parse_unit_depends_on_list_flow_style() {
    let raw = "---\ndepends_on: [x, y, z]\n---\nbody\n";
    let (fm, _) =
        frontmatter::parse::<UnitFrontmatter>(raw).expect("parse unit");
    assert_eq!(fm.depends_on, vec!["x", "y", "z"]);
}

#[test]
fn parse_unit_empty_list_is_empty() {
    let raw = "---\ndepends_on: []\n---\nbody\n";
    let (fm, _) =
        frontmatter::parse::<UnitFrontmatter>(raw).expect("parse unit");
    assert!(fm.depends_on.is_empty());
}

#[test]
fn parse_unit_outputs_and_inputs_lists() {
    let raw = "---\ninputs:\n  - in/a.md\noutputs:\n  - out/b.md\n  - out/c.md\n---\nbody\n";
    let (fm, _) =
        frontmatter::parse::<UnitFrontmatter>(raw).expect("parse unit");
    assert_eq!(fm.inputs, vec!["in/a.md"]);
    assert_eq!(fm.outputs, vec!["out/b.md", "out/c.md"]);
}

#[test]
fn parse_double_quoted_title() {
    let raw = "---\nfactory: software\ntitle: \"Ship: the thing\"\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.title.as_deref(), Some("Ship: the thing"));
}

#[test]
fn parse_single_quoted_title() {
    let raw = "---\nfactory: software\ntitle: 'it''s mine'\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.title.as_deref(), Some("it's mine"));
}

#[test]
fn parse_quoted_value_with_special_chars() {
    let raw = "---\nfactory: \"a # b: c\"\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.factory, "a # b: c");
}

#[test]
fn parse_unicode_title() {
    let raw = "---\nfactory: software\ntitle: \"日本語 — café 🚀\"\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.title.as_deref(), Some("日本語 — café 🚀"));
}

#[test]
fn parse_unicode_factory_unquoted() {
    let raw = "---\nfactory: café\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.factory, "café");
}

#[test]
fn parse_yaml_multiline_block_scalar_in_body_field() {
    // serde_yaml literal block scalar `|` into a String field (active_station).
    let raw = "---\nfactory: software\nactive_station: |\n  line1\n  line2\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.active_station, "line1\nline2\n");
}

#[test]
fn parse_yaml_folded_block_scalar() {
    let raw = "---\nfactory: software\nactive_station: >\n  one\n  two\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    // Folded scalar joins lines with spaces, trailing newline.
    assert_eq!(fm.active_station, "one two\n");
}

#[test]
fn parse_archived_bool_true() {
    let (fm, _) = parse_run("---\nfactory: software\narchived: true\n---\nb\n");
    assert_eq!(fm.archived, Some(true));
}

#[test]
fn parse_archived_bool_false() {
    let (fm, _) = parse_run("---\nfactory: software\narchived: false\n---\nb\n");
    assert_eq!(fm.archived, Some(false));
}

#[test]
fn parse_timestamps_as_strings() {
    let raw = "---\nfactory: software\nstarted_at: \"2026-05-30T00:00:00Z\"\ncompleted_at: \"2026-05-31T00:00:00Z\"\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.started_at.as_deref(), Some("2026-05-30T00:00:00Z"));
    assert_eq!(fm.completed_at.as_deref(), Some("2026-05-31T00:00:00Z"));
}

#[test]
fn legacy_stored_pass_key_is_ignored() {
    // `pass` is no longer a stored field — it is derived from the iteration
    // array. A legacy document carrying `pass:` parses fine and the key is
    // simply dropped (no field to bind it to).
    let raw = "---\npass: 7\n---\nbody\n";
    let (_fm, _) =
        frontmatter::parse::<UnitFrontmatter>(raw).expect("parse tolerates legacy pass key");
}

// ===========================================================================
// serialize — envelope shape
// ===========================================================================

#[test]
fn serialize_emits_fences_and_separates_body() {
    let out = frontmatter::serialize(&run_fm(), "# Heading\n").expect("ser");
    assert!(out.starts_with("---\n"));
    assert!(out.contains("\n---\n"));
    assert!(out.contains("---\n\n# Heading"));
}

#[test]
fn serialize_starts_with_open_fence() {
    let out = frontmatter::serialize(&run_fm(), "body\n").expect("ser");
    assert!(out.starts_with("---\n"));
}

#[test]
fn serialize_contains_factory_value() {
    let out = frontmatter::serialize(&run_fm(), "").expect("ser");
    assert!(out.contains("factory: software"));
}

#[test]
fn serialize_empty_body_has_no_trailing_content() {
    let out = frontmatter::serialize(&run_fm(), "").expect("ser");
    assert!(out.ends_with("---\n"), "no body appended: {out:?}");
}

#[test]
fn serialize_empty_body_no_double_fence_newline() {
    let out = frontmatter::serialize(&run_fm(), "").expect("ser");
    // Exactly one close fence, no spurious blank line after it.
    assert!(!out.ends_with("---\n\n"));
}

#[test]
fn serialize_injects_blank_line_before_body() {
    let out = frontmatter::serialize(&run_fm(), "no leading newline\n").expect("ser");
    assert!(out.contains("---\n\nno leading newline\n"));
}

#[test]
fn serialize_preserves_body_already_starting_with_newline() {
    let out = frontmatter::serialize(&run_fm(), "\nalready spaced\n").expect("ser");
    assert!(out.contains("---\n\nalready spaced\n"));
    assert!(!out.contains("---\n\n\nalready spaced"));
}

#[test]
fn serialize_body_starting_with_two_newlines_kept() {
    let out = frontmatter::serialize(&run_fm(), "\n\nspaced\n").expect("ser");
    // body already starts with \n, so no extra injection -> three? No: fence
    // ends with `---\n`, body adds `\n\nspaced\n`.
    assert!(out.contains("---\n\n\nspaced\n"));
}

#[test]
fn serialize_skips_none_options() {
    // title/archived/started_at/etc. default to None and are skipped.
    let out = frontmatter::serialize(&run_fm(), "").expect("ser");
    assert!(!out.contains("title:"));
    assert!(!out.contains("archived:"));
    assert!(!out.contains("started_at:"));
    assert!(!out.contains("git:"));
}

#[test]
fn serialize_emits_set_options() {
    let mut fm = run_fm();
    fm.title = Some("T".into());
    fm.archived = Some(true);
    let out = frontmatter::serialize(&fm, "").expect("ser");
    assert!(out.contains("title: T"));
    assert!(out.contains("archived: true"));
}

#[test]
fn serialize_no_blank_line_inside_frontmatter() {
    // The YAML block between fences should have its trailing newline trimmed
    // and exactly one inserted before the close fence.
    let out = frontmatter::serialize(&run_fm(), "").expect("ser");
    assert!(!out.contains("\n\n---\n"), "stray blank line in fm: {out:?}");
}

#[test]
fn serialize_multiline_body_preserved() {
    let body = "# A\n\nline 1\nline 2\n\nline 3\n";
    let out = frontmatter::serialize(&run_fm(), body).expect("ser");
    assert!(out.ends_with(body));
}

#[test]
fn serialize_unicode_body_preserved() {
    let body = "# 日本語\n\ncafé 🚀\n";
    let out = frontmatter::serialize(&run_fm(), body).expect("ser");
    assert!(out.contains("café 🚀"));
    assert!(out.contains("# 日本語"));
}

// ===========================================================================
// roundtrip: serialize -> parse for RunFrontmatter (field by field)
// ===========================================================================

#[test]
fn roundtrip_run_full_frontmatter() {
    let fm = RunFrontmatter {
        title: Some("Ship the thing".into()),
        factory: "software".into(),
        mode: Mode::Solo,
        active_station: "frame".into(),
        status: Status::Active,
        surface: None,
        archived: Some(false),
        started_at: Some("2026-05-30T01:02:03Z".into()),
        completed_at: None,
        git: Some(RunGit {
            change_strategy: "worktree-per-unit".into(),
            auto_merge: true,
            auto_squash: false,
        }),
        seal: None,
        external_refs: Default::default(),
        setup: None,
        created_by: Some("jason@example.com".into()),
        composite: None,
        sync: vec![],
        composite_state: Default::default(),
    };
    let doc = frontmatter::serialize(&fm, "# Body\n").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.created_by, fm.created_by);
    assert_eq!(back.title, fm.title);
    assert_eq!(back.factory, fm.factory);
    assert_eq!(back.mode, fm.mode);
    assert_eq!(back.active_station, fm.active_station);
    assert_eq!(back.status, fm.status);
    assert_eq!(back.archived, fm.archived);
    assert_eq!(back.started_at, fm.started_at);
    assert_eq!(back.completed_at, fm.completed_at);
    let g = back.git.expect("git");
    assert_eq!(g.change_strategy, "worktree-per-unit");
    assert!(g.auto_merge);
    assert!(!g.auto_squash);
}

#[test]
fn roundtrip_run_minimal_frontmatter() {
    let fm = run_fm();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.factory, "software");
    assert_eq!(back.status, Status::Pending);
    assert!(back.title.is_none());
    assert!(back.git.is_none());
}

#[test]
fn roundtrip_run_each_status() {
    for status in [
        Status::Pending,
        Status::Active,
        Status::InProgress,
        Status::Completed,
        Status::Blocked,
    ] {
        let mut fm = run_fm();
        fm.status = status;
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.status, status, "status {status:?}");
    }
}

#[test]
fn roundtrip_run_title_with_colon() {
    let mut fm = run_fm();
    fm.title = Some("Phase 1: Spec".into());
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.title.as_deref(), Some("Phase 1: Spec"));
}

#[test]
fn roundtrip_run_title_with_unicode() {
    let mut fm = run_fm();
    fm.title = Some("Café — 日本語 🚀".into());
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.title.as_deref(), Some("Café — 日本語 🚀"));
}

#[test]
fn roundtrip_run_title_with_quotes() {
    let mut fm = run_fm();
    fm.title = Some("the \"quoted\" thing".into());
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.title.as_deref(), Some("the \"quoted\" thing"));
}

#[test]
fn roundtrip_run_title_with_leading_dash() {
    // A value that looks like YAML syntax must survive the roundtrip.
    let mut fm = run_fm();
    fm.title = Some("- not a list".into());
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.title.as_deref(), Some("- not a list"));
}

#[test]
fn roundtrip_run_value_that_looks_like_bool() {
    // A string "true" must roundtrip as a string, not a bool.
    let mut fm = run_fm();
    fm.active_station = "true".into();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.active_station, "true");
}

#[test]
fn roundtrip_run_value_that_looks_like_number() {
    let mut fm = run_fm();
    fm.active_station = "12345".into();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.active_station, "12345");
}

#[test]
fn roundtrip_run_value_with_newlines() {
    let mut fm = run_fm();
    fm.active_station = "a\nb\nc".into();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    assert_eq!(back.active_station, "a\nb\nc");
}

#[test]
fn roundtrip_run_git_all_true() {
    let mut fm = run_fm();
    fm.git = Some(RunGit {
        change_strategy: "rebase".into(),
        auto_merge: true,
        auto_squash: true,
    });
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    let g = back.git.expect("git");
    assert!(g.auto_merge && g.auto_squash);
    assert_eq!(g.change_strategy, "rebase");
}

#[test]
fn roundtrip_run_git_default() {
    let mut fm = run_fm();
    fm.git = Some(RunGit::default());
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = parse_run(&doc);
    let g = back.git.expect("git");
    assert_eq!(g.change_strategy, "");
    assert!(!g.auto_merge);
    assert!(!g.auto_squash);
}

#[test]
fn roundtrip_run_body_idempotent_double_pass() {
    // serialize -> parse -> serialize must yield a stable document.
    let fm = RunFrontmatter {
        title: Some("Stable".into()),
        factory: "software".into(),
        status: Status::Completed,
        ..Default::default()
    };
    let body = "# Stable\n\nContent.\n";
    let doc1 = frontmatter::serialize(&fm, body).expect("ser1");
    let (back, b) = parse_run(&doc1);
    let doc2 = frontmatter::serialize(&back, &b).expect("ser2");
    assert_eq!(doc1, doc2, "serialize is not idempotent");
}

// ===========================================================================
// roundtrip: UnitFrontmatter
// ===========================================================================

#[test]
fn roundtrip_unit_full() {
    let fm = UnitFrontmatter {
        name: Some("Build the API".into()),
        unit_type: "feature".into(),
        status: Status::InProgress,
        depends_on: vec!["a".into(), "b".into()],
        worker: "builder".into(),
        model: Some("opus".into()),
        station: Some("frame".into()),
        revise: false,
        inputs: vec!["in/x.md".into()],
        outputs: vec!["out/y.md".into(), "out/z.md".into()],
        started_at: Some("2026-05-30T00:00:00Z".into()),
        completed_at: Some("2026-05-30T01:00:00Z".into()),
        ..Default::default()
    };
    let doc = frontmatter::serialize(&fm, "# Unit\n").expect("ser");
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse unit");
    assert_eq!(back.name, fm.name);
    assert_eq!(back.unit_type, fm.unit_type);
    assert_eq!(back.status, fm.status);
    assert_eq!(back.depends_on, fm.depends_on);
    assert_eq!(back.worker, fm.worker);
    assert_eq!(back.model, fm.model);
    assert_eq!(back.station, fm.station);
    assert_eq!(back.inputs, fm.inputs);
    assert_eq!(back.outputs, fm.outputs);
    assert_eq!(back.started_at, fm.started_at);
    assert_eq!(back.completed_at, fm.completed_at);
}

#[test]
fn roundtrip_unit_default() {
    let fm = UnitFrontmatter::default();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse unit");
    assert!(back.name.is_none());
    assert_eq!(back.unit_type, "");
    assert_eq!(back.status, Status::Pending);
    assert!(back.depends_on.is_empty());
    assert!(back.iterations.is_empty());
    assert_eq!(back.worker, "");
    assert!(back.model.is_none());
    assert!(back.station.is_none());
    assert!(back.inputs.is_empty());
    assert!(back.outputs.is_empty());
}

#[test]
fn roundtrip_unit_skips_empty_vecs() {
    let fm = UnitFrontmatter::default();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    // inputs/outputs are skip_serializing_if Vec::is_empty.
    assert!(!doc.contains("inputs:"));
    assert!(!doc.contains("outputs:"));
}

#[test]
fn roundtrip_unit_emits_nonempty_depends_on() {
    let mut fm = UnitFrontmatter::default();
    fm.depends_on = vec!["x".into()];
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    assert!(doc.contains("depends_on:"));
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
    assert_eq!(back.depends_on, vec!["x"]);
}

#[test]
fn roundtrip_unit_depends_on_preserves_order() {
    let mut fm = UnitFrontmatter::default();
    fm.depends_on = vec!["z".into(), "a".into(), "m".into()];
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
    assert_eq!(back.depends_on, vec!["z", "a", "m"]);
}

#[test]
fn roundtrip_unit_each_status() {
    for status in [
        Status::Pending,
        Status::Active,
        Status::InProgress,
        Status::Completed,
        Status::Blocked,
    ] {
        let mut fm = UnitFrontmatter::default();
        fm.status = status;
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) =
            frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
        assert_eq!(back.status, status, "status {status:?}");
    }
}

#[test]
fn roundtrip_unit_iterations_carry_note_and_completed_at() {
    use darkrun_core::domain::{IterationResult, UnitIteration};
    let mut fm = UnitFrontmatter::default();
    fm.iterations = vec![
        UnitIteration {
            worker: "make".into(),
            started_at: Some("2026-06-02T00:00:00Z".into()),
            completed_at: Some("2026-06-02T00:05:00Z".into()),
            result: Some(IterationResult::Advance),
            note: Some("drafted the limiter; next: stress the burst path".into()),
        },
        UnitIteration {
            worker: "challenge".into(),
            result: Some(IterationResult::Reject),
            note: Some("burst path overflows the bucket — bounce to make".into()),
            ..Default::default()
        },
    ];
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) = frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
    assert_eq!(back.iterations.len(), 2);
    assert_eq!(back.iterations[0].completed_at.as_deref(), Some("2026-06-02T00:05:00Z"));
    assert_eq!(back.iterations[0].note.as_deref(), Some("drafted the limiter; next: stress the burst path"));
    assert_eq!(back.iterations[1].result, Some(IterationResult::Reject));
    assert_eq!(back.iterations[1].note.as_deref(), Some("burst path overflows the bucket — bounce to make"));
}

#[test]
fn roundtrip_unit_many_deps() {
    let deps: Vec<String> = (0..50).map(|i| format!("dep-{i}")).collect();
    let mut fm = UnitFrontmatter::default();
    fm.depends_on = deps.clone();
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
    assert_eq!(back.depends_on, deps);
}

#[test]
fn roundtrip_unit_dep_with_special_chars() {
    let mut fm = UnitFrontmatter::default();
    fm.depends_on = vec!["a:b".into(), "c#d".into(), "  spaced  ".into()];
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
    assert_eq!(back.depends_on, vec!["a:b", "c#d", "  spaced  "]);
}

#[test]
fn roundtrip_unit_unicode_name() {
    let mut fm = UnitFrontmatter::default();
    fm.name = Some("構築 🛠".into());
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    let (back, _) =
        frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
    assert_eq!(back.name.as_deref(), Some("構築 🛠"));
}

#[test]
fn roundtrip_unit_idempotent() {
    let fm = UnitFrontmatter {
        name: Some("U".into()),
        status: Status::Active,
        depends_on: vec!["a".into()],
        ..Default::default()
    };
    let doc1 = frontmatter::serialize(&fm, "# U\n").expect("ser");
    let (back, b) =
        frontmatter::parse::<UnitFrontmatter>(&doc1).expect("parse");
    let doc2 = frontmatter::serialize(&back, &b).expect("ser2");
    assert_eq!(doc1, doc2);
}

// ===========================================================================
// roundtrip: other domain frontmatter types through the YAML envelope
// ===========================================================================

#[test]
fn roundtrip_station_through_envelope() {
    let st = Station {
        station: "frame".into(),
        status: Status::Active,
        phase: StationPhase::Manufacture,
            elaborated: false,
        checkpoint: Some(Checkpoint {
            kind: CheckpointKind::Ask,
            entered_at: Some("2026-05-30T00:00:00Z".into()),
            outcome: Some(CheckpointOutcome::Paused),
        }),
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: Some("2026-05-30T00:00:00Z".into()),
        completed_at: None,
    };
    let doc = frontmatter::serialize(&st, "# Station\n").expect("ser");
    let (back, _) =
        frontmatter::parse::<Station>(&doc).expect("parse station");
    assert_eq!(back.station, "frame");
    assert_eq!(back.status, Status::Active);
    assert_eq!(back.phase, StationPhase::Manufacture);
    let cp = back.checkpoint.expect("checkpoint");
    assert_eq!(cp.kind, CheckpointKind::Ask);
    assert_eq!(cp.outcome, Some(CheckpointOutcome::Paused));
}

#[test]
fn roundtrip_station_each_phase() {
    for phase in [
        StationPhase::Spec,
        StationPhase::Review,
        StationPhase::Manufacture,
        StationPhase::Audit,
        StationPhase::Reflect,
        StationPhase::Checkpoint,
    ] {
        let st = Station {
            station: "s".into(),
            status: Status::Pending,
            phase,
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
        };
        let doc = frontmatter::serialize(&st, "").expect("ser");
        let (back, _) =
            frontmatter::parse::<Station>(&doc).expect("parse");
        assert_eq!(back.phase, phase, "phase {phase:?}");
    }
}

#[test]
fn roundtrip_checkpoint_each_kind() {
    for kind in [
        CheckpointKind::Auto,
        CheckpointKind::Ask,
        CheckpointKind::External,
        CheckpointKind::Await,
    ] {
        let st = Station {
            station: "s".into(),
            status: Status::Pending,
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
        let doc = frontmatter::serialize(&st, "").expect("ser");
        let (back, _) =
            frontmatter::parse::<Station>(&doc).expect("parse");
        assert_eq!(back.checkpoint.unwrap().kind, kind, "kind {kind:?}");
    }
}

#[test]
fn roundtrip_checkpoint_each_outcome() {
    for outcome in [
        CheckpointOutcome::Advanced,
        CheckpointOutcome::Paused,
        CheckpointOutcome::Blocked,
        CheckpointOutcome::Awaiting,
    ] {
        let st = Station {
            station: "s".into(),
            status: Status::Pending,
            phase: StationPhase::Checkpoint,
            elaborated: false,
            checkpoint: Some(Checkpoint {
                kind: CheckpointKind::Auto,
                entered_at: None,
                outcome: Some(outcome),
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
        let doc = frontmatter::serialize(&st, "").expect("ser");
        let (back, _) =
            frontmatter::parse::<Station>(&doc).expect("parse");
        assert_eq!(
            back.checkpoint.unwrap().outcome,
            Some(outcome),
            "outcome {outcome:?}"
        );
    }
}

#[test]
fn roundtrip_station_skips_none_checkpoint() {
    let st = Station {
        station: "s".into(),
        status: Status::Pending,
        phase: StationPhase::Spec,
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
    };
    let doc = frontmatter::serialize(&st, "").expect("ser");
    assert!(!doc.contains("checkpoint:"));
    assert!(!doc.contains("started_at:"));
}

#[test]
fn roundtrip_station_idempotent() {
    let st = Station {
        station: "build".into(),
        status: Status::Completed,
        phase: StationPhase::Reflect,
            elaborated: false,
        checkpoint: Some(Checkpoint {
            kind: CheckpointKind::External,
            entered_at: Some("2026-05-30T00:00:00Z".into()),
            outcome: Some(CheckpointOutcome::Advanced),
        }),
        branch: None,
        pr_ref: None,
        pr_status: None,
        pr_ready_at: None,
        pr_merged_at: None,
        verifier_nonce: None,
        started_at: Some("2026-05-30T00:00:00Z".into()),
        completed_at: Some("2026-05-30T02:00:00Z".into()),
    };
    let doc1 = frontmatter::serialize(&st, "# Build\n").expect("ser");
    let (back, b) = frontmatter::parse::<Station>(&doc1).expect("parse");
    let doc2 = frontmatter::serialize(&back, &b).expect("ser2");
    assert_eq!(doc1, doc2);
}

// ===========================================================================
// first_heading
// ===========================================================================

#[test]
fn first_heading_finds_h1() {
    assert_eq!(
        frontmatter::first_heading("# My Title\nbody\n").as_deref(),
        Some("My Title")
    );
}

#[test]
fn first_heading_on_first_line() {
    assert_eq!(
        frontmatter::first_heading("# Only Line").as_deref(),
        Some("Only Line")
    );
}

#[test]
fn first_heading_skips_non_heading_lines() {
    let body = "intro paragraph\n\n# Real Title\nmore\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), Some("Real Title"));
}

#[test]
fn first_heading_returns_first_of_several() {
    let body = "# First\n# Second\n# Third\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), Some("First"));
}

#[test]
fn first_heading_ignores_h2() {
    assert_eq!(frontmatter::first_heading("## Subhead\n").as_deref(), None);
}

#[test]
fn first_heading_ignores_h3_and_deeper() {
    assert_eq!(frontmatter::first_heading("### Deep\n").as_deref(), None);
    assert_eq!(frontmatter::first_heading("###### Deepest\n").as_deref(), None);
}

#[test]
fn first_heading_skips_h2_then_finds_h1() {
    let body = "## sub\n# Main\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), Some("Main"));
}

#[test]
fn first_heading_trims_indentation_and_whitespace() {
    assert_eq!(
        frontmatter::first_heading("   #   Spaced Out   \n").as_deref(),
        Some("Spaced Out")
    );
}

#[test]
fn first_heading_trims_only_outer_whitespace() {
    // Internal spacing within the title is preserved.
    assert_eq!(
        frontmatter::first_heading("#  a  b  c  \n").as_deref(),
        Some("a  b  c")
    );
}

#[test]
fn first_heading_skips_empty_heading() {
    let body = "# \n# Real\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), Some("Real"));
}

#[test]
fn first_heading_skips_whitespace_only_heading() {
    let body = "#    \n# Real\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), Some("Real"));
}

#[test]
fn first_heading_none_when_absent() {
    assert_eq!(frontmatter::first_heading("just text\n").as_deref(), None);
    assert_eq!(frontmatter::first_heading("").as_deref(), None);
}

#[test]
fn first_heading_none_for_whitespace_only_body() {
    assert_eq!(frontmatter::first_heading("   \n\n  \n").as_deref(), None);
}

#[test]
fn first_heading_does_not_match_hashtag_without_space() {
    assert_eq!(frontmatter::first_heading("#NoSpace\n").as_deref(), None);
}

#[test]
fn first_heading_does_not_match_hash_alone() {
    assert_eq!(frontmatter::first_heading("#\n").as_deref(), None);
}

#[test]
fn first_heading_matches_hash_tab_is_not_heading() {
    // The module requires `# ` (hash + space); a tab does not match.
    assert_eq!(frontmatter::first_heading("#\tTabbed\n").as_deref(), None);
}

#[test]
fn first_heading_with_unicode_title() {
    assert_eq!(
        frontmatter::first_heading("# 日本語 café 🚀\n").as_deref(),
        Some("日本語 café 🚀")
    );
}

#[test]
fn first_heading_with_trailing_hashes_kept() {
    // The module does not strip closing-hash ATX syntax; they are part of the
    // title text.
    assert_eq!(
        frontmatter::first_heading("# Title #\n").as_deref(),
        Some("Title #")
    );
}

#[test]
fn first_heading_with_inline_markdown_kept() {
    assert_eq!(
        frontmatter::first_heading("# A **bold** title\n").as_deref(),
        Some("A **bold** title")
    );
}

#[test]
fn first_heading_after_blank_lines() {
    assert_eq!(
        frontmatter::first_heading("\n\n\n# Late\n").as_deref(),
        Some("Late")
    );
}

#[test]
fn first_heading_single_line_no_newline() {
    assert_eq!(frontmatter::first_heading("# NoNewline").as_deref(), Some("NoNewline"));
}

#[test]
fn first_heading_ignores_hash_in_middle_of_line() {
    assert_eq!(
        frontmatter::first_heading("text # not a heading\n").as_deref(),
        None
    );
}

#[test]
fn first_heading_from_split_body() {
    // Integration: split then first_heading on the body.
    let doc = frontmatter::split("---\nfactory: x\n---\n# Extracted\nbody\n");
    assert_eq!(
        frontmatter::first_heading(&doc.body).as_deref(),
        Some("Extracted")
    );
}

#[test]
fn first_heading_none_on_frontmatter_string() {
    // The frontmatter string is YAML, not markdown headings.
    let doc = frontmatter::split("---\nfactory: x\n---\nbody\n");
    assert_eq!(frontmatter::first_heading(&doc.frontmatter).as_deref(), None);
}

// ===========================================================================
// cross-cutting: parse/split agreement
// ===========================================================================

#[test]
fn parse_and_split_agree_on_body() {
    let raw = "---\nfactory: software\n---\n# H\nbody text\n";
    let doc = frontmatter::split(raw);
    let (_, parse_body) = parse_run(raw);
    assert_eq!(doc.body, parse_body);
}

#[test]
fn serialize_output_splits_back_cleanly() {
    let doc = frontmatter::serialize(&run_fm(), "# H\nbody\n").expect("ser");
    let split = frontmatter::split(&doc);
    assert!(split.frontmatter.contains("factory: software"));
    assert!(split.body.contains("# H"));
    assert!(split.body.contains("body"));
}

#[test]
fn serialize_then_first_heading_recovers_title() {
    let doc = frontmatter::serialize(&run_fm(), "# Recovered Title\n\nbody\n").expect("ser");
    let split = frontmatter::split(&doc);
    assert_eq!(
        frontmatter::first_heading(&split.body).as_deref(),
        Some("Recovered Title")
    );
}

#[test]
fn empty_body_roundtrips_to_empty_body() {
    let doc = frontmatter::serialize(&run_fm(), "").expect("ser");
    let (_, body) = parse_run(&doc);
    assert_eq!(body, "");
}

#[test]
fn body_with_yaml_frontmatter_lookalike_inside() {
    // A body that *contains* `---` lines must not confuse the splitter beyond
    // the first close fence.
    let body = "intro\n---\nfake: yaml\n---\nmore\n";
    let doc = frontmatter::serialize(&run_fm(), body).expect("ser");
    let split = frontmatter::split(&doc);
    // The first close fence ends the real frontmatter; everything after is body.
    assert!(split.frontmatter.contains("factory: software"));
    assert!(split.body.contains("fake: yaml"));
}

// ===========================================================================
// roundtrip: Worker / Explorer / Reviewer / Pass through the envelope
// ===========================================================================

#[test]
fn roundtrip_worker_full() {
    let w = Worker {
        name: "builder".into(),
        model: Some("opus".into()),
        terminal: true,
    };
    let doc = frontmatter::serialize(&w, "# Worker\n").expect("ser");
    let (back, _) = frontmatter::parse::<Worker>(&doc).expect("parse");
    assert_eq!(back.name, "builder");
    assert_eq!(back.model.as_deref(), Some("opus"));
    assert!(back.terminal);
}

#[test]
fn roundtrip_worker_minimal_defaults() {
    let w = Worker {
        name: "challenger".into(),
        model: None,
        terminal: false,
    };
    let doc = frontmatter::serialize(&w, "").expect("ser");
    assert!(!doc.contains("model:"));
    let (back, _) = frontmatter::parse::<Worker>(&doc).expect("parse");
    assert_eq!(back.name, "challenger");
    assert!(back.model.is_none());
    assert!(!back.terminal);
}

#[test]
fn roundtrip_worker_terminal_true_emitted() {
    let w = Worker {
        name: "x".into(),
        model: None,
        terminal: true,
    };
    let doc = frontmatter::serialize(&w, "").expect("ser");
    assert!(doc.contains("terminal: true"));
}

#[test]
fn parse_worker_missing_name_errors() {
    let err = frontmatter::parse::<Worker>("---\nterminal: true\n---\nb\n").unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn roundtrip_explorer_full() {
    let e = Explorer {
        name: "context".into(),
        mandate: "gather repo conventions".into(),
    };
    let doc = frontmatter::serialize(&e, "").expect("ser");
    let (back, _) = frontmatter::parse::<Explorer>(&doc).expect("parse");
    assert_eq!(back.name, "context");
    assert_eq!(back.mandate, "gather repo conventions");
}

#[test]
fn roundtrip_explorer_default_mandate() {
    let e = Explorer {
        name: "value".into(),
        mandate: String::new(),
    };
    let doc = frontmatter::serialize(&e, "").expect("ser");
    let (back, _) = frontmatter::parse::<Explorer>(&doc).expect("parse");
    assert_eq!(back.name, "value");
    assert_eq!(back.mandate, "");
}

#[test]
fn parse_explorer_mandate_defaults_when_absent() {
    let (back, _) =
        frontmatter::parse::<Explorer>("---\nname: ctx\n---\nbody\n").expect("parse");
    assert_eq!(back.mandate, "");
}

#[test]
fn roundtrip_reviewer_full() {
    let r = Reviewer {
        name: "feasibility".into(),
        dimension: "can it ship".into(),
    };
    let doc = frontmatter::serialize(&r, "").expect("ser");
    let (back, _) = frontmatter::parse::<Reviewer>(&doc).expect("parse");
    assert_eq!(back.name, "feasibility");
    assert_eq!(back.dimension, "can it ship");
}

#[test]
fn roundtrip_reviewer_default_dimension() {
    let r = Reviewer {
        name: "value".into(),
        dimension: String::new(),
    };
    let doc = frontmatter::serialize(&r, "").expect("ser");
    let (back, _) = frontmatter::parse::<Reviewer>(&doc).expect("parse");
    assert_eq!(back.dimension, "");
}

#[test]
fn roundtrip_pass_full() {
    let p = Pass {
        index: 4,
        unit: "u1".into(),
        beat: PassBeat::Challenge,
    };
    let doc = frontmatter::serialize(&p, "").expect("ser");
    let (back, _) = frontmatter::parse::<Pass>(&doc).expect("parse");
    assert_eq!(back.index, 4);
    assert_eq!(back.unit, "u1");
    assert_eq!(back.beat, PassBeat::Challenge);
}

#[test]
fn roundtrip_pass_each_beat() {
    for beat in [PassBeat::Make, PassBeat::Challenge, PassBeat::Resolve] {
        let p = Pass {
            index: 0,
            unit: "u".into(),
            beat,
        };
        let doc = frontmatter::serialize(&p, "").expect("ser");
        let (back, _) = frontmatter::parse::<Pass>(&doc).expect("parse");
        assert_eq!(back.beat, beat, "beat {beat:?}");
    }
}

#[test]
fn roundtrip_pass_index_boundaries() {
    for idx in [0u32, 1, 1000, u32::MAX] {
        let p = Pass {
            index: idx,
            unit: "u".into(),
            beat: PassBeat::Make,
        };
        let doc = frontmatter::serialize(&p, "").expect("ser");
        let (back, _) = frontmatter::parse::<Pass>(&doc).expect("parse");
        assert_eq!(back.index, idx, "idx {idx}");
    }
}

#[test]
fn parse_pass_missing_beat_errors() {
    let err = frontmatter::parse::<Pass>("---\nindex: 0\nunit: u\n---\nb\n").unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_pass_unknown_beat_errors() {
    let raw = "---\nindex: 0\nunit: u\nbeat: dance\n---\nb\n";
    let err = frontmatter::parse::<Pass>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

// ===========================================================================
// roundtrip: Feedback through the envelope
// ===========================================================================

#[test]
fn roundtrip_feedback_full() {
    let f = Feedback {
        id: "fb-1".into(),
        run: "run-a".into(),
        station: "frame".into(),
        status: FeedbackStatus::Fixing,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: Some(FeedbackSeverity::Blocker),
        body: "the gate failed".into(),
        created_at: Some("2026-05-30T00:00:00Z".into()),
    };
    let doc = frontmatter::serialize(&f, "# Feedback\n").expect("ser");
    let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
    assert_eq!(back.id, "fb-1");
    assert_eq!(back.run, "run-a");
    assert_eq!(back.station, "frame");
    assert_eq!(back.status, FeedbackStatus::Fixing);
    assert_eq!(back.severity, Some(FeedbackSeverity::Blocker));
    assert_eq!(back.body, "the gate failed");
    assert_eq!(back.created_at.as_deref(), Some("2026-05-30T00:00:00Z"));
}

#[test]
fn roundtrip_feedback_no_severity_skipped() {
    let f = Feedback {
        id: "fb-2".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::Pending,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: None,
        body: String::new(),
        created_at: None,
    };
    let doc = frontmatter::serialize(&f, "").expect("ser");
    assert!(!doc.contains("severity:"));
    assert!(!doc.contains("created_at:"));
    let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
    assert!(back.severity.is_none());
    assert_eq!(back.body, "");
}

#[test]
fn roundtrip_feedback_each_status() {
    for status in [
        FeedbackStatus::Pending,
        FeedbackStatus::Fixing,
        FeedbackStatus::Addressed,
        FeedbackStatus::Answered,
        FeedbackStatus::NonActionable,
        FeedbackStatus::Escalated,
        FeedbackStatus::Closed,
        FeedbackStatus::Rejected,
    ] {
        let f = Feedback {
            id: "id".into(),
            run: "r".into(),
            station: "s".into(),
            status,
            severity: None,
            origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
            invalidates: vec![],
            closure_reply: None,
            body: String::new(),
            created_at: None,
        };
        let doc = frontmatter::serialize(&f, "").expect("ser");
        let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
        assert_eq!(back.status, status, "status {status:?}");
    }
}

#[test]
fn roundtrip_feedback_each_severity() {
    for sev in [
        FeedbackSeverity::Blocker,
        FeedbackSeverity::High,
        FeedbackSeverity::Medium,
        FeedbackSeverity::Low,
    ] {
        let f = Feedback {
            id: "id".into(),
            run: "r".into(),
            station: "s".into(),
            status: FeedbackStatus::Pending,
            origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
            invalidates: vec![],
            closure_reply: None,
            severity: Some(sev),
            body: String::new(),
            created_at: None,
        };
        let doc = frontmatter::serialize(&f, "").expect("ser");
        let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
        assert_eq!(back.severity, Some(sev), "severity {sev:?}");
    }
}

#[test]
fn roundtrip_feedback_multiline_body_field() {
    let f = Feedback {
        id: "id".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::Pending,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: None,
        body: "line one\nline two\nline three".into(),
        created_at: None,
    };
    let doc = frontmatter::serialize(&f, "").expect("ser");
    let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
    assert_eq!(back.body, "line one\nline two\nline three");
}

#[test]
fn roundtrip_feedback_unicode_body() {
    let f = Feedback {
        id: "id".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::Pending,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: None,
        body: "壊れた — café 🚀".into(),
        created_at: None,
    };
    let doc = frontmatter::serialize(&f, "").expect("ser");
    let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
    assert_eq!(back.body, "壊れた — café 🚀");
}

#[test]
fn parse_feedback_missing_required_id_errors() {
    let raw = "---\nrun: r\nstation: s\nstatus: pending\n---\nb\n";
    let err = frontmatter::parse::<Feedback>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn parse_feedback_unknown_status_errors() {
    let raw = "---\nid: i\nrun: r\nstation: s\nstatus: vibing\n---\nb\n";
    let err = frontmatter::parse::<Feedback>(raw).unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

#[test]
fn roundtrip_feedback_idempotent() {
    let f = Feedback {
        id: "fb".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::Escalated,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: Some(FeedbackSeverity::High),
        body: "x".into(),
        created_at: Some("2026-05-30T00:00:00Z".into()),
    };
    let doc1 = frontmatter::serialize(&f, "# FB\n").expect("ser");
    let (back, b) = frontmatter::parse::<Feedback>(&doc1).expect("parse");
    let doc2 = frontmatter::serialize(&back, &b).expect("ser2");
    assert_eq!(doc1, doc2);
}

// ===========================================================================
// ===========================================================================

// ===========================================================================
// additional split edge cases
// ===========================================================================

#[test]
fn split_frontmatter_with_comment_lines() {
    let doc = frontmatter::split("---\n# a yaml comment\nfactory: software\n---\nbody\n");
    assert!(doc.frontmatter.contains("# a yaml comment"));
    assert!(doc.frontmatter.contains("factory: software"));
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_frontmatter_with_blank_lines_inside() {
    let doc = frontmatter::split("---\na: 1\n\nb: 2\n---\nbody\n");
    assert_eq!(doc.frontmatter, "a: 1\n\nb: 2\n");
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_body_with_leading_blank_line() {
    let doc = frontmatter::split("---\nx: 1\n---\n\nbody after blank\n");
    assert_eq!(doc.body, "\nbody after blank\n");
}

#[test]
fn split_does_not_match_fence_not_at_start() {
    // A doc whose very first chars are not `---\n` is body-only even if a
    // fence appears later.
    let doc = frontmatter::split(" ---\nx: 1\n---\nbody\n");
    assert_eq!(doc.frontmatter, "");
    assert!(doc.body.starts_with(" ---"));
}

#[test]
fn split_tab_indented_first_line_is_body() {
    let doc = frontmatter::split("\t---\nx: 1\n---\nbody\n");
    assert_eq!(doc.frontmatter, "");
    assert!(doc.body.starts_with("\t---"));
}

#[test]
fn split_close_fence_on_very_first_content_line() {
    // `---\n---\n...`: open then immediate close.
    let doc = frontmatter::split("---\n---\ncontent\nmore\n");
    assert_eq!(doc.frontmatter, "");
    assert_eq!(doc.body, "content\nmore\n");
}

#[test]
fn split_large_frontmatter_body() {
    let fm_lines: String = (0..100).map(|i| format!("k{i}: {i}\n")).collect();
    let raw = format!("---\n{fm_lines}---\nbody\n");
    let doc = frontmatter::split(&raw);
    assert!(doc.frontmatter.contains("k0: 0"));
    assert!(doc.frontmatter.contains("k99: 99"));
    assert_eq!(doc.body, "body\n");
}

#[test]
fn split_body_preserves_internal_dashes_block() {
    let doc = frontmatter::split("---\nx: 1\n---\n```\n---\n```\n");
    assert_eq!(doc.frontmatter, "x: 1\n");
    assert_eq!(doc.body, "```\n---\n```\n");
}

#[test]
fn split_carriage_return_only_not_normalized() {
    // The module normalizes `\r\n` but a lone `\r` is not a line break it
    // handles; ensure it does not crash and treats the content as body.
    let doc = frontmatter::split("plain\rbody\r");
    assert_eq!(doc.frontmatter, "");
    assert!(doc.body.contains("plain"));
}

// ===========================================================================
// additional parse edge cases
// ===========================================================================

#[test]
fn parse_run_with_yaml_comment_in_frontmatter() {
    let raw = "---\n# comment\nfactory: software\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.factory, "software");
}

#[test]
fn parse_run_extra_unknown_fields_ignored() {
    // serde_yaml ignores unknown fields by default (no deny_unknown_fields).
    let raw = "---\nfactory: software\nunknown_key: whatever\n---\nbody\n";
    let (fm, _) = parse_run(raw);
    assert_eq!(fm.factory, "software");
}

#[test]
fn parse_run_empty_string_factory_is_valid() {
    // An empty string is a valid String value.
    let (fm, _) = parse_run("---\nfactory: \"\"\n---\nbody\n");
    assert_eq!(fm.factory, "");
}

#[test]
fn parse_run_null_optional_stays_none() {
    let (fm, _) = parse_run("---\nfactory: software\ntitle: null\n---\nbody\n");
    assert!(fm.title.is_none());
}

#[test]
fn parse_run_explicit_tilde_null() {
    let (fm, _) = parse_run("---\nfactory: software\ntitle: ~\n---\nbody\n");
    assert!(fm.title.is_none());
}

#[test]
fn parse_run_body_carries_trailing_blank_lines() {
    let (_, body) = parse_run("---\nfactory: software\n---\nbody\n\n\n");
    assert_eq!(body, "body\n\n\n");
}

#[test]
fn parse_unit_station_field() {
    let raw = "---\nstation: build\n---\nbody\n";
    let (fm, _) = frontmatter::parse::<UnitFrontmatter>(raw).expect("parse");
    assert_eq!(fm.station.as_deref(), Some("build"));
}

#[test]
fn parse_unit_model_override() {
    let raw = "---\nmodel: sonnet\n---\nbody\n";
    let (fm, _) = frontmatter::parse::<UnitFrontmatter>(raw).expect("parse");
    assert_eq!(fm.model.as_deref(), Some("sonnet"));
}

#[test]
fn parse_unit_worker_assignment() {
    let raw = "---\nworker: challenger\n---\nbody\n";
    let (fm, _) = frontmatter::parse::<UnitFrontmatter>(raw).expect("parse");
    assert_eq!(fm.worker, "challenger");
}

#[test]
fn parse_status_in_progress_underscore_form() {
    // snake_case wire form for InProgress is `in_progress`.
    let (fm, _) = parse_run("---\nfactory: software\nstatus: in_progress\n---\nb\n");
    assert_eq!(fm.status, Status::InProgress);
}

#[test]
fn parse_status_rejects_camelcase() {
    // `inProgress` is not the snake_case wire form.
    let err =
        frontmatter::parse::<RunFrontmatter>("---\nfactory: software\nstatus: inProgress\n---\nb\n")
            .unwrap_err();
    assert!(matches!(err, CoreError::Yaml(_)), "got {err:?}");
}

// ===========================================================================
// additional serialize edge cases
// ===========================================================================

#[test]
fn serialize_unit_emits_snake_case_status() {
    let mut fm = UnitFrontmatter::default();
    fm.status = Status::InProgress;
    let doc = frontmatter::serialize(&fm, "").expect("ser");
    assert!(doc.contains("status: in_progress"));
}

#[test]
fn serialize_station_emits_snake_case_phase() {
    let st = Station {
        station: "s".into(),
        status: Status::Pending,
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
    };
    let doc = frontmatter::serialize(&st, "").expect("ser");
    assert!(doc.contains("phase: manufacture"));
}

#[test]
fn serialize_pass_emits_snake_case_beat() {
    let p = Pass {
        index: 0,
        unit: "u".into(),
        beat: PassBeat::Resolve,
    };
    let doc = frontmatter::serialize(&p, "").expect("ser");
    assert!(doc.contains("beat: resolve"));
}

#[test]
fn serialize_feedback_status_snake_case() {
    let f = Feedback {
        id: "i".into(),
        run: "r".into(),
        station: "s".into(),
        status: FeedbackStatus::NonActionable,
        origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
        invalidates: vec![],
        closure_reply: None,
        severity: None,
        body: String::new(),
        created_at: None,
    };
    let doc = frontmatter::serialize(&f, "").expect("ser");
    assert!(doc.contains("status: non_actionable"));
}

#[test]
fn serialize_then_parse_worker_idempotent() {
    let w = Worker {
        name: "n".into(),
        model: Some("m".into()),
        terminal: true,
    };
    let doc1 = frontmatter::serialize(&w, "# W\n").expect("ser");
    let (back, b) = frontmatter::parse::<Worker>(&doc1).expect("parse");
    let doc2 = frontmatter::serialize(&back, &b).expect("ser2");
    assert_eq!(doc1, doc2);
}

#[test]
fn serialize_then_parse_pass_idempotent() {
    let p = Pass {
        index: 9,
        unit: "u".into(),
        beat: PassBeat::Make,
    };
    let doc1 = frontmatter::serialize(&p, "").expect("ser");
    let (back, b) = frontmatter::parse::<Pass>(&doc1).expect("parse");
    let doc2 = frontmatter::serialize(&back, &b).expect("ser2");
    assert_eq!(doc1, doc2);
}

// ===========================================================================
// additional first_heading edge cases
// ===========================================================================

#[test]
fn first_heading_handles_crlf_in_body() {
    // first_heading uses lines(), which handles `\r\n`; the trailing `\r`
    // is part of the line content unless trimmed. The module trims trailing
    // whitespace, so `\r` is removed.
    assert_eq!(
        frontmatter::first_heading("# Title\r\nbody\r\n").as_deref(),
        Some("Title")
    );
}

#[test]
fn first_heading_emoji_only_title() {
    assert_eq!(frontmatter::first_heading("# 🚀\n").as_deref(), Some("🚀"));
}

#[test]
fn first_heading_numeric_title() {
    assert_eq!(frontmatter::first_heading("# 12345\n").as_deref(), Some("12345"));
}

#[test]
fn first_heading_with_colon() {
    assert_eq!(
        frontmatter::first_heading("# Phase 1: Spec\n").as_deref(),
        Some("Phase 1: Spec")
    );
}

#[test]
fn first_heading_among_many_h2_lines() {
    let body = "## a\n## b\n## c\n# Finally\n## d\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), Some("Finally"));
}

#[test]
fn first_heading_indented_two_spaces() {
    assert_eq!(
        frontmatter::first_heading("  # Indented\n").as_deref(),
        Some("Indented")
    );
}

#[test]
fn first_heading_very_long_title() {
    let long = "x".repeat(5000);
    let body = format!("# {long}\n");
    assert_eq!(frontmatter::first_heading(&body).as_deref(), Some(long.as_str()));
}

#[test]
fn first_heading_ignores_setext_underline() {
    // Setext headings (`Title` then `===`) are not ATX `# ` headings.
    let body = "Title\n=====\n";
    assert_eq!(frontmatter::first_heading(body).as_deref(), None);
}

// ===========================================================================
// determinism: same input always yields the same output
// ===========================================================================

#[test]
fn split_is_deterministic() {
    let raw = "---\nfactory: software\nmode: continuous\n---\n# H\nbody\n";
    let a = frontmatter::split(raw);
    let b = frontmatter::split(raw);
    assert_eq!(a, b);
}

#[test]
fn serialize_is_deterministic() {
    let fm = RunFrontmatter {
        title: Some("T".into()),
        factory: "software".into(),
        status: Status::Active,
        ..Default::default()
    };
    let a = frontmatter::serialize(&fm, "# T\n").expect("ser");
    let b = frontmatter::serialize(&fm, "# T\n").expect("ser");
    assert_eq!(a, b);
}

#[test]
fn first_heading_is_deterministic() {
    let body = "intro\n# Title\nmore\n";
    assert_eq!(
        frontmatter::first_heading(body),
        frontmatter::first_heading(body)
    );
}

#[test]
fn parse_is_deterministic() {
    let raw = "---\nfactory: software\n---\nbody\n";
    let (a, ab) = parse_run(raw);
    let (b, bb) = parse_run(raw);
    assert_eq!(a.factory, b.factory);
    assert_eq!(ab, bb);
}

// ===========================================================================
// matrix: run roundtrip over many factory string shapes
// ===========================================================================

#[test]
fn roundtrip_run_factory_shapes() {
    for factory in [
        "software",
        "a-b-c",
        "with space",
        "café",
        "日本語",
        "x:y",
        "x#y",
        "123",
        "  leading-trailing  ",
        "🚀",
        "very_long_factory_name_that_keeps_going_and_going",
    ] {
        let mut fm = run_fm();
        fm.factory = factory.into();
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.factory, factory, "factory {factory:?}");
    }
}

#[test]
fn roundtrip_run_mode_shapes() {
    // Every mode round-trips through frontmatter as its canonical token.
    for mode in Mode::ALL {
        let mut fm = run_fm();
        fm.mode = mode;
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.mode, mode, "mode {mode:?}");
    }
    // Legacy mode strings still load, mapped onto the team/solo/dark model.
    for (legacy, want) in [
        ("continuous", Mode::Solo),
        ("collaborative", Mode::Solo),
        ("discrete", Mode::Team),
        ("discrete-hybrid", Mode::Team),
        ("autopilot", Mode::Dark),
        ("quick", Mode::Solo),
    ] {
        let raw = format!("---\nfactory: software\nmode: {legacy}\n---\nbody\n");
        let (back, _) = parse_run(&raw);
        assert_eq!(back.mode, want, "legacy mode {legacy:?}");
    }
}

#[test]
fn roundtrip_run_active_station_shapes() {
    for st in ["frame", "build", "ship", "_root", "a/b", ""] {
        let mut fm = run_fm();
        fm.active_station = st.into();
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.active_station, st, "station {st:?}");
    }
}

#[test]
fn roundtrip_run_title_shapes() {
    for title in [
        "Simple",
        "With: colon",
        "With \"quotes\"",
        "With 'apostrophe'",
        "café 🚀 日本語",
        "- looks like list",
        "# looks like comment",
        "  padded  ",
        "trailing newline\n",
    ] {
        let mut fm = run_fm();
        fm.title = Some(title.into());
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.title.as_deref(), Some(title), "title {title:?}");
    }
}

#[test]
fn roundtrip_run_timestamp_shapes() {
    for ts in [
        "2026-05-30T00:00:00Z",
        "2026-01-01T12:34:56.789Z",
        "2026-05-30T00:00:00+05:30",
        "1970-01-01T00:00:00Z",
    ] {
        let mut fm = run_fm();
        fm.started_at = Some(ts.into());
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.started_at.as_deref(), Some(ts), "ts {ts}");
    }
}

#[test]
fn roundtrip_run_git_change_strategy_shapes() {
    for strat in ["worktree-per-unit", "rebase", "squash", "", "branch/per/unit"] {
        let mut fm = run_fm();
        fm.git = Some(RunGit {
            change_strategy: strat.into(),
            auto_merge: false,
            auto_squash: false,
        });
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        assert_eq!(back.git.unwrap().change_strategy, strat, "strat {strat:?}");
    }
}

#[test]
fn roundtrip_run_git_bool_matrix() {
    for (m, s) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut fm = run_fm();
        fm.git = Some(RunGit {
            change_strategy: "x".into(),
            auto_merge: m,
            auto_squash: s,
        });
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = parse_run(&doc);
        let g = back.git.unwrap();
        assert_eq!(g.auto_merge, m);
        assert_eq!(g.auto_squash, s);
    }
}

// ===========================================================================
// matrix: unit worker / unit_type / model shapes
// ===========================================================================

#[test]
fn roundtrip_unit_worker_shapes() {
    for w in ["builder", "challenger", "resolver", "", "multi word worker"] {
        let mut fm = UnitFrontmatter::default();
        fm.worker = w.into();
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
        assert_eq!(back.worker, w, "worker {w:?}");
    }
}

#[test]
fn roundtrip_unit_type_shapes() {
    for t in ["feature", "bugfix", "spike", "", "refactor/cleanup"] {
        let mut fm = UnitFrontmatter::default();
        fm.unit_type = t.into();
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
        assert_eq!(back.unit_type, t, "type {t:?}");
    }
}

#[test]
fn roundtrip_unit_model_shapes() {
    for m in ["opus", "sonnet", "swift-x", "custom:model", ""] {
        let mut fm = UnitFrontmatter::default();
        fm.model = Some(m.into());
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
        assert_eq!(back.model.as_deref(), Some(m), "model {m:?}");
    }
}

#[test]
fn roundtrip_unit_station_shapes() {
    for s in ["frame", "_root", "build", "a/b/c", ""] {
        let mut fm = UnitFrontmatter::default();
        fm.station = Some(s.into());
        let doc = frontmatter::serialize(&fm, "").expect("ser");
        let (back, _) = frontmatter::parse::<UnitFrontmatter>(&doc).expect("parse");
        assert_eq!(back.station.as_deref(), Some(s), "station {s:?}");
    }
}

// ===========================================================================
// matrix: explorer / reviewer name+mandate/dimension shapes
// ===========================================================================

#[test]
fn roundtrip_explorer_shapes() {
    for (name, mandate) in [
        ("context", "gather conventions"),
        ("value", ""),
        ("risk", "find what breaks"),
        ("名前", "日本語 mandate 🚀"),
    ] {
        let e = Explorer {
            name: name.into(),
            mandate: mandate.into(),
        };
        let doc = frontmatter::serialize(&e, "").expect("ser");
        let (back, _) = frontmatter::parse::<Explorer>(&doc).expect("parse");
        assert_eq!(back.name, name);
        assert_eq!(back.mandate, mandate);
    }
}

#[test]
fn roundtrip_reviewer_shapes() {
    for (name, dim) in [
        ("value", "is it worth it"),
        ("feasibility", ""),
        ("security", "auth & authz"),
        ("レビュー", "次元 🔍"),
    ] {
        let r = Reviewer {
            name: name.into(),
            dimension: dim.into(),
        };
        let doc = frontmatter::serialize(&r, "").expect("ser");
        let (back, _) = frontmatter::parse::<Reviewer>(&doc).expect("parse");
        assert_eq!(back.name, name);
        assert_eq!(back.dimension, dim);
    }
}

// ===========================================================================
// matrix: drift path / unit shapes
// ===========================================================================

// ===========================================================================
// matrix: feedback id/run/station shapes
// ===========================================================================

#[test]
fn roundtrip_feedback_id_shapes() {
    for id in ["fb-1", "FEEDBACK_42", "uuid-abc-123", "café-id"] {
        let f = Feedback {
            id: id.into(),
            run: "r".into(),
            station: "s".into(),
            status: FeedbackStatus::Pending,
            origin: darkrun_core::domain::FeedbackOrigin::Unspecified,
            invalidates: vec![],
            closure_reply: None,
            severity: None,
            body: String::new(),
            created_at: None,
        };
        let doc = frontmatter::serialize(&f, "").expect("ser");
        let (back, _) = frontmatter::parse::<Feedback>(&doc).expect("parse");
        assert_eq!(back.id, id, "id {id:?}");
    }
}

// ===========================================================================
// body content preservation across serialize for many shapes
// ===========================================================================

#[test]
fn serialize_preserves_varied_bodies() {
    for body in [
        "# H\n",
        "no heading body\n",
        "# H\n\nmulti\n\npara\n",
        "list:\n- a\n- b\n",
        "```\ncode block\n```\n",
        "unicode 日本語 🚀\n",
        "trailing spaces   \n",
        "\nleading newline\n",
    ] {
        let out = frontmatter::serialize(&run_fm(), body).expect("ser");
        let (_, recovered) = parse_run(&out);
        // Body should reappear after the fence; a leading-newline body keeps
        // its leading newline, others get exactly one injected blank line.
        assert!(
            recovered.contains(body.trim_start_matches('\n').lines().next().unwrap_or("")),
            "body fragment missing for {body:?}: recovered {recovered:?}"
        );
    }
}

#[test]
fn roundtrip_body_with_emoji_and_cjk_exact() {
    let body = "# 計画\n\ncafé ☕ 🚀 — 日本語テスト\n";
    let out = frontmatter::serialize(&run_fm(), body).expect("ser");
    let split = frontmatter::split(&out);
    // serialize injects one blank line between the close fence and a body that
    // does not already start with a newline; the split body carries it back.
    assert_eq!(split.body, format!("\n{body}"));
}

#[test]
fn roundtrip_body_only_blank_lines() {
    let out = frontmatter::serialize(&run_fm(), "\n\n\n").expect("ser");
    let split = frontmatter::split(&out);
    // body starts with \n so no injection; fence `---\n` + body.
    assert_eq!(split.body, "\n\n\n");
}
