//! darkrun-prompts — the engine-driven prompt system.
//!
//! **Prompts are engine-driven data; skills are thin wrappers.** The real
//! per-action instructions live as minijinja templates in `plugin/prompts/`,
//! embedded into the binary. The manager picks a template key for each
//! [`RunAction`] it emits, and this crate resolves and renders it with live
//! context — honoring a **project-override cascade** so any prompt can be
//! overridden by dropping a file at `<repo_root>/.darkrun/prompts/<rel>.md`
//! with no fork.
//!
//! ## Public API
//!
//! - [`render`] — resolve `rel` through the cascade, render it with `context`
//!   (a serde-serializable value), and return the final markdown. `{% include %}`
//!   inside templates also honors the cascade.
//! - [`resolve`] — return the raw (unrendered) template source for `rel`.
//! - [`template_key_for_action`] — the action → template key mapping the manager
//!   uses to turn a `RunAction` tag into a `rel`.
//! - [`Cascade`] — the resolver itself, if a caller wants to reuse one across
//!   many renders.
//!
//! ```
//! # use serde_json::json;
//! let md = darkrun_prompts::render(
//!     "phases/spec",
//!     "/nonexistent-repo-root",
//!     &json!({ "run": "r1", "station": "frame", "kills": "ambiguity",
//!              "explorers": ["context"], "units": ["u1"] }),
//! ).unwrap();
//! assert!(md.contains("frame"));
//! ```

mod cascade;
mod error;
mod providers;

use std::path::Path;

use minijinja::Environment;
use serde::Serialize;

pub use cascade::Cascade;
pub use error::{PromptError, Result};

/// Build a minijinja [`Environment`] whose loader resolves `{% include %}` and
/// `{% extends %}` through the cascade — so partials honor project overrides
/// exactly like the top-level template does.
fn environment_for(repo_root: impl AsRef<Path>) -> Environment<'static> {
    let cascade = Cascade::new(repo_root);
    let mut env = Environment::new();
    env.set_loader(move |name| {
        // minijinja includes use the bare key (e.g. `_shared/contracts.md`);
        // strip the `.md` so cascade keys are suffix-free and overrides line up.
        let rel = name.strip_suffix(".md").unwrap_or(name);
        cascade
            .resolve_for_loader(rel)
            .map_err(|e| minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string()))
    });
    env
}

/// Resolve the raw template source for `rel` through the override cascade.
///
/// Project override (`<repo_root>/.darkrun/prompts/<rel>.md`) wins over the
/// embedded default. Returns [`PromptError::UnknownTemplate`] when neither
/// exists. This returns the *unrendered* source; use [`render`] to render it.
pub fn resolve(rel: &str, repo_root: impl AsRef<Path>) -> Result<String> {
    Cascade::new(repo_root).resolve(rel)
}

/// Resolve `rel` through the cascade and render it with `context`.
///
/// `context` is any serde-serializable value; its fields become template
/// variables (`{{ run }}`, `{% for u in units %}`, …). Includes inside the
/// template are resolved through the same cascade, so an overridden partial is
/// honored transitively.
pub fn render<C: Serialize>(rel: &str, repo_root: impl AsRef<Path>, context: &C) -> Result<String> {
    let repo_root = repo_root.as_ref().to_path_buf();
    // The top-level source comes from the cascade; if it's missing, surface a
    // clean UnknownTemplate rather than a minijinja "template not found".
    let source = Cascade::new(&repo_root).resolve(rel)?;

    let env = environment_for(&repo_root);
    let value = serde_json::to_value(context).map_err(PromptError::Context)?;
    let tmpl = env
        .template_from_str(&source)
        .map_err(|source| PromptError::Render {
            rel: rel.to_string(),
            source,
        })?;
    let rendered = tmpl.render(value).map_err(|source| PromptError::Render {
        rel: rel.to_string(),
        source,
    })?;
    // Splice active provider behavior contracts (git, ticketing, spec,
    // knowledge, design) into the phases they declare — the agent carries an
    // integration's rules exactly where they apply, and nowhere else.
    Ok(match providers::provider_block(&repo_root, rel) {
        Some(block) => format!("{rendered}{block}"),
        None => rendered,
    })
}

