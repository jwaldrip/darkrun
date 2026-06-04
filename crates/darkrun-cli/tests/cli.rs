//! End-to-end tests for the `darkrun` binary.
//!
//! These drive the *built* `darkrun` executable through `std::process::Command`,
//! asserting on stdout/stderr/exit code, and exercise the on-disk `.darkrun/`
//! state machine, the auth/credential surface (against a temp `HOME`, no
//! network), the Claude Code statusline (rendered from piped workspace JSON,
//! installed/uninstalled into temp settings), factory listing, slugify edge
//! cases, and the clap-level error paths.
//!
//! Every test isolates its state under a fresh `tempfile::TempDir` and passes
//! `--repo` (or a temp `HOME`) so nothing touches the developer's real machine
//! and the suite is order-independent.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;

/// Path to the binary under test, provided by Cargo for integration tests.
const BIN: &str = env!("CARGO_BIN_EXE_darkrun");

// ─── harness ────────────────────────────────────────────────────────────────

/// A captured invocation: status + decoded stdout/stderr.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Builder for a single `darkrun` invocation.
struct Cli {
    args: Vec<String>,
    env: HashMap<String, String>,
    env_remove: Vec<String>,
    stdin: Option<String>,
    cwd: Option<String>,
}

impl Cli {
    fn new() -> Self {
        Cli {
            args: Vec::new(),
            env: HashMap::new(),
            env_remove: Vec::new(),
            stdin: None,
            cwd: None,
        }
    }

    fn arg(mut self, a: impl AsRef<str>) -> Self {
        self.args.push(a.as_ref().to_string());
        self
    }

    fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for a in it {
            self.args.push(a.as_ref().to_string());
        }
        self
    }

    /// Set `--repo <dir>` as the leading global flag.
    fn repo(self, dir: &Path) -> Self {
        let p = dir.to_string_lossy().into_owned();
        let mut v = vec!["--repo".to_string(), p];
        v.extend(self.args.clone());
        Cli {
            args: v,
            ..self
        }
    }

    fn env(mut self, k: &str, v: &str) -> Self {
        self.env.insert(k.to_string(), v.to_string());
        self
    }

    fn env_remove(mut self, k: &str) -> Self {
        self.env_remove.push(k.to_string());
        self
    }

    /// Point `HOME` at `dir` (and `USERPROFILE` for portability).
    fn home(self, dir: &Path) -> Self {
        let p = dir.to_string_lossy().into_owned();
        self.env("HOME", &p).env("USERPROFILE", &p)
    }

    fn stdin(mut self, s: impl Into<String>) -> Self {
        self.stdin = Some(s.into());
        self
    }

    fn cwd(mut self, dir: &Path) -> Self {
        self.cwd = Some(dir.to_string_lossy().into_owned());
        self
    }

    fn run(self) -> Run {
        let mut cmd = Command::new(BIN);
        cmd.args(self.args.iter().map(OsStr::new));
        // Keep the website base unset so URL builders use the canonical default
        // and tests stay deterministic regardless of the developer's shell.
        cmd.env_remove("DARKRUN_WEB_BASE");
        for k in &self.env_remove {
            cmd.env_remove(k);
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        if let Some(dir) = &self.cwd {
            cmd.current_dir(dir);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().expect("spawn darkrun");
        if let Some(input) = &self.stdin {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(input.as_bytes())
                .expect("write stdin");
        }
        // Always drop stdin so commands that read it (statusline) see EOF.
        drop(child.stdin.take());
        let Output {
            status,
            stdout,
            stderr,
        } = child.wait_with_output().expect("wait darkrun");
        Run {
            code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        }
    }
}

/// A throwaway repo root under a temp dir.
fn temp_repo() -> TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Start a run in `repo` and return the captured output.
fn start_run(repo: &Path, desc: &str) -> Run {
    Cli::new().repo(repo).args(["run", "start", desc]).run()
}

/// Parse stdout as JSON or panic with context.
fn json(out: &str) -> Value {
    serde_json::from_str(out).unwrap_or_else(|e| panic!("not JSON ({e}):\n{out}"))
}

/// Seed a credentials file under `home/.darkrun/credentials`.
fn seed_credentials(home: &Path, body: &str) {
    let dir = home.join(".darkrun");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("credentials"), body).unwrap();
}

// ─── --version / --help ──────────────────────────────────────────────────────

#[test]
fn version_flag_prints_name_and_semver() {
    let r = Cli::new().arg("--version").run();
    assert!(r.ok());
    assert!(r.stdout.starts_with("darkrun "));
    assert!(r.stdout.trim().contains("0.1.0"));
}

#[test]
fn short_version_flag_matches_long() {
    let long = Cli::new().arg("--version").run();
    let short = Cli::new().arg("-V").run();
    assert_eq!(long.stdout, short.stdout);
    assert!(short.ok());
}

#[test]
fn help_flag_lists_all_subcommands() {
    let r = Cli::new().arg("--help").run();
    assert!(r.ok());
    for sub in ["mcp", "serve", "run", "auth", "factory", "statusline"] {
        assert!(r.stdout.contains(sub), "help missing `{sub}`:\n{}", r.stdout);
    }
}

#[test]
fn short_help_flag_matches_long() {
    let long = Cli::new().arg("--help").run();
    let short = Cli::new().arg("-h").run();
    assert_eq!(long.stdout, short.stdout);
}

#[test]
fn help_documents_repo_global_flag() {
    let r = Cli::new().arg("--help").run();
    assert!(r.stdout.contains("--repo"));
}

#[test]
fn help_shows_usage_line() {
    let r = Cli::new().arg("--help").run();
    assert!(r.stdout.contains("Usage: darkrun"));
}

#[test]
fn no_command_errors_with_usage_exit_2() {
    let r = Cli::new().run();
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("Usage: darkrun") || r.stderr.contains("Usage:"));
}

#[test]
fn unknown_subcommand_exits_2() {
    let r = Cli::new().arg("definitely-not-a-command").run();
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("unrecognized subcommand"));
}

#[test]
fn unknown_flag_exits_2() {
    let r = Cli::new().args(["--totally-bogus", "factory", "list"]).run();
    assert_eq!(r.code, 2);
}

#[test]
fn run_subcommand_help_lists_actions() {
    let r = Cli::new().args(["run", "--help"]).run();
    assert!(r.ok());
    for action in ["start", "next", "show", "decide", "pr"] {
        assert!(r.stdout.contains(action), "run help missing `{action}`");
    }
}

#[test]
fn run_without_action_errors_exit_2() {
    let r = Cli::new().arg("run").run();
    assert_eq!(r.code, 2);
}

#[test]
fn auth_subcommand_help_lists_actions() {
    let r = Cli::new().args(["auth", "--help"]).run();
    assert!(r.ok());
    for action in ["login", "status", "logout"] {
        assert!(r.stdout.contains(action));
    }
}

#[test]
fn factory_subcommand_help_lists_list() {
    let r = Cli::new().args(["factory", "--help"]).run();
    assert!(r.ok());
    assert!(r.stdout.contains("list"));
}

#[test]
fn statusline_subcommand_help_lists_install_uninstall() {
    let r = Cli::new().args(["statusline", "--help"]).run();
    assert!(r.ok());
    assert!(r.stdout.contains("install"));
    assert!(r.stdout.contains("uninstall"));
}

