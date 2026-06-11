//! Agent-harness capability registry.
//!
//! darkrun runs as an MCP server inside several agent harnesses — Claude Code,
//! Cursor, Windsurf, Gemini CLI, OpenCode, and Kiro — and each exposes a
//! different feature surface: a tool-count budget, parallel subagents or not,
//! interactive elicitation or not, a hook system or not, native slash commands
//! or MCP prompts, and a different model-tier vocabulary.
//!
//! This crate is the single source of truth for those differences. The active
//! harness is [`detect`]ed once at MCP boot (from the `--harness` flag or the
//! `DARKRUN_HARNESS` env), and everything downstream branches on
//! [`Capabilities`] — never on the harness name directly. The capability set is
//! [`serde::Serialize`], so it drops straight into the prompt context for the
//! template guards that adapt the engine's rendered instructions per harness.

use serde::Serialize;

/// One supported agent harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    /// Anthropic Claude Code — the reference harness (every capability on).
    ClaudeCode,
    /// Cursor — tool-budget capped, no hooks, elicitation, parallel subagents.
    Cursor,
    /// Windsurf — no subagents, no elicitation, larger tool budget.
    Windsurf,
    /// Gemini CLI — native `.toml` slash commands, Google model tiers.
    GeminiCli,
    /// OpenCode — sequential subagents, no MCP prompts, no elicitation.
    Opencode,
    /// Kiro — hooks + elicitation + parallel subagents (Claude-Code-like).
    Kiro,
    /// OpenAI Codex CLI — single-agent, no hooks/browser; conservative profile
    /// (elicitation + MCP-prompt slash commands left off pending confirmation).
    Codex,
}

impl Harness {
    /// Every harness, in registry order.
    pub const ALL: [Harness; 7] = [
        Harness::ClaudeCode,
        Harness::Cursor,
        Harness::Windsurf,
        Harness::GeminiCli,
        Harness::Opencode,
        Harness::Kiro,
        Harness::Codex,
    ];