/// Map a manager [`RunAction`](https://docs.rs) tag to its template key (`rel`).
///
/// The manager emits a `RunAction` whose `serde` tag (`action` field) is one of
/// the snake_case strings below; this turns that tag into the `rel` passed to
/// [`render`]. Centralizing the mapping here keeps the manager free of template
/// paths and lets the corpus reorganize without touching `darkrun-mcp`.
///
/// | action tag        | template key        |
/// |-------------------|---------------------|
/// | `spec`            | `phases/spec`       |
/// | `review`          | `phases/review`     |
/// | `manufacture`     | `phases/manufacture`|
/// | `audit`           | `phases/audit`      |
/// | `reflect`         | `phases/reflect`    |
/// | `user_gate`       | `phases/user_gate`  |
/// | `checkpoint`      | `phases/checkpoint` |
/// | `fix_feedback`    | `tracks/fix_feedback`|
/// | `sealed`          | `run/sealed`        |
/// | `noop`            | `run/noop`          |
/// | `run_completion`  | `run/run_completion`|
///
/// Returns `None` for an unrecognized tag so the caller can decide how to
/// handle a future action that has no template yet.
pub fn template_key_for_action(action_tag: &str) -> Option<&'static str> {
    Some(match action_tag {
        "spec" => "phases/spec",
        "review" => "phases/review",
        "manufacture" => "phases/manufacture",
        "audit" => "phases/audit",
        "reflect" => "phases/reflect",
        "user_gate" => "phases/user_gate",
        "checkpoint" => "phases/checkpoint",
        "fix_feedback" => "tracks/fix_feedback",
        "feedback_question" => "tracks/feedback_question",
        "merge_conflict" => "tracks/merge_conflict",
        "save_wip" => "tracks/save_wip",
        "units_invalid" => "validation/units_invalid",
        "escalate" => "validation/escalate",
        "best_effort_boot" => "validation/best_effort_boot",
        "escalate_to_user" => "validation/escalate_to_user",
        "safe_repair" => "validation/safe_repair",
        "revise_unit_specs" => "run/revise_unit_specs",
        "run_review" => "run/run_review",
        "external_review_requested" => "run/external_review_requested",
        "pending_seal" => "run/pending_seal",
        "sealed" => "run/sealed",
        "noop" => "run/noop",
        "run_completion" => "run/run_completion",
        _ => return None,
    })
}

