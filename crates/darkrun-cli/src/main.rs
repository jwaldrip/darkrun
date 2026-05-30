//! darkrun — the command-line entry point.
//!
//! Assembles the engine crates behind a single `darkrun` binary:
//!
//! - `darkrun mcp`              — serve the manager over stdio (MCP), co-hosting the HTTP/WS review server in-process.
//! - `darkrun serve`           — serve the HTTP/WebSocket review server (axum) standalone (remote/headless case).
//! - `darkrun run start <desc>` — seed a new run at the factory's first station.
//! - `darkrun run next [slug]` — tick the manager and print the next action.
//! - `darkrun run show [slug]` — print a run's current state + derived position.
//! - `darkrun run decide [slug]` — approve (or `--reject`) the active Checkpoint.
//! - `darkrun run pr [slug]`   — open a PR/MR for a run at its external Checkpoint.
//! - `darkrun auth login`      — website-brokered OAuth login (GitHub/GitLab).
//! - `darkrun auth status`     — show which providers are authed.
//! - `darkrun auth logout`     — remove a stored credential.
//! - `darkrun factory list`    — list the embedded factories and their stations.
//! - `darkrun statusline`      — render the Claude Code status line (+ install/uninstall).
//!
//! The `run` subcommands drive on-disk `.darkrun/` state via darkrun-mcp's
//! manager (a pure read over darkrun-core state). When a slug is omitted they
//! resolve the **active run** (the `.darkrun/active` pointer). All commands root
//! their state at the current working directory unless `--repo` overrides it.

mod auth;
mod hook;
mod pr;
mod statusline;
mod verify;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use darkrun_core::{run_is_complete, StateStore};
use darkrun_mcp::{checkpoint_decide, list_factories, run_start, run_tick};

/// darkrun: a software factory that drives a Run through ordered stations.
#[derive(Debug, Parser)]
#[command(name = "darkrun", version, about, long_about = None)]
struct Cli {
    /// Repository root whose `.darkrun/` directory holds run state.
    #[arg(long, global = true, value_name = "DIR")]
    repo: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the stdio MCP server (the manager), co-hosting the HTTP/WS review
    /// server in-process on a shared in-memory session registry.
    Mcp(McpArgs),
    /// Start the axum HTTP + WebSocket review server.
    Serve(ServeArgs),
    /// Drive a run through the factory.
    #[command(subcommand)]
    Run(RunCommand),
    /// Authenticate to a version-control provider (GitHub / GitLab).
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Inspect embedded factory content.
    #[command(subcommand)]
    Factory(FactoryCommand),
    /// Capture objective verification evidence (the Prove station's NUMBERS).
    #[command(subcommand)]
    Verify(VerifyCommand),
    /// Run the HTTP load harness against a target and print a BenchProof.
    Bench(BenchArgs),
    /// Render or wire the Claude Code status line.
    Statusline(StatuslineArgs),
    /// Run a plugin hook handler (invoked by Claude Code; advisory, never blocks).
    Hook {
        /// The hook name (e.g. redirect-plan-mode, inject-state-file).
        name: String,
        /// Trailing args from Claude Code are tolerated and ignored.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Address to bind, e.g. 127.0.0.1:4317.
    #[arg(long, default_value = "127.0.0.1:4317")]
    addr: SocketAddr,
}

#[derive(Debug, Args)]
struct McpArgs {
    /// Address the in-process HTTP/WS review server binds. Overrides
    /// DARKRUN_PORT; defaults to 127.0.0.1:4317 when neither is set.
    #[arg(long)]
    addr: Option<SocketAddr>,
}

#[derive(Debug, Subcommand)]
enum RunCommand {
    /// Start a new run from a one-line description.
    Start {
        /// What this run is about (becomes the title; slug is derived).
        description: String,
        /// Factory (methodology) to drive the run.
        #[arg(long, default_value = "software")]
        factory: String,
        /// Run sizing mode.
        #[arg(long, default_value = "continuous")]
        mode: String,
        /// Explicit slug (otherwise derived from the description).
        #[arg(long)]
        slug: Option<String>,
    },
    /// Tick the manager and print the next action.
    Next {
        /// Run slug (defaults to the active run).
        slug: Option<String>,
    },
    /// Print a run's current state and derived next action.
    Show {
        /// Run slug (defaults to the active run).
        slug: Option<String>,
    },
    /// Decide the current Checkpoint — approve to advance, or `--reject` to
    /// route rework back as drift.
    Decide {
        /// Run slug (defaults to the active run).
        slug: Option<String>,
        /// Reject instead of approve (holds the station; routes feedback).
        #[arg(long)]
        reject: bool,
        /// Notes recorded with the decision (the rework feedback on reject).
        #[arg(long)]
        notes: Option<String>,
    },
    /// Open a Pull Request (GitHub) / Merge Request (GitLab) for a run sitting
    /// at its external Checkpoint, using the stored credential.
    Pr {
        /// Run slug (defaults to the active run).
        slug: Option<String>,
        /// Source branch (defaults to the repo's current branch).
        #[arg(long)]
        head: Option<String>,
        /// Target branch (defaults to the repo's default branch).
        #[arg(long)]
        base: Option<String>,
        /// Git remote to read coordinates from.
        #[arg(long, default_value = "origin")]
        remote: String,
    },
}

/// `darkrun auth` — website-brokered OAuth.
#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Open the browser to authorize, then store the returned token.
    Login {
        /// Which provider to authorize against.
        #[arg(long, value_name = "github|gitlab")]
        provider: String,
    },
    /// Show which providers currently have a stored credential.
    Status,
    /// Remove a stored credential.
    Logout {
        /// Which provider's credential to remove.
        #[arg(long, value_name = "github|gitlab")]
        provider: String,
    },
}