    /// The canonical kebab-case key (`claude-code`, `gemini-cli`, …) — the value
    /// passed to `--harness` / `DARKRUN_HARNESS` and emitted on the wire.
    pub fn key(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Cursor => "cursor",
            Harness::Windsurf => "windsurf",
            Harness::GeminiCli => "gemini-cli",
            Harness::Opencode => "opencode",
            Harness::Kiro => "kiro",
            Harness::Codex => "codex",
        }
    }

    /// Parse a harness key, tolerating case, spaces, and `_`/`-` spelling
    /// (`Gemini CLI`, `gemini_cli`, `GEMINI-CLI` all resolve). `None` for an
    /// unknown name — callers fall back to [`Harness::ClaudeCode`].
    pub fn parse(raw: &str) -> Option<Harness> {
        let norm = raw.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        if let Some(h) = Harness::ALL.into_iter().find(|h| h.key() == norm) {
            return Some(h);
        }
        // A few common aliases.
        match norm.as_str() {
            "claude" | "claudecode" | "cc" => Some(Harness::ClaudeCode),
            "gemini" | "geminicli" => Some(Harness::GeminiCli),
            "open-code" => Some(Harness::Opencode),
            "codex-cli" | "codexcli" | "openai-codex" => Some(Harness::Codex),
            _ => None,
        }
    }

    /// True for the reference harness (every capability on).
    pub fn is_claude_code(self) -> bool {
        matches!(self, Harness::ClaudeCode)
    }

    /// This harness's full capability set.
    pub fn capabilities(self) -> Capabilities {
        match self {
            Harness::ClaudeCode => Capabilities {
                harness: self,
                display_name: "Claude Code",
                native_skills: true,
                prompts_as_slash_commands: true,
                hooks: true,
                elicitation: true,
                native_ask_user: true,
                plan_mode: true,
                max_tools: None,
                mcp_prompts: true,
                mcp_resources: true,
                browser_ui: true,
                model_provider: "anthropic",
                model_tiers: &["haiku", "sonnet", "opus", "fable"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["Agent", "Task"],
                    parallel_spawn: true,
                    background_spawn: true,
                    isolation: true,
                    model_param: false,
                },
                autonomous_launch_args: Some("--dangerously-skip-permissions"),
                // NOTE: per the product spec; re-verify Claude Code exposes this flag.
                worktree_flag: Some("--worktree"),
                worktree_dir: Some(".claude/worktrees"),
            },
            Harness::Cursor => Capabilities {
                harness: self,
                display_name: "Cursor",
                native_skills: false,
                prompts_as_slash_commands: false,
                hooks: false,
                elicitation: true,
                native_ask_user: false,
                plan_mode: false,
                max_tools: Some(40),
                mcp_prompts: true,
                mcp_resources: true,
                browser_ui: false,
                model_provider: "multi",
                model_tiers: &["fast", "balanced", "powerful"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["Agent"],
                    parallel_spawn: true,
                    background_spawn: false,
                    isolation: false,
                    model_param: false,
                },
                // GUI-driven — autonomous mode lives in Cursor's settings, no CLI flag.
                autonomous_launch_args: None,
                worktree_flag: None,
                worktree_dir: None,
            },
            Harness::Windsurf => Capabilities {
                harness: self,
                display_name: "Windsurf",
                native_skills: false,
                prompts_as_slash_commands: false,
                hooks: false,
                elicitation: false,
                native_ask_user: false,
                plan_mode: false,
                max_tools: Some(100),
                mcp_prompts: true,
                mcp_resources: true,
                browser_ui: false,
                model_provider: "multi",
                model_tiers: &["fast", "balanced", "powerful"],
                subagents: Subagents {
                    supported: false,
                    tool_names: &[],
                    parallel_spawn: false,
                    background_spawn: false,
                    isolation: false,
                    model_param: false,
                },
                // GUI-driven — autonomous mode lives in Windsurf's settings, no CLI flag.
                autonomous_launch_args: None,
                worktree_flag: None,
                worktree_dir: None,
            },
            Harness::GeminiCli => Capabilities {
                harness: self,
                display_name: "Gemini CLI",
                native_skills: false,
                prompts_as_slash_commands: true,
                hooks: false,
                elicitation: false,
                native_ask_user: false,
                plan_mode: false,
                max_tools: None,
                mcp_prompts: true,
                mcp_resources: true,
                browser_ui: false,
                model_provider: "google",
                model_tiers: &["flash", "pro"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["@subagent"],
                    parallel_spawn: true,
                    background_spawn: false,
                    isolation: false,
                    model_param: false,
                },
                autonomous_launch_args: Some("--yolo"),
                worktree_flag: None,
                worktree_dir: None,
            },
            Harness::Opencode => Capabilities {
                harness: self,
                display_name: "OpenCode",
                native_skills: false,
                prompts_as_slash_commands: false,
                hooks: false,
                elicitation: false,
                native_ask_user: false,
                plan_mode: false,
                max_tools: None,
                mcp_prompts: false,
                mcp_resources: false,
                browser_ui: false,
                model_provider: "multi",
                model_tiers: &["fast", "balanced", "powerful"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["subagent"],
                    parallel_spawn: false,
                    background_spawn: false,
                    isolation: false,
                    model_param: true,
                },
                // No known autonomous-mode CLI flag — update to Some(...) if one lands.
                autonomous_launch_args: None,
                worktree_flag: None,
                worktree_dir: None,
            },
            Harness::Kiro => Capabilities {
                harness: self,
                display_name: "Kiro",
                native_skills: false,
                prompts_as_slash_commands: true,
                hooks: true,
                elicitation: true,
                native_ask_user: false,
                plan_mode: false,
                max_tools: None,
                mcp_prompts: true,
                mcp_resources: true,
                browser_ui: false,
                model_provider: "anthropic",
                model_tiers: &["haiku", "sonnet", "opus", "fable"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["/spawn"],
                    parallel_spawn: true,
                    background_spawn: false,
                    isolation: false,
                    model_param: false,
                },
                // GUI-driven — autonomous mode lives in Kiro's settings, no CLI flag.
                autonomous_launch_args: None,
                worktree_flag: None,
                worktree_dir: None,
            },
            // Conservative profile. Codex reads the MCP `instructions` field and
            // serves STDIO/HTTP MCP servers, but its docs don't confirm MCP
            // elicitation or MCP-prompts-as-slash-commands — both left off so we
            // never instruct the agent to use machinery that may be absent
            // (degrades to inline text rather than breaking). It is single-agent
            // (no parallel subagent spawn) and has no Claude-Code-style hooks. It
            // does have a native plan mode (`/plan-mode`).
            Harness::Codex => Capabilities {
                harness: self,
                display_name: "Codex",
                native_skills: false,
                prompts_as_slash_commands: false,
                hooks: false,
                elicitation: false,
                native_ask_user: false,
                plan_mode: true,
                max_tools: None,
                mcp_prompts: false,
                mcp_resources: false,
                browser_ui: false,
                model_provider: "openai",
                model_tiers: &["low", "medium", "high"],
                subagents: Subagents {
                    supported: false,
                    tool_names: &[],
                    parallel_spawn: false,
                    background_spawn: false,
                    isolation: false,
                    model_param: false,
                },
                autonomous_launch_args: Some("--full-auto"),
                worktree_flag: None,
                worktree_dir: None,
            },
        }
    }
}