/// Every action tag the mapping recognizes, in a stable order. Handy for tests
/// and for tooling that wants to enumerate the manager's vocabulary.
pub const ACTION_TAGS: &[&str] = &[
    "spec",
    "review",
    "manufacture",
    "audit",
    "reflect",
    "user_gate",
    "checkpoint",
    "fix_feedback",
    "feedback_question",
    "merge_conflict",
    "save_wip",
    "units_invalid",
    "escalate",
    "best_effort_boot",
    "escalate_to_user",
    "safe_repair",
    "revise_unit_specs",
    "run_review",
    "external_review_requested",
    "pending_seal",
    "sealed",
    "noop",
    "run_completion",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    /// A repo root with no `.darkrun/prompts` overrides — exercises the embedded
    /// arm of the cascade.
    fn empty_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_override(root: &Path, rel: &str, body: &str) {
        let path = root.join(".darkrun").join("prompts").join(format!("{rel}.md"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    // ── Mapping ──────────────────────────────────────────────────────────────

    #[test]
    fn every_action_tag_maps_and_resolves() {
        let root = empty_root();
        for tag in ACTION_TAGS {
            let key = template_key_for_action(tag).unwrap_or_else(|| panic!("no key for {tag}"));
            resolve(key, root.path()).unwrap_or_else(|e| panic!("{tag}->{key} resolve: {e}"));
        }
    }

    #[test]
    fn render_surfaces_a_runtime_template_fault() {
        // A syntactically-valid override that fails at RENDER time: it includes a
        // partial that doesn't resolve through the cascade. template_from_str
        // succeeds; the include errors when the body renders → PromptError::Render.
        let root = empty_root();
        write_override(root.path(), "phases/reflect", "{% include \"_shared/does-not-exist\" %}");
        match render("phases/reflect", root.path(), &json!({ "run": "r", "station": "s" })) {
            Err(PromptError::Render { rel, .. }) => assert_eq!(rel, "phases/reflect"),
            other => panic!("expected a Render fault, got {other:?}"),
        }
    }

    #[test]
    fn unknown_action_tag_has_no_key() {
        assert_eq!(template_key_for_action("teleport"), None);
    }

    // ── Render: vars / conditionals / loops / includes ───────────────────────

    #[test]
    fn render_interpolates_vars() {
        let root = empty_root();
        let out = render(
            "phases/reflect",
            root.path(),
            &json!({ "run": "r9", "station": "prove" }),
        )
        .unwrap();
        assert!(out.contains("`r9`"));
        assert!(out.contains("`prove`"));
    }

    #[test]
    fn render_honors_conditionals_per_checkpoint_kind() {
        let root = empty_root();
        let ask = render(
            "phases/checkpoint",
            root.path(),
            &json!({ "run": "r", "station": "frame", "kind": "ask", "kills": "x" }),
        )
        .unwrap();
        assert!(ask.contains("a human must approve"));

        let auto = render(
            "phases/checkpoint",
            root.path(),
            &json!({ "run": "r", "station": "build", "kind": "auto", "kills": "x" }),
        )
        .unwrap();
        assert!(auto.contains("no human in the loop"));
        assert!(!auto.contains("a human must approve"));
    }

    #[test]
    fn render_iterates_loops() {
        let root = empty_root();
        let out = render(
            "phases/manufacture",
            root.path(),
            &json!({
                "run": "r", "station": "build", "worker": "make",
                "units": ["alpha", "beta", "gamma"]
            }),
        )
        .unwrap();
        for u in ["alpha", "beta", "gamma"] {
            assert!(out.contains(u), "missing unit {u}");
        }
        assert!(out.contains("make"));
    }

    #[test]
    fn render_pulls_in_shared_includes() {
        let root = empty_root();
        let out = render(
            "phases/spec",
            root.path(),
            &json!({ "run": "r1", "station": "frame", "kills": "ambiguity",
                     "explorers": ["context", "value"] }),
        )
        .unwrap();
        // From _shared/contracts.md
        assert!(out.contains("Contract"));
        // From _shared/announcement.md
        assert!(out.contains("**Run**"));
        // From _shared/roster.md (loop inside an included partial)
        assert!(out.contains("Explorers"));
        assert!(out.contains("context") && out.contains("value"));
    }

    // ── Cascade: override beats embedded ─────────────────────────────────────

    #[test]
    fn project_override_beats_embedded() {
        let root = empty_root();
        write_override(root.path(), "phases/spec", "OVERRIDDEN {{ station }}");
        let out = render(
            "phases/spec",
            root.path(),
            &json!({ "run": "r", "station": "frame", "kills": "x" }),
        )
        .unwrap();
        assert!(out.starts_with("OVERRIDDEN frame"));
        assert!(!out.contains("Eliminates"));
    }

    #[test]
    fn override_of_a_shared_partial_is_honored_via_include() {
        let root = empty_root();
        // Override only the partial; the top-level template is still embedded.
        write_override(root.path(), "_shared/contracts", "MY CUSTOM CONTRACT");
        let out = render(
            "phases/audit",
            root.path(),
            &json!({ "run": "r", "station": "prove", "kills": "x", "reviewers": ["coverage"] }),
        )
        .unwrap();
        assert!(out.contains("MY CUSTOM CONTRACT"));
        assert!(!out.contains("source of truth"));
    }

    #[test]
    fn override_refreshes_on_mtime_change() {
        let root = empty_root();
        let cascade = Cascade::new(root.path());

        write_override(root.path(), "phases/spec", "V1");
        let path = root
            .path()
            .join(".darkrun/prompts/phases/spec.md");
        // Backdate the first write so the second write has a strictly later mtime.
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(10);
        filetime_set(&path, past);
        assert_eq!(cascade.resolve("phases/spec").unwrap(), "V1");

        // Rewrite with a fresh (current) mtime; cache must refresh.
        fs::write(&path, "V2").unwrap();
        filetime_set(&path, std::time::SystemTime::now());
        assert_eq!(cascade.resolve("phases/spec").unwrap(), "V2");
    }

    #[test]
    fn removing_override_falls_back_to_embedded() {
        let root = empty_root();
        let cascade = Cascade::new(root.path());
        write_override(root.path(), "phases/spec", "TEMP OVERRIDE");
        assert_eq!(cascade.resolve("phases/spec").unwrap(), "TEMP OVERRIDE");

        fs::remove_file(root.path().join(".darkrun/prompts/phases/spec.md")).unwrap();
        let back = cascade.resolve("phases/spec").unwrap();
        assert!(back.contains("Spec"), "should fall back to embedded default");
    }

    // ── Errors ───────────────────────────────────────────────────────────────

    #[test]
    fn render_unknown_template_is_unknown_template_error() {
        let root = empty_root();
        match render("phases/ghost", root.path(), &json!({})) {
            Err(PromptError::UnknownTemplate(k)) => assert_eq!(k, "phases/ghost"),
            other => panic!("expected UnknownTemplate, got {other:?}"),
        }
    }

    #[test]
    fn render_bad_template_syntax_is_render_error() {
        let root = empty_root();
        write_override(root.path(), "phases/spec", "{% if %}broken");
        match render("phases/spec", root.path(), &json!({ "station": "x" })) {
            Err(PromptError::Render { rel, .. }) => assert_eq!(rel, "phases/spec"),
            other => panic!("expected Render error, got {other:?}"),
        }
    }

    // ── Every corpus template renders with a representative context ───────────

    #[test]
    fn every_corpus_template_renders() {
        let root = empty_root();
        let ctx = json!({
            "run": "demo-run",
            "station": "build",
            "phase": "manufacture",
            "kills": "an entire class of risk",
            "kind": "ask",
            "locked_artifact": "build.md",
            "worker": "make",
            "feedback_id": "fb-007",
            "path": "frame.md",
            "message": "mid-wave hold",
            "explorers": ["reuse", "integration_point"],
            "workers": ["builder", "test_author"],
            "reviewers": ["correctness", "maintainability"],
            "reflections": ["architecture", "quality"],
            "units": ["u1", "u2", "u3"],
        });
        // Only render the *non-partial* corpus entries (partials are exercised
        // via includes); rendering a partial standalone is still valid though.
        for key in Cascade::embedded_keys() {
            let out = render(&key, root.path(), &ctx)
                .unwrap_or_else(|e| panic!("template `{key}` failed to render: {e}"));
            assert!(!out.trim().is_empty(), "template `{key}` rendered empty");
        }
    }

    #[test]
    fn corpus_covers_phases_tracks_and_run() {
        let keys = Cascade::embedded_keys();
        for required in [
            "phases/spec",
            "phases/review",
            "phases/manufacture",
            "phases/audit",
            "phases/reflect",
            "phases/checkpoint",
            "tracks/fix_feedback",
            "run/run_completion",
            "run/sealed",
            "_shared/announcement",
            "_shared/contracts",
        ] {
            assert!(keys.contains(&required.to_string()), "corpus missing {required}");
        }
    }

    /// Each phase template must walk its named sub-step beats (the methodology
    /// lives in these beats). If an edit drops a beat, this fails fast.
    #[test]
    fn each_phase_walks_its_named_beats() {
        let root = empty_root();
        let ctx = json!({
            "run": "r", "station": "build", "kills": "a class of risk",
            "kind": "ask", "worker": "make",
            "explorers": ["context"], "workers": ["builder"],
            "reviewers": ["correctness"], "units": ["u1"],
        });
        let beats: &[(&str, &[&str])] = &[
            ("phases/spec", &["elaborate", "explore"]),
            ("phases/review", &["spec", "adversarial", "brief"]),
            ("phases/user_gate", &["gate", "operator"]),
            ("phases/manufacture", &["make", "challenge", "resolve"]),
            ("phases/audit", &["spec", "adversarial"]),
            ("phases/reflect", &["agentic"]),
            ("phases/checkpoint", &["brief", "user"]),
        ];
        for (key, want) in beats {
            let out = render(key, root.path(), &ctx).unwrap();
            for beat in *want {
                assert!(out.contains(beat), "`{key}` missing beat `{beat}`:\n{out}");
            }
        }
    }

    // ── Visual design direction (user-facing work) ──────────────────────────

    /// When `user_facing` is set, the manufacture phase must instruct the agent to
    /// generate design options and get the operator's visual decision via the
    /// `darkrun_question` / `darkrun_direction` tools before building any UI.
    #[test]
    fn manufacture_asks_for_a_design_direction_on_user_facing_work() {
        let root = empty_root();
        let out = render(
            "phases/manufacture",
            root.path(),
            &json!({
                "run": "r", "station": "shape", "worker": "make",
                "units": ["u1"], "user_facing": true,
            }),
        )
        .unwrap();
        let lower = out.to_lowercase();
        assert!(lower.contains("design direction"), "should name a design direction:\n{out}");
        assert!(out.contains("darkrun_question"), "should reference darkrun_question:\n{out}");
        assert!(out.contains("darkrun_direction"), "should reference darkrun_direction:\n{out}");
        assert!(
            lower.contains("mockup") || lower.contains("option"),
            "should tell the agent to generate options:\n{out}"
        );
    }

    /// Non-UI work skips the visual-design step entirely — the design-direction
    /// guidance only renders when `user_facing` is truthy.
    #[test]
    fn manufacture_skips_design_direction_when_not_user_facing() {
        let root = empty_root();
        let out = render(
            "phases/manufacture",
            root.path(),
            &json!({ "run": "r", "station": "build", "worker": "make", "units": ["u1"] }),
        )
        .unwrap();
        assert!(
            !out.contains("darkrun_question") && !out.contains("darkrun_direction"),
            "non-UI work must not see the visual-decision tools:\n{out}"
        );
        // The core Pass loop is still intact.
        for beat in ["make", "challenge", "resolve"] {
            assert!(out.contains(beat), "manufacture missing beat `{beat}`:\n{out}");
        }
    }

    /// The spec phase flags user-facing surfaces so Shape's visual step knows to
    /// act — but only when `user_facing` is set.
    #[test]
    fn spec_flags_user_facing_surfaces_conditionally() {
        let root = empty_root();
        let ctx_ui = json!({
            "run": "r", "station": "specify", "kills": "ambiguity",
            "explorers": ["contract"], "user_facing": true,
        });
        let ui = render("phases/spec", root.path(), &ctx_ui).unwrap();
        assert!(
            ui.to_lowercase().contains("user-facing surface"),
            "UI specs flag the surface:\n{ui}"
        );
        assert!(ui.contains("darkrun_question") || ui.contains("darkrun_direction"));

        let plain = render(
            "phases/spec",
            root.path(),
            &json!({ "run": "r", "station": "specify", "kills": "ambiguity",
                     "explorers": ["contract"] }),
        )
        .unwrap();
        assert!(
            !plain.contains("darkrun_question") && !plain.contains("darkrun_direction"),
            "non-UI specs carry no design-direction requirement:\n{plain}"
        );
    }

    // ── Surface-routed audit verification ───────────────────────────────────

    /// A visual surface routes Audit to the headless-browser proof: `darkrun
    /// verify web`, the web vitals, and a `WebProof` attached via
    /// `darkrun_proof_attach`.
    #[test]
    fn audit_routes_visual_surface_to_web_verification() {
        let root = empty_root();
        let out = render(
            "phases/audit",
            root.path(),
            &json!({
                "run": "r", "station": "prove", "kills": "escaped defects",
                "reviewers": ["evidence"], "surface": "web_ui", "user_facing": true,
            }),
        )
        .unwrap();
        assert!(out.contains("darkrun verify web"), "visual audit runs the web verifier:\n{out}");
        assert!(out.contains("darkrun_proof_attach"), "visual audit attaches a proof:\n{out}");
        let lower = out.to_lowercase();
        assert!(lower.contains("lcp") && lower.contains("vitals"), "names the web vitals:\n{out}");
        assert!(!out.contains("darkrun bench"), "visual audit must not route to bench:\n{out}");
    }

    /// A bench surface routes Audit to `darkrun bench` + a `BenchProof`.
    #[test]
    fn audit_routes_bench_surface_to_bench_verification() {
        let root = empty_root();
        let out = render(
            "phases/audit",
            root.path(),
            &json!({
                "run": "r", "station": "prove", "kills": "escaped defects",
                "reviewers": ["evidence"], "surface": "api", "bench_surface": true,
            }),
        )
        .unwrap();
        assert!(out.contains("darkrun bench"), "bench audit runs the bench harness:\n{out}");
        assert!(out.contains("darkrun_proof_attach"), "bench audit attaches a proof:\n{out}");
        let lower = out.to_lowercase();
        assert!(lower.contains("p95") || lower.contains("percentile"), "names percentiles:\n{out}");
        assert!(!out.contains("darkrun verify web"), "bench audit must not route to the browser:\n{out}");
    }

    /// A terminal surface routes Audit to an output snapshot.
    #[test]
    fn audit_routes_terminal_surface_to_snapshot() {
        let root = empty_root();
        let out = render(
            "phases/audit",
            root.path(),
            &json!({
                "run": "r", "station": "prove", "kills": "escaped defects",
                "reviewers": ["evidence"], "surface": "cli", "terminal_surface": true,
            }),
        )
        .unwrap();
        let lower = out.to_lowercase();
        assert!(lower.contains("snapshot"), "terminal audit takes an output snapshot:\n{out}");
        assert!(out.contains("darkrun_proof_attach"), "terminal audit attaches a proof:\n{out}");
        assert!(!out.contains("darkrun verify web") && !out.contains("darkrun bench"),
            "terminal audit must not route to browser/bench:\n{out}");
    }

    /// With no surface classified, the audit carries no surface-routed proof
    /// requirement — the verification block stays dark.
    #[test]
    fn audit_without_surface_carries_no_proof_route() {
        let root = empty_root();
        let out = render(
            "phases/audit",
            root.path(),
            &json!({ "run": "r", "station": "build", "kills": "regressions",
                     "reviewers": ["correctness"] }),
        )
        .unwrap();
        assert!(
            !out.contains("darkrun verify web") && !out.contains("darkrun bench")
                && !out.contains("darkrun_proof_attach"),
            "unclassified audit carries no surface proof route:\n{out}"
        );
    }

    /// Regression: the checkpoint template must not claim the station "passed
    /// tests" — Tests is folded into Audit, and the phase before Checkpoint is
    /// Reflect. The pre-gate summary should name reflect, not tests.
    #[test]
    fn checkpoint_does_not_reference_removed_tests_phase() {
        let root = empty_root();
        let out = render(
            "phases/checkpoint",
            root.path(),
            &json!({ "run": "r", "station": "build", "kind": "ask", "kills": "x" }),
        )
        .unwrap();
        assert!(!out.contains("audit, and tests"), "stale tests reference:\n{out}");
        assert!(out.contains("reflect"), "checkpoint should name reflect:\n{out}");
    }

    /// Force a file's mtime so the cache-refresh test is deterministic rather
    /// than racing the filesystem clock. Uses the `filetime` dev-dependency.
    fn filetime_set(path: &Path, when: std::time::SystemTime) {
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when))
            .expect("set mtime");
    }
}