#[derive(Debug, Subcommand)]
enum FactoryCommand {
    /// List the embedded factories and their stations.
    List,
}

/// `darkrun verify` — surface-routed objective evidence.
#[derive(Debug, Subcommand)]
enum VerifyCommand {
    /// Drive a real headless browser over a URL: capture web vitals + a11y
    /// audits + a screenshot, and print the WebProof JSON.
    Web(VerifyWebArgs),
}

#[derive(Debug, Args)]
struct VerifyWebArgs {
    /// The URL to capture (http(s):// / file:// / data:).
    url: String,
    /// Write the proof JSON here (also prints to stdout).
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Where to save the screenshot PNG (defaults to <out>.png, else proof.png).
    #[arg(long, value_name = "FILE")]
    shot: Option<PathBuf>,
    /// Tag the proof with this surface (web-ui|desktop|mobile). When omitted,
    /// the bare WebProof is printed instead of a surface-tagged Proof.
    #[arg(long)]
    surface: Option<String>,
    /// Viewport width in CSS pixels.
    #[arg(long, default_value_t = 1280)]
    width: u32,
    /// Viewport height in CSS pixels.
    #[arg(long, default_value_t = 800)]
    height: u32,
    /// Settle delay (ms) after navigation for paint/layout metrics to record.
    #[arg(long, default_value_t = 800)]
    settle_ms: u64,
    /// Overall capture timeout (seconds).
    #[arg(long, default_value_t = 30)]
    timeout_s: u64,
}

#[derive(Debug, Args)]
struct BenchArgs {
    /// The HTTP target to load (http(s):// URL).
    target: String,
    /// Write the proof JSON here (also prints to stdout).
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Tag the proof with this surface (library|api|data). When omitted, the
    /// bare BenchProof is printed instead of a surface-tagged Proof.
    #[arg(long)]
    surface: Option<String>,
    /// Total number of requests to issue.
    #[arg(long, default_value_t = 100)]
    requests: u64,
    /// Maximum in-flight requests at once.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,
    /// Per-request timeout (seconds).
    #[arg(long, default_value_t = 10)]
    timeout_s: u64,
}