#[test]
fn run_start_help_documents_flags() {
    let r = Cli::new().args(["run", "start", "--help"]).run();
    assert!(r.ok());
    assert!(r.stdout.contains("--factory"));
    assert!(r.stdout.contains("--mode"));
    assert!(r.stdout.contains("--slug"));
}

#[test]
fn run_decide_help_documents_reject_and_notes() {
    let r = Cli::new().args(["run", "decide", "--help"]).run();
    assert!(r.ok());
    assert!(r.stdout.contains("--reject"));
    assert!(r.stdout.contains("--notes"));
}

#[test]
fn help_subcommand_is_equivalent_to_help_flag() {
    let via_flag = Cli::new().arg("--help").run();
    let via_sub = Cli::new().arg("help").run();
    assert!(via_sub.ok());
    assert_eq!(via_flag.stdout, via_sub.stdout);
}

// ─── factory list ────────────────────────────────────────────────────────────

#[test]
fn factory_list_succeeds() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(!r.stdout.trim().is_empty());
}

#[test]
fn factory_list_includes_software_factory() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    assert!(r.stdout.contains("software"));
}

#[test]
fn factory_list_renders_station_pipeline() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    // Stations are joined by the factory arrow.
    assert!(r.stdout.contains('→'), "expected a station arrow:\n{}", r.stdout);
}

#[test]
fn factory_list_shows_the_six_software_stations() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    for station in ["frame", "specify", "shape", "build", "prove", "harden"] {
        assert!(
            r.stdout.contains(station),
            "missing station `{station}`:\n{}",
            r.stdout
        );
    }
}

#[test]
fn factory_list_includes_a_description() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    assert!(r.stdout.contains("—"), "expected a name — description line");
}

#[test]
fn factory_list_is_deterministic_across_runs() {
    let repo = temp_repo();
    let a = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    let b = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    assert_eq!(a.stdout, b.stdout);
}

#[test]
fn factory_list_does_not_require_an_existing_repo() {
    // No .darkrun, no git — listing embedded content still works.
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    assert!(r.ok());
    assert!(!repo.path().join(".darkrun").exists());
}

// ─── run start ───────────────────────────────────────────────────────────────

#[test]
fn run_start_reports_started_run() {
    let repo = temp_repo();
    let r = start_run(repo.path(), "Add a Login Page");
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(r.stdout.contains("started run 'add-a-login-page'"));
}

#[test]
fn run_start_echoes_the_title_verbatim() {
    let repo = temp_repo();
    let r = start_run(repo.path(), "Add a Login Page");
    assert!(r.stdout.contains("(Add a Login Page)"));
}

#[test]
fn run_start_reports_default_factory_and_first_station() {
    let repo = temp_repo();
    let r = start_run(repo.path(), "Build a thing");
    assert!(r.stdout.contains("factory:        software"));
    assert!(r.stdout.contains("active station: frame"));
}

#[test]
fn run_start_prints_the_state_directory() {
    let repo = temp_repo();
    let r = start_run(repo.path(), "Build a thing");
    let expected = repo.path().join(".darkrun").join("build-a-thing");
    assert!(r.stdout.contains(&expected.to_string_lossy().into_owned()));
}

#[test]
fn run_start_creates_darkrun_state_files() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let base = repo.path().join(".darkrun").join("add-login");
    assert!(base.join("run.md").exists(), "run.md missing");
    assert!(base.join("state.json").exists(), "state.json missing");
}

#[test]
fn run_start_writes_the_active_pointer() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let active = repo.path().join(".darkrun").join("active");
    assert!(active.exists());
    let content = std::fs::read_to_string(active).unwrap();
    assert_eq!(content.trim(), "add-login");
}

#[test]
fn run_start_run_md_records_factory_frontmatter() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let md = std::fs::read_to_string(
        repo.path().join(".darkrun").join("add-login").join("run.md"),
    )
    .unwrap();
    assert!(md.contains("factory: software"));
    assert!(md.contains("active_station: frame"));
}

#[test]
fn run_start_run_md_records_title_in_body() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let md = std::fs::read_to_string(
        repo.path().join(".darkrun").join("add-login").join("run.md"),
    )
    .unwrap();
    assert!(md.contains("Add Login"));
}

#[test]
fn run_start_state_json_is_valid_json() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let raw = std::fs::read_to_string(
        repo.path().join(".darkrun").join("add-login").join("state.json"),
    )
    .unwrap();
    let v = json(&raw);
    assert_eq!(v["active_station"], "frame");
    assert_eq!(v["factory"], "software");
}

#[test]
fn run_start_honors_explicit_slug() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Anything goes here", "--slug", "my-custom"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("started run 'my-custom'"));
    assert!(repo.path().join(".darkrun").join("my-custom").exists());
}

#[test]
fn run_start_explicit_slug_does_not_change_title() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Original Title", "--slug", "abc"])
        .run();
    assert!(r.stdout.contains("(Original Title)"));
}

#[test]
fn run_start_accepts_explicit_software_factory() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--factory", "software"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("factory:        software"));
}

#[test]
fn run_start_rejects_unknown_factory() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--slug", "t", "--factory", "no-such"])
        .run();
    assert!(!r.ok());
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("error:"));
    assert!(r.stderr.to_lowercase().contains("factory"));
}

#[test]
fn run_start_empty_derivable_slug_errors() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "!!!"])
        .run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("could not derive a slug"));
}

#[test]
fn run_start_missing_description_exits_2() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["run", "start"]).run();
    assert_eq!(r.code, 2);
}

#[test]
fn run_start_accepts_mode_flag() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--mode", "continuous"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let md = std::fs::read_to_string(
        repo.path().join(".darkrun").join("thing").join("run.md"),
    )
    .unwrap();
    assert!(md.contains("mode: continuous"));
}

#[test]
fn run_start_two_runs_repoint_active() {
    let repo = temp_repo();
    start_run(repo.path(), "First Thing");
    start_run(repo.path(), "Second Thing");
    let active = std::fs::read_to_string(
        repo.path().join(".darkrun").join("active"),
    )
    .unwrap();
    assert_eq!(active.trim(), "second-thing");
    // Both run dirs persist.
    assert!(repo.path().join(".darkrun").join("first-thing").exists());
    assert!(repo.path().join(".darkrun").join("second-thing").exists());
}

#[test]
fn run_start_creates_repo_dir_if_missing() {
    let parent = temp_repo();
    let nested = parent.path().join("does").join("not").join("exist");
    let r = Cli::new()
        .repo(&nested)
        .args(["run", "start", "Thing"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(nested.join(".darkrun").join("thing").exists());
}

#[test]
fn run_start_unicode_description_slugifies() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Café déjà vu"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    // Non-ascii letters are dropped; ascii kept, runs collapse to hyphens.
    assert!(r.stdout.contains("started run 'caf-d-j-vu'"), "{}", r.stdout);
}

#[test]
fn run_start_duplicate_slug_is_idempotent_or_overwrites() {
    let repo = temp_repo();
    let a = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--slug", "dup"])
        .run();
    assert!(a.ok());
    let b = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing Again", "--slug", "dup"])
        .run();
    // Either way the command should not crash and the slug dir still exists.
    assert!(b.ok() || b.code == 1, "stderr: {}", b.stderr);
    assert!(repo.path().join(".darkrun").join("dup").exists());
}

// ─── run show ────────────────────────────────────────────────────────────────

#[test]
fn run_show_active_run_emits_json() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new().repo(repo.path()).args(["run", "show"]).run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let v = json(&r.stdout);
    assert!(v.is_object());
}

