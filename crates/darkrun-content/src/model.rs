//! Loaded content model — the parsed shape of the embedded factory corpus.
//!
//! These types mirror the markdown+frontmatter definitions under
//! `plugin/factories/<name>/`. A [`Factory`] is the top-level methodology; it
//! owns an ordered list of [`Station`]s; each station references named
//! [`Explorer`]s, [`Worker`]s, and [`Reviewer`]s by their file definitions.
//!
//! Frontmatter is parsed with `serde`; bodies are kept as raw markdown so the
//! manager can hand a role's instructions to an agent verbatim.

use serde::{Deserialize, Serialize};

/// Frontmatter of a `FACTORY.md` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryFrontmatter {
    /// Factory slug (e.g. `software`).
    pub name: String,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Category label (e.g. `engineering`).
    #[serde(default)]
    pub category: String,
    /// Default model assigned to roles that do not override it.
    #[serde(default)]
    pub default_model: String,
    /// Optional single parent factory this one specializes. The parent becomes
    /// **walkable in the resolution path**: any station/role this factory does
    /// not define falls through to the parent (and transitively up the chain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
    /// Ordered station slugs, in cost-of-late-discovery order. Vestigial — the
    /// engine walks the fixed `Position::FLOW`; kept for tolerant parsing.
    #[serde(default)]
    pub stations: Vec<String>,
    /// fix-worker slugs that handle drift/feedback repairs.
    #[serde(default)]
    pub fix_workers: Vec<String>,
    /// Run-level reviewer slugs — whole-Run auditors that run AFTER the final
    /// station, judging the Run end-to-end across every station's locked
    /// artifact rather than any single station's output.
    #[serde(default)]
    pub reviewers: Vec<String>,
    /// Reflection-dimension slugs evaluated at Run completion — a backward look
    /// over the finished Run that produces learnings, not a gate.
    #[serde(default)]
    pub reflections: Vec<String>,
    /// The delivery surfaces this factory can produce, declared as data (e.g.
    /// `web_ui`, `library`, `cli`). The Shape station classifies the run into
    /// **one** of these, which routes how Prove/Audit verify it. A factory that
    /// declares none offers no surface classification. Per-factory data, not a
    /// fixed enum — `software` offers the full set, including `library`/`api`.
    #[serde(default)]
    pub surfaces: Vec<String>,
}

/// Frontmatter of a `STATION.md` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationFrontmatter {
    /// Station slug (e.g. `frame`) — must be one of the six FSSBPH positions.
    pub name: String,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// The risk class this station eliminates, in the domain's words
    /// (`wrong-thing`, `implementation-defects`, …). A prompt variable
    /// (`{{ kills }}`), not engine logic.
    #[serde(default)]
    pub kills: String,
    /// Optional domain-facing display name shown over the fixed position (legal
    /// → `Intake`). Display-only; defaults to the position name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Explorer slugs this station runs in its Explore phase.
    #[serde(default)]
    pub explorers: Vec<String>,
    /// Worker slugs, in Make -> Challenge -> Resolve sequence.
    #[serde(default)]
    pub workers: Vec<String>,
    /// Fix-worker slugs for THIS station's Track-B repairs. Overrides the
    /// factory-level `fix_workers` when non-empty; inherited from the factory
    /// otherwise — so a station can specialize who repairs its feedback.
    #[serde(default)]
    pub fix_workers: Vec<String>,
    /// Reviewer slugs that verify output in the Review phase.
    #[serde(default)]
    pub reviewers: Vec<String>,
    /// The durable artifact this station locks (e.g. `frame.md` or `code`).
    #[serde(default)]
    pub locked_artifact: String,
    /// Artifacts (from upstream stations) this station consumes.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Upstream locked artifacts this station **deliberately does not** carry
    /// forward. Cross-station coverage requires every prior station's artifact
    /// to be either an `input` or here — so the run's distillation is never
    /// *silently* dropped, only consciously waived.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs_waived: Vec<String>,
}

/// The kind of role a definition file describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    /// Gathers context in the Explore phase.
    Explorer,
    /// Performs a beat of a Pass (Make/Challenge/Resolve).
    Worker,
    /// Verifies output independently in the Review phase.
    Reviewer,
    /// Looks back over a completed Run to produce learnings on one dimension.
    /// Unlike a Reviewer it gates nothing — it reflects, it does not block.
    Reflection,
}