/// Subagent-spawning capabilities for a harness.
#[derive(Debug, Clone, Serialize)]
pub struct Subagents {
    /// Whether the harness can spawn subagents at all. When `false`, Units run
    /// sequentially in the main agent.
    pub supported: bool,
    /// The tool name(s) the harness exposes for spawning (`Agent`/`Task`,
    /// `@subagent`, `subagent`, `/spawn`). Empty when unsupported.
    pub tool_names: &'static [&'static str],
    /// Whether multiple subagents can be spawned in parallel (one wave at once).
    pub parallel_spawn: bool,
    /// Whether subagents can be spawned in the background (detached).
    pub background_spawn: bool,
    /// Whether subagents run with isolation from the main context.
    pub isolation: bool,
    /// Whether the spawn call takes an explicit model parameter.
    pub model_param: bool,
}

/// The full capability surface of a harness — the value everything downstream
/// branches on. Serializable so it drops into the prompt context for the
/// `{% if capabilities.* %}` guards that adapt rendered instructions.
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    /// Which harness these belong to.
    pub harness: Harness,
    /// Human-readable name (`Claude Code`, `Gemini CLI`).
    pub display_name: &'static str,
    /// Whether the harness loads `SKILL.md` skills natively as slash commands.
    pub native_skills: bool,
    /// Whether MCP prompts / commands surface as native `/darkrun:x` slash
    /// commands (vs. an MCP prompt picker, vs. neither).
    pub prompts_as_slash_commands: bool,
    /// Whether the harness runs a PreToolUse/PostToolUse hook system.
    pub hooks: bool,
    /// Whether the harness supports interactive MCP elicitation (for review
    /// gates and decisions).
    pub elicitation: bool,
    /// Whether the harness exposes a native structured "ask the user" tool
    /// (Claude Code's `AskUserQuestion`).
    pub native_ask_user: bool,
    /// Whether the harness has a plan mode.
    pub plan_mode: bool,
    /// The maximum number of tools the harness will accept, if capped (Cursor
    /// caps at 40). `None` means uncapped.
    pub max_tools: Option<usize>,
    /// Whether the harness consumes the MCP `prompts` capability.
    pub mcp_prompts: bool,
    /// Whether the harness consumes the MCP `resources` capability.
    pub mcp_resources: bool,
    /// Whether the harness can render the browser/visual review tools
    /// (screenshots, visual question/direction). Only Claude Code today.
    pub browser_ui: bool,
    /// The model provider (`anthropic`, `google`, `multi`).
    pub model_provider: &'static str,
    /// The harness's own model-tier names, cheapest→deepest.
    pub model_tiers: &'static [&'static str],
    /// Subagent-spawning capabilities.
    pub subagents: Subagents,
    /// The CLI flag that puts this harness into autonomous (no-approval) mode,
    /// if it exposes one (`--dangerously-skip-permissions` for Claude Code,
    /// `--full-auto` for Codex, `--yolo` for Gemini CLI). `None` for harnesses
    /// that have no such flag — either GUI-driven ones whose autonomous mode
    /// lives in in-app settings (Cursor, Windsurf, Kiro), or CLIs with no known
    /// autonomous-mode flag (OpenCode). These flag names are canonical for the
    /// current harness versions and should be re-verified against release notes.
    pub autonomous_launch_args: Option<&'static str>,
    /// The flag a harness takes to open/create a named git worktree on launch
    /// (e.g. Claude Code's `--worktree <name>`). `None` when the harness has no
    /// such flag; the start command then just `cd`s into the project dir. When
    /// set, the command appends `<flag> darkrun-<run>` so a Run gets its own tree.
    pub worktree_flag: Option<&'static str>,
    /// The directory (repo-relative) under which the harness keeps its managed
    /// worktrees (`.claude/worktrees` for Claude Code). A project path under
    /// this dir is one of the harness's own worktrees — launchers can hand the
    /// `cd` back to the harness via `worktree_flag`. `None` = no convention.
    pub worktree_dir: Option<&'static str>,
}