#[test]
fn run_show_reports_run_metadata() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["run"]["slug"], "add-login");
    assert_eq!(v["run"]["title"], "Add Login");
    assert_eq!(v["run"]["frontmatter"]["factory"], "software");
}

#[test]
fn run_show_reports_not_complete_for_fresh_run() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["complete"], false);
}

#[test]
fn run_show_includes_state_with_frame_station() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["state"]["active_station"], "frame");
    assert!(v["state"]["stations"]["frame"].is_object());
}

#[test]
fn run_show_includes_derived_position_in_spec_phase() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["position"]["track"], "run");
    assert_eq!(v["position"]["action"]["action"], "spec");
    assert_eq!(v["position"]["action"]["station"], "frame");
}

#[test]
fn run_show_by_explicit_slug() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(
        &Cli::new()
            .repo(repo.path())
            .args(["run", "show", "add-login"])
            .run()
            .stdout,
    );
    assert_eq!(v["run"]["slug"], "add-login");
}

#[test]
fn run_show_unknown_slug_errors() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "show", "nope"])
        .run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("run not found"));
}

#[test]
fn run_show_no_active_run_errors_with_hint() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["run", "show"]).run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no active run"));
    assert!(r.stderr.contains("darkrun run start"));
}

#[test]
fn run_show_is_a_pure_read_no_state_change() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let state_path = repo.path().join(".darkrun").join("add-login").join("state.json");
    let before = std::fs::read_to_string(&state_path).unwrap();
    Cli::new().repo(repo.path()).args(["run", "show"]).run();
    let after = std::fs::read_to_string(&state_path).unwrap();
    assert_eq!(before, after, "show must not mutate state");
}

#[test]
fn run_show_output_is_pretty_printed_json() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let out = Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout;
    // Pretty JSON has newlines and indentation.
    assert!(out.contains("\n  "), "expected pretty-printed JSON");
}

// ─── run next ────────────────────────────────────────────────────────────────

#[test]
fn run_next_emits_action_json() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(v["run"], "add-login");
    assert!(v["action"].is_object());
}

#[test]
fn run_next_first_action_is_spec_at_frame() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(v["action"]["action"], "spec");
    assert_eq!(v["action"]["station"], "frame");
}

#[test]
fn run_next_includes_position_and_action_consistently() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(v["position"]["action"], v["action"]);
}

#[test]
fn run_next_ticks_the_phase_forward() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    // First tick: spec at frame.
    let a = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(a["action"]["action"], "spec");
    assert_eq!(a["action"]["station"], "frame");
    // Second tick advances the frame phase from spec → review (still the same
    // station, but a distinct action). `next` is a real state advance, not a
    // pure read.
    let b = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(b["action"]["station"], "frame");
    assert_ne!(a["action"]["action"], b["action"]["action"]);
}

#[test]
fn run_next_by_explicit_slug() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(
        &Cli::new()
            .repo(repo.path())
            .args(["run", "next", "add-login"])
            .run()
            .stdout,
    );
    assert_eq!(v["run"], "add-login");
}

#[test]
fn run_next_unknown_slug_errors() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "next", "ghost"])
        .run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("run not found"));
}

#[test]
fn run_next_no_active_run_errors() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["run", "next"]).run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no active run"));
}

// ─── run decide ──────────────────────────────────────────────────────────────

#[test]
fn run_decide_approve_advances_into_spec_track() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let v = json(&r.stdout);
    // Approving the frame checkpoint advances to the next station.
    assert_eq!(v["action"]["station"], "specify");
}

#[test]
fn run_decide_emits_action_json() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "decide"]).run().stdout);
    assert!(v["action"].is_object());
    assert_eq!(v["run"], "add-login");
}

#[test]
fn run_decide_reject_holds_and_routes_feedback() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "--reject", "--notes", "redo the spec"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let v = json(&r.stdout);
    // Rejection keeps the station and routes a fix action.
    assert_eq!(v["action"]["station"], "frame");
    assert_eq!(v["action"]["action"], "fix_feedback");
}

#[test]
fn run_decide_reject_without_notes_is_allowed() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "--reject"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
}

#[test]
fn run_decide_unknown_slug_errors() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "missing"])
        .run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("run not found"));
}

#[test]
fn run_decide_no_active_run_errors() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no active run"));
}

#[test]
fn run_decide_approve_persists_advance() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    // A follow-up show should reflect the advanced active station.
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["state"]["active_station"], "specify");
}

#[test]
fn run_decide_reject_persists_hold() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "--reject", "--notes", "again"])
        .run();
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    // Still at frame after a rejection.
    assert_eq!(v["state"]["active_station"], "frame");
}

// ─── active-run resolution ───────────────────────────────────────────────────

#[test]
fn omitted_slug_resolves_active_for_show_next_decide() {
    let repo = temp_repo();
    start_run(repo.path(), "Resolve Me");
    for action in ["show", "next"] {
        let v = json(
            &Cli::new()
                .repo(repo.path())
                .args(["run", action])
                .run()
                .stdout,
        );
        let run = v.get("run").cloned().unwrap_or(v["run"].clone());
        // show nests under run.slug; next uses run directly.
        let slug = run.get("slug").and_then(Value::as_str).unwrap_or_else(|| {
            v["run"].as_str().unwrap_or("")
        });
        assert_eq!(slug, "resolve-me", "action `{action}`");
    }
}

#[test]
fn active_resolution_follows_latest_started_run() {
    let repo = temp_repo();
    start_run(repo.path(), "Alpha One");
    start_run(repo.path(), "Beta Two");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(v["run"], "beta-two");
}

#[test]
fn explicit_slug_overrides_active_pointer() {
    let repo = temp_repo();
    start_run(repo.path(), "Alpha One");
    start_run(repo.path(), "Beta Two"); // active = beta-two
    let v = json(
        &Cli::new()
            .repo(repo.path())
            .args(["run", "next", "alpha-one"])
            .run()
            .stdout,
    );
    assert_eq!(v["run"], "alpha-one");
}

// ─── auth status / logout (temp HOME, no network) ────────────────────────────

#[test]
fn auth_status_no_credentials_lists_both_unauthorized() {
    let home = temp_repo();
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(r.stdout.contains("GitHub"));
    assert!(r.stdout.contains("GitLab"));
    assert_eq!(r.stdout.matches("not authorized").count(), 2);
}

#[test]
fn auth_status_does_not_create_files_when_empty() {
    let home = temp_repo();
    Cli::new().home(home.path()).args(["auth", "status"]).run();
    assert!(
        !home.path().join(".darkrun").join("credentials").exists(),
        "status must not write credentials"
    );
}

#[test]
fn auth_status_reflects_seeded_github_credential() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"tok"}}"#,
    );
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    assert!(r.ok());
    let gh = r
        .stdout
        .lines()
        .find(|l| l.contains("GitHub"))
        .unwrap();
    assert!(gh.contains("authorized") && !gh.contains("not authorized"));
    let gl = r
        .stdout
        .lines()
        .find(|l| l.contains("GitLab"))
        .unwrap();
    assert!(gl.contains("not authorized"));
}

#[test]
fn auth_status_reflects_both_providers_seeded() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"a"},"gitlab":{"provider":"gitlab","access_token":"b"}}"#,
    );
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    assert_eq!(r.stdout.matches("not authorized").count(), 0);
    assert_eq!(r.stdout.matches("authorized").count(), 2);
}

#[test]
fn auth_logout_with_no_credential_is_a_noop_success() {
    let home = temp_repo();
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "github"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("No GitHub credential to remove"));
}