/// Frontmatter of a role definition (`explorers|workers|reviewers|reflections/*.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleFrontmatter {
    /// Role slug (matches the station's reference list entry).
    pub name: String,
    /// DEPRECATED — the role kind is inferred from the role's *directory*
    /// (`explorers/` → Explorer, `workers/` → Worker, …). Optional and ignored
    /// if present; never written in new content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<RoleKind>,
    /// Optional model override; falls back to the factory default.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional review posture for a reviewer — `lens` (constructive, one
    /// perspective) or `strict` (adversarial, find every flaw). Injected into
    /// the reviewer's dispatch framing; absent → the default neutral posture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<String>,
    /// Optional pass-loop role for a worker — `plan` (designs, doesn't fix),
    /// `build` (produces and repairs), or `verify` (judges, doesn't fix). A
    /// reject bounces to the nearest preceding `build` worker; `verify`/`plan`
    /// beats are skipped on the way back. Absent → treated as a `build` worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Optional surface scope for a reviewer — the delivery surfaces this
    /// reviewer applies to (e.g. `[web_ui, desktop, mobile]` for an a11y or
    /// visual-regression reviewer). When set, the reviewer fires only on a run
    /// classified into one of these surfaces; empty → it fires on every run.
    /// Lets a factory declare a surface-specific reviewer without omitting it
    /// per-station by hand (E6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applies_to: Vec<String>,
}

/// A fully-loaded role: its frontmatter, its raw markdown instructions, and the
/// kind inferred from the directory it was loaded from.
#[derive(Debug, Clone)]
pub struct Role {
    /// Parsed frontmatter.
    pub frontmatter: RoleFrontmatter,
    /// Raw markdown body — the role's instructions, handed to an agent verbatim.
    pub body: String,
    /// The role kind, inferred from the typed directory (`explorers/` etc.).
    pub kind: RoleKind,
}

impl Role {
    /// The role's slug.
    pub fn name(&self) -> &str {
        &self.frontmatter.name
    }

    /// The role kind — inferred from its directory, not its frontmatter.
    pub fn kind(&self) -> RoleKind {
        self.kind
    }
}

impl RoleKind {
    /// The role kind for a typed role directory (`explorers` → `Explorer`, …).
    pub fn from_dir(subdir: &str) -> Option<RoleKind> {
        match subdir {
            "explorers" => Some(RoleKind::Explorer),
            "workers" => Some(RoleKind::Worker),
            "reviewers" => Some(RoleKind::Reviewer),
            "reflections" => Some(RoleKind::Reflection),
            _ => None,
        }
    }
}

/// A fully-loaded station: its frontmatter, body, and resolved roles.
#[derive(Debug, Clone)]
pub struct Station {
    /// Parsed frontmatter.
    pub frontmatter: StationFrontmatter,
    /// Raw markdown body explaining the station's purpose and risk class.
    pub body: String,
    /// Explorers, in declaration order.
    pub explorers: Vec<Role>,
    /// Workers, in Make -> Challenge -> Resolve order.
    pub workers: Vec<Role>,
    /// Reviewers, in declaration order.
    pub reviewers: Vec<Role>,
}

impl Station {
    /// The station's slug — the fixed FSSBPH position, used for routing/URLs.
    pub fn name(&self) -> &str {
        &self.frontmatter.name
    }

    /// The station's domain-facing display name (e.g. legal → `Intake`),
    /// falling back to the position slug when no `label` is declared.
    /// Display-only; never use this for routing or lookup.
    pub fn label(&self) -> &str {
        self.frontmatter.label.as_deref().unwrap_or(self.name())
    }
}

/// A fully-loaded factory: its frontmatter, body, and ordered stations.
#[derive(Debug, Clone)]
pub struct Factory {
    /// Parsed frontmatter.
    pub frontmatter: FactoryFrontmatter,
    /// Raw markdown body — the factory overview.
    pub body: String,
    /// Stations in `frontmatter.stations` order.
    pub stations: Vec<Station>,
    /// Whole-Run reviewers, in `frontmatter.reviewers` order. These audit the
    /// finished Run end-to-end, after the final station's checkpoint.
    pub run_reviewers: Vec<Role>,
    /// Reflection dimensions, in `frontmatter.reflections` order. Evaluated at
    /// Run completion to produce learnings.
    pub reflections: Vec<Role>,
}

impl Factory {
    /// The factory's slug.
    pub fn name(&self) -> &str {
        &self.frontmatter.name
    }

    /// Find a station by slug.
    pub fn station(&self, name: &str) -> Option<&Station> {
        self.stations.iter().find(|s| s.name() == name)
    }

    /// Find a whole-Run reviewer by slug.
    pub fn run_reviewer(&self, name: &str) -> Option<&Role> {
        self.run_reviewers.iter().find(|r| r.name() == name)
    }

    /// Find a reflection dimension by slug.
    pub fn reflection(&self, name: &str) -> Option<&Role> {
        self.reflections.iter().find(|r| r.name() == name)
    }
}

#[cfg(test)]
mod role_kind_tests {
    use super::*;

    #[test]
    fn role_kind_parses_every_section_name() {
        assert_eq!(RoleKind::from_dir("explorers"), Some(RoleKind::Explorer));
        assert_eq!(RoleKind::from_dir("workers"), Some(RoleKind::Worker));
        assert_eq!(RoleKind::from_dir("reviewers"), Some(RoleKind::Reviewer));
        assert_eq!(RoleKind::from_dir("reflections"), Some(RoleKind::Reflection));
        assert_eq!(RoleKind::from_dir("nonsense"), None);
    }
}