impl Capabilities {
    /// Translate a canonical model tier (`haiku`/`sonnet`/`opus`/`fable`, as
    /// the factory corpus names them) into this harness's provider vocabulary.
    /// `fable` is the Mythos-family frontier tier — the deepest reasoning the
    /// provider offers; harnesses without a distinct frontier collapse it onto
    /// their top tier. Unknown tiers fall back to the balanced middle.
    pub fn map_model(&self, tier: &str) -> &'static str {
        let t = tier.trim().to_ascii_lowercase();
        match self.model_provider {
            // Anthropic harnesses use the canonical tier names directly.
            "anthropic" => match t.as_str() {
                "haiku" => "haiku",
                "sonnet" => "sonnet",
                "opus" => "opus",
                "fable" | "mythos" => "fable",
                _ => "sonnet",
            },
            // Google collapses to flash (cheap) / pro (everything else).
            "google" => match t.as_str() {
                "haiku" => "flash",
                "sonnet" | "opus" | "fable" | "mythos" => "pro",
                _ => "pro",
            },
            // OpenAI (Codex) maps tiers to reasoning-effort levels — the model
            // is fixed, the depth is what varies.
            "openai" => match t.as_str() {
                "haiku" => "low",
                "sonnet" => "medium",
                "opus" | "fable" | "mythos" => "high",
                _ => "medium",
            },
            // Multi-provider harnesses use abstract fast/balanced/powerful tiers.
            _ => match t.as_str() {
                "haiku" => "fast",
                "sonnet" => "balanced",
                "opus" | "fable" | "mythos" => "powerful",
                _ => "balanced",
            },
        }
    }

    /// The primary subagent spawn-tool name for this harness (the first listed),
    /// or `"Agent"` as a neutral default when the harness has none.
    pub fn subagent_tool(&self) -> &'static str {
        self.subagents.tool_names.first().copied().unwrap_or("Agent")
    }

    /// The CLI flag(s) to launch this harness in autonomous (no-approval) mode,
    /// or `""` when the harness has none (GUI-driven settings, or no known flag).
    /// Callers can splice this straight into a launch command line — an empty
    /// string contributes nothing.
    pub fn autonomous_launch_args(&self) -> &'static str {
        self.autonomous_launch_args.unwrap_or("")
    }
}

