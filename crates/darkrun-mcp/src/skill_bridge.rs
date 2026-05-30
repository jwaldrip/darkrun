//! Skill → MCP-prompt bridge.
//!
//! Claude Code loads `plugin/skills/*/SKILL.md` as native slash commands. Other
//! harnesses don't — but the ones that consume the MCP `prompts` capability
//! (Cursor, Windsurf, Gemini CLI, Kiro) can surface those same skills as MCP
//! prompts. This module embeds the thin skill redirects and exposes them as
//! prompts named `darkrun:<skill>`, plus an always-present `darkrun:status`
//! resume prompt, so the workflow is reachable on every prompts-capable harness.

use rust_embed::RustEmbed;

/// The shipped skills, embedded so the single binary carries them (debug builds
/// read the filesystem via the `debug-embed` feature).
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../plugin/skills"]
struct Skills;

/// A skill surfaced as an MCP prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPrompt {
    /// The MCP prompt name (`darkrun:<skill>`).
    pub name: String,
    /// One-line description (from the skill's frontmatter).
    pub description: String,
    /// The prompt body the harness injects when the prompt is invoked.
    pub body: String,
}

/// The resume prompt, always offered so a session on a hook-less harness can
/// load the active Run without an auto-injection hook.
fn status_prompt() -> SkillPrompt {
    SkillPrompt {
        name: "darkrun:status".to_string(),
        description: "Resume darkrun — load and advance the active Run".to_string(),
        body: "Call `darkrun_run_next` to load and advance the active darkrun Run, then do \
               exactly what the returned action says. If no Run is active, offer to start one \
               with `darkrun_run_start`."
            .to_string(),
    }
}

/// Every bridged prompt: `darkrun:status` plus one `darkrun:<skill>` per shipped
/// skill, sorted by name for a stable listing.
pub fn skill_prompts() -> Vec<SkillPrompt> {
    let mut out = vec![status_prompt()];
    for path in Skills::iter() {
        // Only the `<skill>/SKILL.md` files; ignore any stray assets.
        if !path.ends_with("/SKILL.md") {
            continue;
        }
        let skill = match path.split('/').next() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let Some(file) = Skills::get(&path) else {
            continue;
        };
        let src = String::from_utf8_lossy(&file.data);
        let (fm_name, description, body) = parse_skill(&src);
        // Prefer the frontmatter `name`, fall back to the directory.
        let name = if fm_name.is_empty() { skill } else { fm_name };
        out.push(SkillPrompt {
            name: format!("darkrun:{name}"),
            description,
            body: body.trim().to_string(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// Look up a single bridged prompt by its full `darkrun:<name>` name.
pub fn skill_prompt(name: &str) -> Option<SkillPrompt> {
    skill_prompts().into_iter().find(|p| p.name == name)
}

/// Parse a `SKILL.md`: returns `(name, description, body)` from the YAML
/// frontmatter + markdown body. Missing fields degrade to empty strings.
fn parse_skill(src: &str) -> (String, String, String) {
    let src = src.trim_start_matches('\u{feff}');
    if let Some(rest) = src.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n').to_string();
            let field = |key: &str| -> String {
                fm.lines()
                    .find_map(|l| {
                        l.trim()
                            .strip_prefix(key)
                            .map(|v| v.trim().trim_matches('"').to_string())
                    })
                    .unwrap_or_default()
            };
            return (field("name:"), field("description:"), body);
        }
    }
    (String::new(), String::new(), src.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_prompt_is_always_present() {
        let prompts = skill_prompts();
        let status = prompts
            .iter()
            .find(|p| p.name == "darkrun:status")
            .expect("status prompt present");
        assert!(status.body.contains("darkrun_run_next"));
    }

    #[test]
    fn bridges_the_shipped_skills() {
        let prompts = skill_prompts();
        // The thin redirects ship ~19 skills; bridge each plus status.
        assert!(prompts.len() > 15, "got {}", prompts.len());
        assert!(prompts.iter().any(|p| p.name == "darkrun:darkrun-pickup"));
        let pickup = skill_prompt("darkrun:darkrun-pickup").expect("pickup");
        assert!(!pickup.description.is_empty());
        assert!(pickup.body.contains("darkrun_run_next"));
    }

    #[test]
    fn names_are_unique_and_sorted() {
        let prompts = skill_prompts();
        let mut names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        names.dedup();
        assert_eq!(names.len(), prompts.len(), "duplicate prompt names");
    }

    #[test]
    fn parse_skill_pulls_name_description_body() {
        let (n, d, b) = parse_skill("---\nname: x\ndescription: A thing\n---\n\nBody here.\n");
        assert_eq!(n, "x");
        assert_eq!(d, "A thing");
        assert_eq!(b.trim(), "Body here.");
    }
}