#[derive(Debug, Args)]
struct StatuslineArgs {
    #[command(subcommand)]
    action: Option<StatuslineAction>,
}

#[derive(Debug, Subcommand)]
enum StatuslineAction {
    /// Point Claude Code's `statusLine` at darkrun (saving any existing line).
    Install {
        /// Wire the user-level `~/.claude/settings.json` instead of the project.
        #[arg(long)]
        global: bool,
        /// The command Claude Code runs (override for plugin installs).
        #[arg(long, default_value = "darkrun statusline")]
        command: String,
    },
    /// Restore the status line that was in place before `install`.
    Uninstall {
        /// Operate on the user-level settings.
        #[arg(long)]
        global: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let Cli { repo, command } = cli;
    match command {
        // Statusline resolves its own root (it prefers the cwd Claude Code
        // pipes in on stdin), so it bypasses the process-cwd default.
        Command::Statusline(args) => statusline_command(repo, args),
        // Hooks are advisory and must never block a tool — handle early, resolve
        // their own cwd, and always succeed.
        Command::Hook { name, .. } => {
            hook::run(&name);
            Ok(())
        }
        // Verification commands measure an external target (a URL), not the
        // repo's `.darkrun/` state — handle them without resolving a repo root.
        Command::Verify(cmd) => verify_command(cmd),
        Command::Bench(args) => verify::bench_command(
            args.target,
            args.out,
            args.surface,
            args.requests,
            args.concurrency,
            args.timeout_s,
        ),
        other => {
            let repo_root = match repo {
                Some(p) => p,
                None => std::env::current_dir()?,
            };
            match other {
                Command::Mcp(args) => serve_mcp(repo_root, args.addr),
                Command::Serve(args) => serve_http(repo_root, args.addr),
                Command::Run(cmd) => run_command(&repo_root, cmd),
                Command::Auth(cmd) => auth_command(cmd),
                Command::Factory(cmd) => factory_command(cmd),
                Command::Statusline(_)
                | Command::Hook { .. }
                | Command::Verify(_)
                | Command::Bench(_) => unreachable!("handled above"),
            }
        }
    }
}

/// Block on the stdio MCP server, which co-hosts the HTTP/WS review server
/// in-process. With an explicit `--addr`, bind there; otherwise the server
/// resolves DARKRUN_PORT (or the 127.0.0.1:4317 default).
fn serve_mcp(repo_root: PathBuf, addr: Option<SocketAddr>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    match addr {
        Some(addr) => runtime.block_on(darkrun_mcp::serve_stdio_on(repo_root, addr))?,
        None => runtime.block_on(darkrun_mcp::serve_stdio(repo_root))?,
    }
    Ok(())
}

/// Block on the axum HTTP/WS review server.
fn serve_http(repo_root: PathBuf, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let store = StateStore::new(&repo_root);
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(darkrun_http::serve(addr, store))?;
    Ok(())
}

/// Resolve an optional slug to a concrete run, falling back to the active run.
fn resolve_slug(
    store: &StateStore,
    slug: Option<String>,
) -> Result<String, Box<dyn std::error::Error>> {
    match slug {
        Some(s) => Ok(s),
        None => store.active_run()?.ok_or_else(|| {
            "no active run — pass a slug or start one with `darkrun run start <desc>`".into()
        }),
    }
}

/// Handle the `run` subcommands.
fn run_command(repo_root: &Path, cmd: RunCommand) -> Result<(), Box<dyn std::error::Error>> {
    let store = StateStore::new(repo_root);
    match cmd {
        RunCommand::Start {
            description,
            factory,
            mode,
            slug,
        } => {
            let slug = slug.unwrap_or_else(|| slugify(&description));
            if slug.is_empty() {
                return Err("could not derive a slug from the description".into());
            }
            let run = run_start(&store, &slug, &factory, Some(description), &mode)?;
            store.set_active_run(&run.slug)?;
            println!("started run '{}' ({})", run.slug, run.title);
            println!("  factory:        {}", run.frontmatter.factory);
            println!("  active station: {}", run.frontmatter.active_station);
            println!("  state:          {}", store.run_dir(&run.slug).display());
            Ok(())
        }
        RunCommand::Next { slug } => {
            let slug = resolve_slug(&store, slug)?;
            let tick = run_tick(&store, &slug)?;
            print_json(&tick)
        }
        RunCommand::Show { slug } => {
            let slug = resolve_slug(&store, slug)?;
            let run = store.read_run(&slug)?;
            let state = store.read_state(&slug)?;
            let position = darkrun_mcp::derive_position(&store, &slug).ok();
            let show = serde_json::json!({
                "run": run,
                "state": state,
                "position": position,
                "complete": run_is_complete(&run),
            });
            print_json(&show)
        }
        RunCommand::Decide {
            slug,
            reject,
            notes,
        } => {
            let slug = resolve_slug(&store, slug)?;
            let tick = checkpoint_decide(&store, &slug, !reject, notes)?;
            print_json(&tick)
        }
        RunCommand::Pr {
            slug,
            head,
            base,
            remote,
        } => {
            let slug = resolve_slug(&store, slug)?;
            let cred_store = darkrun_vcs::CredentialStore::default_path()?;
            let transport = auth::ReqwestTransport::new()?;
            let facts = pr::GitCliFacts::new(repo_root.to_path_buf(), remote);
            let cr = pr::create_for_run(
                &transport,
                &facts,
                &store,
                &cred_store,
                &slug,
                head,
                base,
            )?;
            println!("Opened {} #{} for run '{}'", cr.provider.display_name(), cr.number, slug);
            println!("  {}", cr.url);
            Ok(())
        }
    }
}

/// Handle the `auth` subcommands.
fn auth_command(cmd: AuthCommand) -> Result<(), Box<dyn std::error::Error>> {
    let store = darkrun_vcs::CredentialStore::default_path()?;
    match cmd {
        AuthCommand::Login { provider } => {
            let provider = auth::parse_provider(&provider)?;
            auth::login(provider, &store)
        }
        AuthCommand::Status => auth::status(&store),
        AuthCommand::Logout { provider } => {
            let provider = auth::parse_provider(&provider)?;
            auth::logout(provider, &store)?;
            Ok(())
        }
    }
}

/// Handle the `factory` subcommands.
fn factory_command(cmd: FactoryCommand) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        FactoryCommand::List => {
            let names = darkrun_content::list_factories();
            if names.is_empty() {
                // Fall back to the manager's built-in plan if the embedded
                // content fails to load.
                for f in list_factories() {
                    println!("{}", f.name);
                }
                return Ok(());
            }
            for name in names {
                match darkrun_content::load_factory(&name) {
                    Ok(factory) => {
                        let fm = &factory.frontmatter;
                        if fm.description.is_empty() {
                            println!("{}", fm.name);
                        } else {
                            println!("{}  —  {}", fm.name, fm.description);
                        }
                        let stations: Vec<&str> =
                            factory.stations.iter().map(|s| s.name()).collect();
                        if !stations.is_empty() {
                            println!("    {}", stations.join(" → "));
                        }
                    }
                    Err(_) => println!("{name}"),
                }
            }
            Ok(())
        }
    }
}