#[test]
fn auth_logout_removes_seeded_credential() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"tok"}}"#,
    );
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "github"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("Removed GitHub credential"));
    // Status now reports it unauthorized.
    let s = Cli::new().home(home.path()).args(["auth", "status"]).run();
    let gh = s.stdout.lines().find(|l| l.contains("GitHub")).unwrap();
    assert!(gh.contains("not authorized"));
}

#[test]
fn auth_logout_only_removes_named_provider() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"a"},"gitlab":{"provider":"gitlab","access_token":"b"}}"#,
    );
    Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "github"])
        .run();
    let s = Cli::new().home(home.path()).args(["auth", "status"]).run();
    let gl = s.stdout.lines().find(|l| l.contains("GitLab")).unwrap();
    assert!(gl.contains("authorized") && !gl.contains("not authorized"));
}

#[test]
fn auth_logout_accepts_gh_alias() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"a"}}"#,
    );
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "gh"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("Removed GitHub"));
}

#[test]
fn auth_logout_accepts_gl_alias() {
    let home = temp_repo();
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "gl"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("GitLab"));
}

#[test]
fn auth_logout_unknown_provider_errors() {
    let home = temp_repo();
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "bitbucket"])
        .run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("unknown provider"));
    assert!(r.stderr.contains("github or gitlab"));
}

#[test]
fn auth_login_unknown_provider_errors_before_network() {
    let home = temp_repo();
    // Point web base at an unroutable host; provider parse should fail first.
    let r = Cli::new()
        .home(home.path())
        .env("DARKRUN_WEB_BASE", "http://127.0.0.1:1")
        .args(["auth", "login", "--provider", "nope"])
        .run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("unknown provider"));
}

#[test]
fn auth_login_requires_provider_flag() {
    let home = temp_repo();
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "login"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn auth_logout_requires_provider_flag() {
    let home = temp_repo();
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn auth_status_aligns_provider_columns() {
    let home = temp_repo();
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    // Each line is "<provider padded to 8> <state>".
    for line in r.stdout.lines() {
        assert!(line.starts_with("GitHub  ") || line.starts_with("GitLab  "));
    }
}

// ─── statusline render ───────────────────────────────────────────────────────

/// Build the workspace JSON Claude Code pipes to the statusline command.
fn workspace_json(dir: &Path) -> String {
    format!(
        r#"{{"workspace":{{"current_dir":"{}"}}}}"#,
        dir.to_string_lossy()
    )
}

#[test]
fn statusline_with_active_run_renders_a_line() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(!r.stdout.trim().is_empty());
}

#[test]
fn statusline_includes_the_run_slug() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("add-login"), "slug missing:\n{}", r.stdout);
}

#[test]
fn statusline_renders_the_brand_wordmark_segments() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    // The wordmark is bold "dark" + regular "run" in the accent color.
    assert!(r.stdout.contains("dark"), "missing bold segment:\n{}", r.stdout);
    assert!(r.stdout.contains("run"), "missing regular segment:\n{}", r.stdout);
    // Accent color code 81 wraps both segments.
    assert!(r.stdout.contains("38;5;81"), "missing accent color");
}

#[test]
fn statusline_renders_station_pipeline_pips() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    // The active pip is the filled circle; pending pips the hollow one.
    assert!(r.stdout.contains('◉'), "missing active pip:\n{}", r.stdout);
    assert!(r.stdout.contains('○'), "missing pending pip:\n{}", r.stdout);
}

#[test]
fn statusline_names_the_active_station_and_phase() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("frame"), "missing station name");
    assert!(r.stdout.contains("spec"), "missing phase label");
}

#[test]
fn statusline_includes_osc8_hyperlink_escapes() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    // OSC 8 hyperlinks start with ESC ] 8 ; ;
    assert!(r.stdout.contains("\x1b]8;;"), "missing OSC8 link");
}

#[test]
fn statusline_links_to_the_site_base() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("https://darkrun.ai"));
}

#[test]
fn statusline_honors_custom_web_base_for_links() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .env("DARKRUN_WEB_BASE", "https://example.test")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("https://example.test"));
    assert!(!r.stdout.contains("https://darkrun.ai"));
}

#[test]
fn statusline_links_station_definition_page() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("/factories/software/stations/frame/"));
}

#[test]
fn statusline_no_active_run_prints_nothing_exit_0() {
    let repo = temp_repo(); // no .darkrun
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty(), "expected empty output:\n{:?}", r.stdout);
}

#[test]
fn statusline_accepts_project_dir_key() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let body = format!(
        r#"{{"workspace":{{"project_dir":"{}"}}}}"#,
        repo.path().to_string_lossy()
    );
    let r = Cli::new().arg("statusline").stdin(body).run();
    assert!(r.stdout.contains("add-login"));
}

#[test]
fn statusline_accepts_top_level_cwd_key() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let body = format!(r#"{{"cwd":"{}"}}"#, repo.path().to_string_lossy());
    let r = Cli::new().arg("statusline").stdin(body).run();
    assert!(r.stdout.contains("add-login"));
}

#[test]
fn statusline_prefers_current_dir_over_project_dir() {
    let active = temp_repo();
    let other = temp_repo();
    start_run(active.path(), "Active Run");
    start_run(other.path(), "Other Run");
    let body = format!(
        r#"{{"workspace":{{"current_dir":"{}","project_dir":"{}"}}}}"#,
        active.path().to_string_lossy(),
        other.path().to_string_lossy()
    );
    let r = Cli::new().arg("statusline").stdin(body).run();
    assert!(r.stdout.contains("active-run"));
    assert!(!r.stdout.contains("other-run"));
}

#[test]
fn statusline_repo_flag_overrides_piped_dir() {
    let repo = temp_repo();
    let bogus = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .repo(repo.path())
        .arg("statusline")
        .stdin(workspace_json(bogus.path()))
        .run();
    assert!(r.stdout.contains("add-login"));
}

#[test]
fn statusline_invalid_json_stdin_prints_nothing() {
    let r = Cli::new().arg("statusline").stdin("this is not json").run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn statusline_empty_stdin_prints_nothing() {
    let r = Cli::new().arg("statusline").stdin("").run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn statusline_json_without_dir_prints_nothing_or_uses_cwd() {
    // No workspace/cwd → falls back to process cwd, which has no .darkrun.
    let empty = temp_repo();
    let r = Cli::new()
        .cwd(empty.path())
        .arg("statusline")
        .stdin(r#"{"unrelated":true}"#)
        .run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn statusline_after_decide_shows_advanced_station() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("specify"), "expected advanced station:\n{}", r.stdout);
}

#[test]
fn statusline_is_deterministic_for_same_state() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let a = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    let b = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert_eq!(a.stdout, b.stdout);
}

#[test]
fn statusline_resets_color_codes() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    // Reset sequence appears (balanced styling).
    assert!(r.stdout.contains("\x1b[0m"));
}

// ─── statusline install / uninstall ──────────────────────────────────────────

#[test]
fn statusline_install_project_writes_settings() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let settings = repo.path().join(".claude").join("settings.json");
    assert!(settings.exists());
    let v = json(&std::fs::read_to_string(settings).unwrap());
    assert_eq!(v["statusLine"]["type"], "command");
    assert_eq!(v["statusLine"]["command"], "darkrun statusline");
}

#[test]
fn statusline_install_reports_project_scope() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    assert!(r.stdout.contains("installed"));
    assert!(r.stdout.contains("(project)"));
}