/// Adapt an engine-rendered instruction to the active harness.
///
/// The canonical prompt corpus is written for the maximal reference harness
/// (Claude Code): parallel subagents, a browser review UI, a hook system. Rather
/// than mutate that body with fragile rewrites, this *appends* a "Harness note"
/// spelling out the execution-model differences the agent must honour on this
/// harness — no subagents → run sequentially, no browser UI → ask inline, no
/// hooks → drive the loop and track outputs yourself. For Claude Code it is a
/// no-op (the corpus already matches).
pub fn adapt_instructions(text: &str, caps: &Capabilities) -> String {
    if caps.harness.is_claude_code() {
        return text.to_string();
    }

    let mut notes: Vec<String> = Vec::new();

    // Subagent execution model.
    if !caps.subagents.supported {
        notes.push(
            "No subagents on this harness — do each Unit's work yourself, sequentially, \
             in the main thread. Ignore any 'dispatch in parallel' phrasing above."
                .to_string(),
        );
    } else if !caps.subagents.parallel_spawn {
        notes.push(format!(
            "Subagents run one at a time here (via `{}`) — dispatch the wave sequentially, \
             not in parallel.",
            caps.subagent_tool()
        ));
    } else if caps.subagent_tool() != "Agent" {
        notes.push(format!(
            "Spawn subagents with this harness's `{}` mechanism.",
            caps.subagent_tool()
        ));
    }

    // Interactive decisions / review surface.
    if !caps.browser_ui {
        if caps.elicitation {
            notes.push(
                "No browser review UI — surface design directions and decisions through MCP \
                 elicitation, not the desktop app."
                    .to_string(),
            );
        } else {
            notes.push(
                "No interactive review UI — when a step asks for a design direction or a user \
                 decision, ask inline in plain text and act on the answer."
                    .to_string(),
            );
        }
    }

    // Hook-driven automation that this harness lacks.
    if !caps.hooks {
        notes.push(
            "No hooks on this harness — call `darkrun_advance` yourself at the start of each \
             session and after each step, and register Unit outputs explicitly. Nothing is \
             auto-injected or auto-tracked."
                .to_string(),
        );
    }

    if notes.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len() + 256);
    out.push_str(text.trim_end());
    out.push_str("\n\n---\n**Harness note (");
    out.push_str(caps.display_name);
    out.push_str("):**\n");
    for n in &notes {
        out.push_str("- ");
        out.push_str(n);
        out.push('\n');
    }
    out
}

/// The env var that selects the harness when no `--harness` flag is passed.
pub const ENV_VAR: &str = "DARKRUN_HARNESS";

