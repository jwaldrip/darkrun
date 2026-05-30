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
}

impl Harness {
    /// Every harness, in registry order.
    pub const ALL: [Harness; 6] = [
        Harness::ClaudeCode,
        Harness::Cursor,
        Harness::Windsurf,
        Harness::GeminiCli,
        Harness::Opencode,
        Harness::Kiro,
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
        }
    }

    /// Parse a harness key, tolerating case, spaces, and `_`/`-` spelling
    /// (`Gemini CLI`, `gemini_cli`, `GEMINI-CLI` all resolve). `None` for an
    /// unknown name — callers fall back to [`Harness::ClaudeCode`].
    pub fn parse(raw: &str) -> Option<Harness> {
        let norm = raw.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        Harness::ALL.into_iter().find(|h| h.key() == norm).or_else(|| {
            // A few common aliases.
            match norm.as_str() {
                "claude" | "claudecode" | "cc" => Some(Harness::ClaudeCode),
                "gemini" | "geminicli" => Some(Harness::GeminiCli),
                "open-code" => Some(Harness::Opencode),
                _ => None,
            }
        })
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
                model_tiers: &["haiku", "sonnet", "opus"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["Agent", "Task"],
                    parallel_spawn: true,
                    background_spawn: true,
                    isolation: true,
                    model_param: false,
                },
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
                model_tiers: &["haiku", "sonnet", "opus"],
                subagents: Subagents {
                    supported: true,
                    tool_names: &["/spawn"],
                    parallel_spawn: true,
                    background_spawn: false,
                    isolation: false,
                    model_param: false,
                },
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
}

impl Capabilities {
    /// Translate a canonical model tier (`haiku`/`sonnet`/`opus`, as the factory
    /// corpus names them) into this harness's provider vocabulary. Unknown tiers
    /// pass through unchanged.
    pub fn map_model(&self, tier: &str) -> &'static str {
        let t = tier.trim().to_ascii_lowercase();
        match self.model_provider {
            // Anthropic harnesses use the canonical tier names directly.
            "anthropic" => match t.as_str() {
                "haiku" => "haiku",
                "sonnet" => "sonnet",
                "opus" => "opus",
                _ => "sonnet",
            },
            // Google collapses to flash (cheap) / pro (everything else).
            "google" => match t.as_str() {
                "haiku" => "flash",
                "sonnet" | "opus" => "pro",
                _ => "pro",
            },
            // Multi-provider harnesses use abstract fast/balanced/powerful tiers.
            _ => match t.as_str() {
                "haiku" => "fast",
                "sonnet" => "balanced",
                "opus" => "powerful",
                _ => "balanced",
            },
        }
    }

    /// The primary subagent spawn-tool name for this harness (the first listed),
    /// or `"Agent"` as a neutral default when the harness has none.
    pub fn subagent_tool(&self) -> &'static str {
        self.subagents.tool_names.first().copied().unwrap_or("Agent")
    }
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
        assert_eq!(Harness::parse("nonsense"), None);
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
}