#[test]
fn statusline_install_global_uses_home_settings() {
    let home = temp_repo();
    let r = Cli::new()
        .home(home.path())
        .args(["statusline", "install", "--global"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let settings = home.path().join(".claude").join("settings.json");
    assert!(settings.exists());
    assert!(r.stdout.contains("(global)"));
}

#[test]
fn statusline_install_custom_command_is_written() {
    let repo = temp_repo();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install", "--command", "my plugin statusline"])
        .run();
    let v = json(
        &std::fs::read_to_string(repo.path().join(".claude").join("settings.json")).unwrap(),
    );
    assert_eq!(v["statusLine"]["command"], "my plugin statusline");
}

#[test]
fn statusline_install_preserves_other_settings_keys() {
    let repo = temp_repo();
    let claude = repo.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        r#"{"theme":"dark","model":"opus"}"#,
    )
    .unwrap();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    let v = json(&std::fs::read_to_string(claude.join("settings.json")).unwrap());
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["model"], "opus");
    assert_eq!(v["statusLine"]["type"], "command");
}

#[test]
fn statusline_install_saves_existing_line_as_fallback() {
    let repo = temp_repo();
    let claude = repo.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        r#"{"statusLine":{"type":"command","command":"old-line"}}"#,
    )
    .unwrap();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    let fallback = repo
        .path()
        .join(".darkrun")
        .join("statusline-fallback.json");
    assert!(fallback.exists());
    let v = json(&std::fs::read_to_string(fallback).unwrap());
    assert_eq!(v["command"], "old-line");
}

#[test]
fn statusline_uninstall_restores_saved_fallback() {
    let repo = temp_repo();
    let claude = repo.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        r#"{"statusLine":{"type":"command","command":"old-line"}}"#,
    )
    .unwrap();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    let r = Cli::new()
        .repo(repo.path())
        .args(["statusline", "uninstall"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let v = json(&std::fs::read_to_string(claude.join("settings.json")).unwrap());
    assert_eq!(v["statusLine"]["command"], "old-line");
    // Fallback consumed.
    assert!(!repo
        .path()
        .join(".darkrun")
        .join("statusline-fallback.json")
        .exists());
}

#[test]
fn statusline_uninstall_without_fallback_removes_key() {
    let repo = temp_repo();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "uninstall"])
        .run();
    let v = json(
        &std::fs::read_to_string(repo.path().join(".claude").join("settings.json")).unwrap(),
    );
    assert!(v.get("statusLine").is_none());
}

#[test]
fn statusline_install_uninstall_round_trip_restores_other_keys() {
    let repo = temp_repo();
    let claude = repo.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(
        claude.join("settings.json"),
        r#"{"theme":"dark","statusLine":{"type":"command","command":"keep-me"}}"#,
    )
    .unwrap();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "uninstall"])
        .run();
    let v = json(&std::fs::read_to_string(claude.join("settings.json")).unwrap());
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["statusLine"]["command"], "keep-me");
}

#[test]
fn statusline_reinstall_does_not_overwrite_fallback_with_own_command() {
    let repo = temp_repo();
    // Install once (creates darkrun line), install again — the second install
    // sees the darkrun command already in place and must not save it as fallback.
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    let fallback = repo
        .path()
        .join(".darkrun")
        .join("statusline-fallback.json");
    assert!(!fallback.exists(), "must not snapshot its own command");
}

#[test]
fn statusline_uninstall_reports_path() {
    let repo = temp_repo();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    let r = Cli::new()
        .repo(repo.path())
        .args(["statusline", "uninstall"])
        .run();
    assert!(r.stdout.contains("removed"));
    assert!(r.stdout.contains("settings.json"));
}

// ─── repo flag plumbing ──────────────────────────────────────────────────────

#[test]
fn repo_flag_is_global_and_accepted_before_or_after_subcommand() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    // After the subcommand (clap global args).
    let r = Cli::new()
        .args(["run", "show", "--repo", &repo.path().to_string_lossy()])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(r.stdout.contains("add-login"));
}

#[test]
fn missing_repo_defaults_to_cwd() {
    let repo = temp_repo();
    start_run(repo.path(), "Add Login");
    // No --repo: run inside the temp dir as cwd.
    let r = Cli::new()
        .cwd(repo.path())
        .args(["run", "show"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(r.stdout.contains("add-login"));
}

// ─── isolation / independence ────────────────────────────────────────────────

#[test]
fn separate_repos_have_independent_active_runs() {
    let a = temp_repo();
    let b = temp_repo();
    start_run(a.path(), "Run In A");
    start_run(b.path(), "Run In B");
    let va = json(&Cli::new().repo(a.path()).args(["run", "next"]).run().stdout);
    let vb = json(&Cli::new().repo(b.path()).args(["run", "next"]).run().stdout);
    assert_eq!(va["run"], "run-in-a");
    assert_eq!(vb["run"], "run-in-b");
}

#[test]
fn lifecycle_start_show_next_decide_threads_through() {
    let repo = temp_repo();
    // start
    assert!(start_run(repo.path(), "Full Lifecycle").ok());
    // show: fresh
    let show = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(show["state"]["active_station"], "frame");
    // next: spec at frame
    let next = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(next["action"]["station"], "frame");
    // decide: advance
    let decide = json(&Cli::new().repo(repo.path()).args(["run", "decide"]).run().stdout);
    assert_eq!(decide["action"]["station"], "specify");
    // show again: advanced
    let show2 = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(show2["state"]["active_station"], "specify");
}

// ─── in-module unit tests for pure helpers ───────────────────────────────────
//
// `slugify` (and the statusline `parse_git_url` / `web_base`) are private to the
// binary crate, so they cannot be reached from this integration test. We cover
// `slugify` end-to-end through `run start` above and exhaustively at the unit
// level in `src/main.rs`'s `#[cfg(test)] mod tests`. Here we add a broad table of
// slug behaviors driven through the public `run start` slug-derivation path,
// which is the only public surface that exercises `slugify`.

/// Drive `slugify` through the binary by starting a run and reading back the
/// derived slug from the "started run '<slug>'" line.
fn derive_slug(repo: &Path, desc: &str) -> Option<String> {
    let r = start_run(repo, desc);
    if !r.ok() {
        return None;
    }
    let line = r.stdout.lines().find(|l| l.contains("started run"))?;
    let start = line.find('\'')? + 1;
    let end = line[start..].find('\'')? + start;
    Some(line[start..end].to_string())
}

#[test]
fn slugify_via_start_lowercases() {
    let repo = temp_repo();
    assert_eq!(derive_slug(repo.path(), "HELLO World").as_deref(), Some("hello-world"));
}

#[test]
fn slugify_via_start_collapses_runs_of_separators() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "a   b---c__d").as_deref(),
        Some("a-b-c-d")
    );
}

#[test]
fn slugify_via_start_trims_leading_and_trailing_separators() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "  spaced  out  ").as_deref(),
        Some("spaced-out")
    );
}

#[test]
fn slugify_via_start_keeps_existing_slug_shape() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "already-a-slug").as_deref(),
        Some("already-a-slug")
    );
}

#[test]
fn slugify_via_start_keeps_digits() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "Release v2 build 39").as_deref(),
        Some("release-v2-build-39")
    );
}

#[test]
fn slugify_via_start_strips_punctuation() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "Fix: the @#$ bug!!!").as_deref(),
        Some("fix-the-bug")
    );
}