/// Resolve the active harness: an explicit `--harness` value wins, else the
/// `DARKRUN_HARNESS` env, else [`Harness::ClaudeCode`]. Unknown names fall back
/// to Claude Code rather than erroring, so a typo degrades gracefully.
pub fn detect(flag: Option<&str>) -> Harness {
    flag.and_then(Harness::parse)
        .or_else(|| std::env::var(ENV_VAR).ok().and_then(|v| Harness::parse(&v)))
        .unwrap_or(Harness::ClaudeCode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_harness_round_trips_through_its_key() {
        for h in Harness::ALL {
            assert_eq!(Harness::parse(h.key()), Some(h), "{}", h.key());
        }
    }

    #[test]
    fn parse_tolerates_spelling() {
        assert_eq!(Harness::parse("Gemini CLI"), Some(Harness::GeminiCli));
        assert_eq!(Harness::parse("gemini_cli"), Some(Harness::GeminiCli));
        assert_eq!(Harness::parse("CLAUDE-CODE"), Some(Harness::ClaudeCode));
        assert_eq!(Harness::parse("claude"), Some(Harness::ClaudeCode));
        assert_eq!(Harness::parse("codex"), Some(Harness::Codex));
        assert_eq!(Harness::parse("codex-cli"), Some(Harness::Codex));
        assert_eq!(Harness::parse("nonsense"), None);
    }

    #[test]
    fn codex_is_conservative_single_agent() {
        let c = Harness::Codex.capabilities();
        assert!(!c.hooks && !c.browser_ui && !c.elicitation && !c.mcp_prompts);
        assert!(!c.subagents.supported);
        assert!(c.plan_mode);
        assert_eq!(c.model_provider, "openai");
        assert_eq!(c.map_model("haiku"), "low");
        assert_eq!(c.map_model("opus"), "high");
    }

    #[test]
    fn detect_prefers_flag_then_env_then_default() {
        // Flag wins.
        assert_eq!(detect(Some("cursor")), Harness::Cursor);
        // Unknown flag with no env → default.
        assert_eq!(detect(Some("nope")), Harness::ClaudeCode);
        // No flag, no env → default (env not set in this test process).
        assert_eq!(detect(None), Harness::ClaudeCode);
    }

    #[test]
    fn claude_code_is_the_maximal_reference() {
        let c = Harness::ClaudeCode.capabilities();
        assert!(c.hooks && c.elicitation && c.native_ask_user && c.browser_ui);
        assert!(c.subagents.supported && c.subagents.parallel_spawn);
        assert_eq!(c.max_tools, None);
    }

    #[test]
    fn cursor_is_tool_budget_capped() {
        assert_eq!(Harness::Cursor.capabilities().max_tools, Some(40));
    }

    #[test]
    fn windsurf_has_no_subagents() {
        assert!(!Harness::Windsurf.capabilities().subagents.supported);
    }

    #[test]
    fn gemini_remaps_model_tiers_to_google() {
        let g = Harness::GeminiCli.capabilities();
        assert_eq!(g.map_model("haiku"), "flash");
        assert_eq!(g.map_model("sonnet"), "pro");
        assert_eq!(g.map_model("opus"), "pro");
    }

    #[test]
    fn anthropic_tiers_are_identity() {
        let k = Harness::Kiro.capabilities();
        assert_eq!(k.map_model("sonnet"), "sonnet");
        assert_eq!(k.map_model("opus"), "opus");
    }

    #[test]
    fn multi_provider_uses_abstract_tiers() {
        let c = Harness::Cursor.capabilities();
        assert_eq!(c.map_model("haiku"), "fast");
        assert_eq!(c.map_model("sonnet"), "balanced");
        assert_eq!(c.map_model("opus"), "powerful");
    }

    #[test]
    fn subagent_tool_picks_primary_or_neutral_default() {
        assert_eq!(Harness::GeminiCli.capabilities().subagent_tool(), "@subagent");
        assert_eq!(Harness::Windsurf.capabilities().subagent_tool(), "Agent");
    }

    #[test]
    fn only_claude_renders_browser_ui() {
        for h in Harness::ALL {
            assert_eq!(h.capabilities().browser_ui, h.is_claude_code(), "{}", h.key());
        }
    }

    #[test]
    fn hooks_only_on_claude_and_kiro() {
        for h in Harness::ALL {
            let expect = matches!(h, Harness::ClaudeCode | Harness::Kiro);
            assert_eq!(h.capabilities().hooks, expect, "{}", h.key());
        }
    }

    #[test]
    fn adapt_is_noop_for_claude_code() {
        let caps = Harness::ClaudeCode.capabilities();
        let text = "Dispatch the make beat in parallel across these Units.";
        assert_eq!(adapt_instructions(text, &caps), text);
    }

    #[test]
    fn adapt_notes_sequential_for_no_subagent_harness() {
        let caps = Harness::Windsurf.capabilities();
        let out = adapt_instructions("Do the work.", &caps);
        assert!(out.contains("Harness note (Windsurf)"));
        assert!(out.contains("No subagents"));
        // Windsurf also lacks a browser UI and hooks.
        assert!(out.contains("No interactive review UI"));
        assert!(out.contains("No hooks"));
        // The original body is preserved.
        assert!(out.contains("Do the work."));
    }

    #[test]
    fn adapt_uses_elicitation_phrasing_when_supported() {
        // Cursor has elicitation but no browser UI.
        let caps = Harness::Cursor.capabilities();
        let out = adapt_instructions("Do the work.", &caps);
        assert!(out.contains("MCP elicitation"));
        assert!(!out.contains("ask inline in plain text"));
    }

    #[test]
    fn adapt_names_the_harness_subagent_tool() {
        // Gemini supports parallel subagents but via @subagent, not Agent.
        let caps = Harness::GeminiCli.capabilities();
        let out = adapt_instructions("Do the work.", &caps);
        assert!(out.contains("@subagent"));
    }

    #[test]
    fn adapt_codex_flags_manual_loop_and_inline_decisions() {
        let caps = Harness::Codex.capabilities();
        let out = adapt_instructions("Do the work.", &caps);
        assert!(out.contains("No subagents"));
        assert!(out.contains("ask inline in plain text"));
        assert!(out.contains("call `darkrun_advance` yourself"));
    }

    #[test]
    fn test_autonomous_launch_args_for_cli_harnesses() {
        assert_eq!(
            Harness::ClaudeCode.capabilities().autonomous_launch_args(),
            "--dangerously-skip-permissions"
        );
        assert_eq!(Harness::GeminiCli.capabilities().autonomous_launch_args(), "--yolo");
        assert_eq!(Harness::Codex.capabilities().autonomous_launch_args(), "--full-auto");
    }

    #[test]
    fn test_autonomous_launch_args_empty_for_gui_harnesses() {
        // GUI harnesses drive autonomous mode through in-app settings, not a CLI flag.
        for h in [Harness::Cursor, Harness::Windsurf, Harness::Kiro] {
            assert_eq!(h.capabilities().autonomous_launch_args(), "", "{}", h.key());
            assert_eq!(h.capabilities().autonomous_launch_args, None, "{}", h.key());
        }
    }

    #[test]
    fn test_autonomous_launch_args_empty_for_unsupported() {
        // OpenCode has no known autonomous-mode CLI flag.
        let c = Harness::Opencode.capabilities();
        assert_eq!(c.autonomous_launch_args(), "");
        assert_eq!(c.autonomous_launch_args, None);
    }

    #[test]
    fn test_all_harnesses_have_autonomous_launch_args() {
        // Every harness defines the field; CLI harnesses carry a non-empty flag,
        // and the helper never returns the literal `None` wrapper.
        let cli = [Harness::ClaudeCode, Harness::GeminiCli, Harness::Codex];
        for h in Harness::ALL {
            let caps = h.capabilities();
            let helper = caps.autonomous_launch_args();
            match caps.autonomous_launch_args {
                Some(flag) => assert_eq!(helper, flag, "{}", h.key()),
                None => assert_eq!(helper, "", "{}", h.key()),
            }
            assert_eq!(cli.contains(&h), !helper.is_empty(), "{}", h.key());
        }
    }

    #[test]
    fn fable_tier_maps_to_each_providers_frontier() {
        assert_eq!(Harness::ClaudeCode.capabilities().map_model("fable"), "fable");
        assert_eq!(Harness::ClaudeCode.capabilities().map_model("mythos"), "fable");
        assert_eq!(Harness::GeminiCli.capabilities().map_model("fable"), "pro");
        assert_eq!(Harness::Codex.capabilities().map_model("fable"), "high");
        assert_eq!(Harness::Opencode.capabilities().map_model("fable"), "powerful");
        // The tier list advertises it where the provider distinguishes it.
        assert!(Harness::ClaudeCode.capabilities().model_tiers.contains(&"fable"));
    }

}

#[cfg(test)]
mod model_and_adapt_tests {
    use super::*;

    #[test]
    fn map_model_covers_every_provider_and_tier() {
        use Harness::*;
        let cases = [
            (ClaudeCode, "haiku", "haiku"), (ClaudeCode, "sonnet", "sonnet"),
            (ClaudeCode, "opus", "opus"), (ClaudeCode, "other", "sonnet"),
            (GeminiCli, "haiku", "flash"), (GeminiCli, "sonnet", "pro"),
            (GeminiCli, "opus", "pro"), (GeminiCli, "other", "pro"),
            (Codex, "haiku", "low"), (Codex, "sonnet", "medium"),
            (Codex, "opus", "high"), (Codex, "other", "medium"),
            (Cursor, "haiku", "fast"), (Cursor, "sonnet", "balanced"),
            (Cursor, "opus", "powerful"), (Cursor, "other", "balanced"),
        ];
        for (h, tier, want) in cases {
            assert_eq!(h.capabilities().map_model(tier), want, "{h:?} {tier}");
        }
    }

    #[test]
    fn adapt_instructions_runs_for_every_harness() {
        let text = "Dispatch the reviewers. Use subagents to fan the wave out in parallel.";
        for h in [
            Harness::ClaudeCode, Harness::Cursor, Harness::Windsurf, Harness::GeminiCli,
            Harness::Opencode, Harness::Kiro, Harness::Codex,
        ] {
            let _ = adapt_instructions(text, &h.capabilities());
        }
    }
}