/// Handle the `verify` subcommands.
fn verify_command(cmd: VerifyCommand) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        VerifyCommand::Web(args) => verify::verify_web_command(
            args.url,
            args.out,
            args.shot,
            args.surface,
            args.width,
            args.height,
            args.settle_ms,
            args.timeout_s,
        ),
    }
}

/// Handle the `statusline` command.
fn statusline_command(
    repo: Option<PathBuf>,
    args: StatuslineArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.action {
        None => {
            if let Some(line) = statusline::render(repo) {
                println!("{line}");
            }
            Ok(())
        }
        Some(StatuslineAction::Install { global, command }) => {
            let repo_root = resolve_repo(repo)?;
            statusline::install(global, &repo_root, &command)
        }
        Some(StatuslineAction::Uninstall { global }) => {
            let repo_root = resolve_repo(repo)?;
            statusline::uninstall(global, &repo_root)
        }
    }
}

/// Resolve a repo root for commands that need a concrete project directory.
fn resolve_repo(repo: Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match repo {
        Some(p) => Ok(p),
        None => Ok(std::env::current_dir()?),
    }
}

/// Pretty-print a serializable value as JSON to stdout.
fn print_json<T: serde::Serialize>(value: &T) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Derive a URL-safe slug from free text: lowercase, alphanumerics kept,
/// every run of other characters collapsed to a single hyphen, trimmed.
fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(slugify("Add a Login Page!"), "add-a-login-page");
        assert_eq!(slugify("  spaced  out  "), "spaced-out");
        assert_eq!(slugify("already-slug"), "already-slug");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn slugify_lowercases_ascii() {
        assert_eq!(slugify("HELLO"), "hello");
        assert_eq!(slugify("MixedCase"), "mixedcase");
    }

    #[test]
    fn slugify_keeps_digits() {
        assert_eq!(slugify("v2 build 39"), "v2-build-39");
        assert_eq!(slugify("404"), "404");
    }

    #[test]
    fn slugify_collapses_consecutive_separators() {
        assert_eq!(slugify("a___b---c   d"), "a-b-c-d");
        assert_eq!(slugify("a/b\\c.d:e"), "a-b-c-d-e");
    }

    #[test]
    fn slugify_trims_leading_and_trailing_separators() {
        assert_eq!(slugify("---abc---"), "abc");
        assert_eq!(slugify("   abc   "), "abc");
        assert_eq!(slugify("...abc..."), "abc");
    }

    #[test]
    fn slugify_empty_and_separator_only_yields_empty() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("   "), "");
        assert_eq!(slugify("-_-_-"), "");
        assert_eq!(slugify("@#$%"), "");
    }

    #[test]
    fn slugify_drops_non_ascii_alphanumerics() {
        // Accented letters and their combining marks are not ascii alphanumeric.
        assert_eq!(slugify("café"), "caf");
        assert_eq!(slugify("naïve"), "na-ve");
        assert_eq!(slugify("über cool"), "ber-cool");
        assert_eq!(slugify("🚀 ship"), "ship");
    }

    #[test]
    fn slugify_single_token_unchanged() {
        assert_eq!(slugify("login"), "login");
        assert_eq!(slugify("a"), "a");
    }

    #[test]
    fn slugify_does_not_start_with_dash_after_leading_junk() {
        // A separator before any alphanumeric must not introduce a leading dash.
        let s = slugify("   hello");
        assert!(!s.starts_with('-'));
        assert_eq!(s, "hello");
    }

    #[test]
    fn slugify_is_idempotent_on_its_own_output() {
        for input in ["Add a Login Page!", "v1.2.3", "  trim  ", "snake_case"] {
            let once = slugify(input);
            let twice = slugify(&once);
            assert_eq!(once, twice, "slugify not idempotent for {input:?}");
        }
    }

    #[test]
    fn slugify_output_is_url_safe() {
        for input in ["Add a Login Page!", "feat/login", "v1.2.3", "café déjà"] {
            let s = slugify(input);
            assert!(
                s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "non-url-safe slug {s:?} from {input:?}"
            );
        }
    }

    #[test]
    fn slugify_newlines_and_tabs_are_separators() {
        assert_eq!(slugify("one\ttwo\nthree"), "one-two-three");
    }
}