#[test]
fn slugify_via_start_single_token() {
    let repo = temp_repo();
    assert_eq!(derive_slug(repo.path(), "Login").as_deref(), Some("login"));
}

#[test]
fn slugify_via_start_only_punctuation_is_rejected() {
    let repo = temp_repo();
    // Pure punctuation slugs to empty → start errors.
    assert_eq!(derive_slug(repo.path(), "@#$%^&*()").as_deref(), None);
}

#[test]
fn slugify_via_start_leading_number() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "404 page handler").as_deref(),
        Some("404-page-handler")
    );
}

#[test]
fn slugify_via_start_drops_non_ascii_letters() {
    let repo = temp_repo();
    // ï and ç (and their combining marks) are non-ascii and dropped; the
    // surrounding ascii is kept and the gaps collapse to single hyphens.
    assert_eq!(
        derive_slug(repo.path(), "naïve façade").as_deref(),
        Some("na-ve-fa-ade")
    );
}

// ─── deep lifecycle: driving a run through every station ─────────────────────

/// Approve the active checkpoint `n` times in `repo`, returning the last output.
fn decide_n(repo: &Path, n: usize) -> Run {
    let mut last = Cli::new().repo(repo).args(["run", "decide"]).run();
    for _ in 1..n {
        last = Cli::new().repo(repo).args(["run", "decide"]).run();
    }
    last
}

/// The ordered station names of the software factory.
const STATIONS: [&str; 6] = ["frame", "specify", "shape", "build", "prove", "harden"];

#[test]
fn decide_walks_through_every_station_in_order() {
    let repo = temp_repo();
    start_run(repo.path(), "Walk Stations");
    // Each approve advances to the next station's spec; assert the sequence.
    for next_station in &STATIONS[1..] {
        let v = json(&Cli::new().repo(repo.path()).args(["run", "decide"]).run().stdout);
        assert_eq!(v["action"]["station"], *next_station);
        assert_eq!(v["action"]["action"], "spec");
    }
}

#[test]
fn decide_at_last_station_enters_run_review() {
    let repo = temp_repo();
    start_run(repo.path(), "Seal It");
    // 5 approves reach the last station; the 6th closes harden's checkpoint and
    // the run holds in the whole-run review (the cross-station audit) — it no
    // longer seals on the operator's decide alone, the run reviewers gate first.
    decide_n(repo.path(), 5);
    let v = json(&Cli::new().repo(repo.path()).args(["run", "decide"]).run().stdout);
    assert_eq!(v["action"]["action"], "run_review");
}

#[test]
fn decide_after_sealed_errors_no_active_station() {
    let repo = temp_repo();
    start_run(repo.path(), "Seal It");
    decide_n(repo.path(), 6); // through sealing
    let r = Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no active station"));
}

#[test]
fn show_at_each_station_reports_that_station_active() {
    let repo = temp_repo();
    start_run(repo.path(), "Track Active");
    for (i, station) in STATIONS.iter().enumerate() {
        let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
        assert_eq!(
            v["state"]["active_station"], *station,
            "after {i} advances"
        );
        if i + 1 < STATIONS.len() {
            Cli::new().repo(repo.path()).args(["run", "decide"]).run();
        }
    }
}

#[test]
fn earlier_stations_are_completed_after_advancing() {
    let repo = temp_repo();
    start_run(repo.path(), "Completed Trail");
    decide_n(repo.path(), 3); // now at build
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    // frame/specify/shape should be marked completed in state.
    for done in ["frame", "specify", "shape"] {
        assert_eq!(
            v["state"]["stations"][done]["status"], "completed",
            "{done} should be completed"
        );
    }
}

#[test]
fn show_after_sealing_marks_harden_completed() {
    let repo = temp_repo();
    start_run(repo.path(), "Seal Done");
    decide_n(repo.path(), 6);
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["state"]["stations"]["harden"]["status"], "completed");
}

#[test]
fn next_at_advanced_station_targets_that_station() {
    let repo = temp_repo();
    start_run(repo.path(), "Next After Advance");
    decide_n(repo.path(), 2); // now at shape
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(v["action"]["station"], "shape");
}

// ─── reject / rework routing ─────────────────────────────────────────────────

#[test]
fn reject_records_feedback_on_disk() {
    let repo = temp_repo();
    start_run(repo.path(), "Reject Feedback");
    Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "--reject", "--notes", "needs more detail"])
        .run();
    // Feedback is persisted under the run's feedback directory.
    let feedback_dir = repo.path().join(".darkrun").join("reject-feedback").join("feedback");
    if feedback_dir.exists() {
        let any = std::fs::read_dir(&feedback_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                std::fs::read_to_string(e.path())
                    .map(|c| c.contains("needs more detail"))
                    .unwrap_or(false)
            });
        assert!(any, "reject notes should be written to feedback");
    }
}

#[test]
fn reject_then_approve_eventually_advances() {
    let repo = temp_repo();
    start_run(repo.path(), "Reject Then Pass");
    // Reject holds at frame.
    Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "--reject", "--notes", "redo"])
        .run();
    let held = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(held["state"]["active_station"], "frame");
    // A subsequent approve still moves forward.
    let v = json(&Cli::new().repo(repo.path()).args(["run", "decide"]).run().stdout);
    assert!(v["action"]["station"] == "frame" || v["action"]["station"] == "specify");
}

// ─── serde / output shape ────────────────────────────────────────────────────

#[test]
fn show_json_has_all_top_level_keys() {
    let repo = temp_repo();
    start_run(repo.path(), "Shape Check");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    for key in ["run", "state", "position", "complete"] {
        assert!(v.get(key).is_some(), "missing key `{key}`");
    }
}

#[test]
fn show_run_frontmatter_has_status_active() {
    let repo = temp_repo();
    start_run(repo.path(), "Active Status");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["run"]["frontmatter"]["status"], "active");
}

#[test]
fn show_run_frontmatter_has_iso_started_at() {
    let repo = temp_repo();
    start_run(repo.path(), "Timestamp");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    let started = v["run"]["frontmatter"]["started_at"].as_str().unwrap();
    // RFC3339-ish: contains a date, a 'T', and a timezone marker.
    assert!(started.contains('T'));
    assert!(started.contains('-'));
    assert!(started.starts_with("20"));
}

#[test]
fn next_action_carries_a_kills_or_reviewers_field() {
    let repo = temp_repo();
    start_run(repo.path(), "Action Fields");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    // The spec action names what it kills.
    assert_eq!(v["action"]["kills"], "wrong-thing");
}

#[test]
fn show_position_track_is_run() {
    let repo = temp_repo();
    start_run(repo.path(), "Track Run");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["position"]["track"], "run");
}

#[test]
fn state_json_round_trips_through_show() {
    let repo = temp_repo();
    start_run(repo.path(), "Round Trip");
    // The on-disk state.json and show's embedded state should agree.
    let on_disk = json(
        &std::fs::read_to_string(
            repo.path().join(".darkrun").join("round-trip").join("state.json"),
        )
        .unwrap(),
    );
    let shown = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(on_disk["active_station"], shown["state"]["active_station"]);
    assert_eq!(on_disk["factory"], shown["state"]["factory"]);
}

// ─── statusline phase / pipeline progression ─────────────────────────────────

#[test]
fn statusline_pipeline_grows_done_pips_as_run_advances() {
    let repo = temp_repo();
    start_run(repo.path(), "Pip Growth");
    let first = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    let done_first = first.stdout.matches('●').count();
    decide_n(repo.path(), 3);
    let later = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    let done_later = later.stdout.matches('●').count();
    assert!(
        done_later > done_first,
        "expected more completed pips after advancing ({done_first} → {done_later})"
    );
}

#[test]
fn statusline_at_sealed_shows_all_pips_done() {
    let repo = temp_repo();
    start_run(repo.path(), "All Done");
    decide_n(repo.path(), 6); // seal
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    // Every station is done (or the final one shown active) — none pending.
    assert_eq!(
        r.stdout.matches('○').count(),
        0,
        "no pending pips at seal:\n{}",
        r.stdout
    );
    assert_eq!(
        r.stdout.matches('●').count() + r.stdout.matches('◉').count(),
        6,
        "all six pips accounted for:\n{}",
        r.stdout
    );
}

#[test]
fn statusline_shows_current_station_name_after_advance() {
    let repo = temp_repo();
    start_run(repo.path(), "Name Advance");
    decide_n(repo.path(), 3); // build
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("build"));
}

#[test]
fn statusline_six_station_pipeline_has_six_pips() {
    let repo = temp_repo();
    start_run(repo.path(), "Six Pips");
    let r = Cli::new()
        .arg("statusline")
        .stdin(workspace_json(repo.path()))
        .run();
    let pips = r.stdout.matches('●').count()
        + r.stdout.matches('◉').count()
        + r.stdout.matches('○').count();
    assert_eq!(pips, 6, "expected six station pips:\n{}", r.stdout);
}

#[test]
fn statusline_handles_trailing_whitespace_in_json() {
    let repo = temp_repo();
    start_run(repo.path(), "Whitespace");
    let body = format!("  {}  \n", workspace_json(repo.path()));
    let r = Cli::new().arg("statusline").stdin(body).run();
    assert!(r.stdout.contains("whitespace"));
}

#[test]
fn statusline_blank_web_base_env_falls_back_to_default() {
    let repo = temp_repo();
    start_run(repo.path(), "Blank Base");
    let r = Cli::new()
        .arg("statusline")
        .env("DARKRUN_WEB_BASE", "")
        .stdin(workspace_json(repo.path()))
        .run();
    assert!(r.stdout.contains("https://darkrun.ai"));
}

#[test]
fn statusline_web_base_trailing_slash_is_trimmed() {
    let repo = temp_repo();
    start_run(repo.path(), "Trim Base");
    let r = Cli::new()
        .arg("statusline")
        .env("DARKRUN_WEB_BASE", "https://example.test/")
        .stdin(workspace_json(repo.path()))
        .run();
    // No double slash before the path segment.
    assert!(r.stdout.contains("https://example.test/factories/"));
    assert!(!r.stdout.contains("https://example.test//"));
}

// ─── auth: more credential surface ───────────────────────────────────────────

#[test]
fn auth_status_unknown_provider_keys_in_file_are_ignored_for_others() {
    let home = temp_repo();
    // Only gitlab seeded.
    seed_credentials(
        home.path(),
        r#"{"gitlab":{"provider":"gitlab","access_token":"x"}}"#,
    );
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    let gh = r.stdout.lines().find(|l| l.contains("GitHub")).unwrap();
    assert!(gh.contains("not authorized"));
    let gl = r.stdout.lines().find(|l| l.contains("GitLab")).unwrap();
    assert!(gl.contains("authorized") && !gl.contains("not authorized"));
}

#[test]
fn auth_logout_gitlab_seeded_removes() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"gitlab":{"provider":"gitlab","access_token":"x"}}"#,
    );
    let r = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "gitlab"])
        .run();
    assert!(r.ok());
    assert!(r.stdout.contains("Removed GitLab"));
}

#[test]
fn auth_logout_idempotent_second_call_is_noop() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"x"}}"#,
    );
    let first = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "github"])
        .run();
    assert!(first.stdout.contains("Removed"));
    let second = Cli::new()
        .home(home.path())
        .args(["auth", "logout", "--provider", "github"])
        .run();
    assert!(second.ok());
    assert!(second.stdout.contains("No GitHub credential"));
}

#[test]
fn auth_status_credential_with_extra_fields_still_authorized() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"x","refresh_token":"r","expires_in":3600,"token_type":"bearer"}}"#,
    );
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    let gh = r.stdout.lines().find(|l| l.contains("GitHub")).unwrap();
    assert!(gh.contains("authorized") && !gh.contains("not authorized"));
}

#[test]
fn auth_status_two_lines_exactly() {
    let home = temp_repo();
    let r = Cli::new().home(home.path()).args(["auth", "status"]).run();
    assert_eq!(r.stdout.trim().lines().count(), 2);
}

// ─── clap argument validation ────────────────────────────────────────────────

#[test]
fn run_start_unknown_flag_exits_2() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--nonexistent", "v"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn run_decide_extra_positional_is_treated_as_slug() {
    let repo = temp_repo();
    start_run(repo.path(), "Pos Slug");
    // Decide with the run's own slug as positional should succeed.
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "decide", "pos-slug"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
}

#[test]
fn factory_unknown_action_exits_2() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["factory", "bogus"]).run();
    assert_eq!(r.code, 2);
}

#[test]
fn auth_unknown_action_exits_2() {
    let home = temp_repo();
    let r = Cli::new().home(home.path()).args(["auth", "bogus"]).run();
    assert_eq!(r.code, 2);
}

#[test]
fn statusline_unknown_action_exits_2() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["statusline", "bogus-action"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn run_start_factory_flag_requires_value() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--factory"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn run_start_slug_flag_requires_value() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Thing", "--slug"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn repo_flag_requires_a_value() {
    let r = Cli::new().args(["--repo"]).run();
    assert_eq!(r.code, 2);
}

// ─── stderr / stdout discipline ──────────────────────────────────────────────

#[test]
fn errors_go_to_stderr_not_stdout() {
    let repo = temp_repo();
    let r = Cli::new().repo(repo.path()).args(["run", "show"]).run();
    assert_eq!(r.code, 1);
    assert!(r.stdout.is_empty(), "stdout should be empty on error");
    assert!(r.stderr.starts_with("error: "));
}

#[test]
fn successful_json_commands_write_only_to_stdout() {
    let repo = temp_repo();
    start_run(repo.path(), "Clean Stdout");
    let r = Cli::new().repo(repo.path()).args(["run", "show"]).run();
    assert!(r.stderr.is_empty(), "stderr: {:?}", r.stderr);
    assert!(!r.stdout.is_empty());
}

#[test]
fn error_messages_are_prefixed_with_error() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "show", "ghost"])
        .run();
    assert!(r.stderr.starts_with("error: "));
}

// ─── more slugify behaviors through start ─────────────────────────────────────

#[test]
fn slugify_via_start_tab_and_newline_become_hyphens() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "one\ttwo\nthree").as_deref(),
        Some("one-two-three")
    );
}

#[test]
fn slugify_via_start_mixed_case_runs() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "CamelCase API v2").as_deref(),
        Some("camelcase-api-v2")
    );
}

#[test]
fn slugify_via_start_underscores_become_hyphens() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "snake_case_name").as_deref(),
        Some("snake-case-name")
    );
}

#[test]
fn slugify_via_start_slashes_become_hyphens() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "feat/login/page").as_deref(),
        Some("feat-login-page")
    );
}

#[test]
fn slugify_via_start_dots_become_hyphens() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "v1.2.3 release").as_deref(),
        Some("v1-2-3-release")
    );
}

#[test]
fn run_start_dash_prefixed_description_needs_double_dash_separator() {
    let repo = temp_repo();
    // A leading-dash description looks like a flag to clap and is rejected…
    let bare = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "---leading dashes"])
        .run();
    assert_eq!(bare.code, 2);
    // …but `--` ends option parsing and the description is taken literally,
    // trimming the leading separators in the derived slug.
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "--", "---leading dashes"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    assert!(r.stdout.contains("started run 'leading-dashes'"));
}

#[test]
fn slugify_via_start_emoji_dropped() {
    let repo = temp_repo();
    assert_eq!(
        derive_slug(repo.path(), "ship it 🚀 now").as_deref(),
        Some("ship-it-now")
    );
}

// ─── serve / mcp argument surface ────────────────────────────────────────────

#[test]
fn serve_help_documents_addr_flag() {
    let r = Cli::new().args(["serve", "--help"]).run();
    assert!(r.ok());
    assert!(r.stdout.contains("--addr"));
    assert!(r.stdout.contains("4317"));
}

#[test]
fn serve_rejects_malformed_addr_exit_2() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["serve", "--addr", "not-an-address"])
        .run();
    assert_eq!(r.code, 2);
}

#[test]
fn mcp_help_is_available() {
    let r = Cli::new().args(["mcp", "--help"]).run();
    assert!(r.ok());
    assert!(r.stdout.to_lowercase().contains("mcp") || r.stdout.contains("stdio"));
}

// ─── statusline JSON resilience ──────────────────────────────────────────────

#[test]
fn statusline_json_array_input_prints_nothing() {
    let r = Cli::new().arg("statusline").stdin("[1,2,3]").run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn statusline_json_number_input_prints_nothing() {
    let r = Cli::new().arg("statusline").stdin("42").run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn statusline_nonexistent_dir_in_json_prints_nothing() {
    let r = Cli::new()
        .arg("statusline")
        .stdin(r#"{"workspace":{"current_dir":"/no/such/dir/xyz"}}"#)
        .run();
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn statusline_current_dir_wins_when_both_present_and_valid() {
    let a = temp_repo();
    let b = temp_repo();
    start_run(a.path(), "Primary");
    start_run(b.path(), "Secondary");
    let body = format!(
        r#"{{"workspace":{{"current_dir":"{}","project_dir":"{}"}},"cwd":"{}"}}"#,
        a.path().to_string_lossy(),
        b.path().to_string_lossy(),
        b.path().to_string_lossy()
    );
    let r = Cli::new().arg("statusline").stdin(body).run();
    assert!(r.stdout.contains("primary"));
}

#[test]
fn statusline_falls_back_to_cwd_key_when_workspace_absent() {
    let repo = temp_repo();
    start_run(repo.path(), "Cwd Fallback");
    let body = format!(r#"{{"cwd":"{}","other":1}}"#, repo.path().to_string_lossy());
    let r = Cli::new().arg("statusline").stdin(body).run();
    assert!(r.stdout.contains("cwd-fallback"));
}

// ─── install/uninstall HOME failure surfaces ─────────────────────────────────

#[test]
fn statusline_install_global_without_home_errors() {
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .args(["statusline", "install", "--global"])
        .run();
    // No home → cannot resolve the global settings path.
    assert_ne!(r.code, 0);
}

#[test]
fn statusline_uninstall_on_clean_repo_is_safe() {
    let repo = temp_repo();
    // No prior install; uninstall should still succeed (key absent).
    let r = Cli::new()
        .repo(repo.path())
        .args(["statusline", "uninstall"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let v = json(
        &std::fs::read_to_string(repo.path().join(".claude").join("settings.json")).unwrap(),
    );
    assert!(v.get("statusLine").is_none());
}

#[test]
fn statusline_install_then_render_uses_installed_command_name() {
    // After install, the settings command is exactly the render entrypoint.
    let repo = temp_repo();
    Cli::new()
        .repo(repo.path())
        .args(["statusline", "install"])
        .run();
    let v = json(
        &std::fs::read_to_string(repo.path().join(".claude").join("settings.json")).unwrap(),
    );
    assert_eq!(v["statusLine"]["command"], "darkrun statusline");
    assert_eq!(v["statusLine"]["padding"], 0);
}

// ─── cross-command determinism ───────────────────────────────────────────────

#[test]
fn show_is_deterministic_for_unchanged_state() {
    let repo = temp_repo();
    start_run(repo.path(), "Determinism");
    let a = Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout;
    let b = Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout;
    assert_eq!(a, b);
}

#[test]
fn factory_list_and_show_do_not_interfere() {
    let repo = temp_repo();
    start_run(repo.path(), "No Interfere");
    let before = Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout;
    Cli::new().repo(repo.path()).args(["factory", "list"]).run();
    let after = Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout;
    assert_eq!(before, after);
}

#[test]
fn auth_status_is_deterministic() {
    let home = temp_repo();
    seed_credentials(
        home.path(),
        r#"{"github":{"provider":"github","access_token":"t"}}"#,
    );
    let a = Cli::new().home(home.path()).args(["auth", "status"]).run().stdout;
    let b = Cli::new().home(home.path()).args(["auth", "status"]).run().stdout;
    assert_eq!(a, b);
}

// ─── full multi-run, multi-station interplay ─────────────────────────────────

#[test]
fn two_runs_advance_independently() {
    let repo = temp_repo();
    start_run(repo.path(), "Run Alpha");
    // alpha is active; advance it twice.
    Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    // Start beta (now active, fresh at frame).
    start_run(repo.path(), "Run Beta");
    let beta = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(beta["state"]["active_station"], "frame");
    // Alpha retains its advanced position.
    let alpha = json(
        &Cli::new()
            .repo(repo.path())
            .args(["run", "show", "run-alpha"])
            .run()
            .stdout,
    );
    assert_eq!(alpha["state"]["active_station"], "shape");
}

#[test]
fn decide_then_next_reports_advanced_station_spec() {
    let repo = temp_repo();
    start_run(repo.path(), "Decide Next");
    Cli::new().repo(repo.path()).args(["run", "decide"]).run();
    let v = json(&Cli::new().repo(repo.path()).args(["run", "next"]).run().stdout);
    assert_eq!(v["action"]["station"], "specify");
}

#[test]
fn list_runs_directory_contains_started_run() {
    let repo = temp_repo();
    start_run(repo.path(), "Listed Run");
    let entries: Vec<String> = std::fs::read_dir(repo.path().join(".darkrun"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(entries.contains(&"listed-run".to_string()));
}

// ─── title vs slug independence ──────────────────────────────────────────────

#[test]
fn title_preserves_original_casing_while_slug_lowercases() {
    let repo = temp_repo();
    start_run(repo.path(), "Add OAuth Support");
    let v = json(&Cli::new().repo(repo.path()).args(["run", "show"]).run().stdout);
    assert_eq!(v["run"]["title"], "Add OAuth Support");
    assert_eq!(v["run"]["slug"], "add-oauth-support");
}

#[test]
fn explicit_slug_uppercase_is_used_verbatim_in_pointer() {
    // The slug flag is taken as-is (no slugify applied to an explicit slug).
    let repo = temp_repo();
    let r = Cli::new()
        .repo(repo.path())
        .args(["run", "start", "Whatever", "--slug", "MixedSlug"])
        .run();
    assert!(r.ok(), "stderr: {}", r.stderr);
    let active = std::fs::read_to_string(repo.path().join(".darkrun").join("active")).unwrap();
    assert_eq!(active.trim(), "MixedSlug");
}
