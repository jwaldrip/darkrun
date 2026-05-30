//! Loaded content model — the parsed shape of the embedded factory corpus.
//!
//! These types mirror the markdown+frontmatter definitions under
//! `plugin/factories/<name>/`. A [`Factory`] is the top-level methodology; it
//! owns an ordered list of [`Station`]s; each station references named
//! [`Explorer`]s, [`Worker`]s, and [`Reviewer`]s by their file definitions.
//!
//! Frontmatter is parsed with `serde`; bodies are kept as raw markdown so the
//! manager can hand a role's instructions to an agent verbatim.

use darkrun_core::domain::CheckpointKind;
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
    /// Ordered station slugs, in cost-of-late-discovery order.
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
}

/// Frontmatter of a `STATION.md` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationFrontmatter {
    /// Station slug (e.g. `frame`).
    pub name: String,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Explorer slugs this station runs in its Explore phase.
    #[serde(default)]
    pub explorers: Vec<String>,
    /// Worker slugs, in Make -> Challenge -> Resolve sequence.
    #[serde(default)]
    pub workers: Vec<String>,
    /// Reviewer slugs that verify output in the Review phase.
    #[serde(default)]
    pub reviewers: Vec<String>,
    /// The checkpoint gate that ends the station.
    pub checkpoint: CheckpointKind,
    /// The durable artifact this station locks (e.g. `frame.md` or `code`).
    #[serde(default)]
    pub locked_artifact: String,
    /// Artifacts (from upstream stations) this station consumes.
    #[serde(default)]
    pub inputs: Vec<String>,
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

/// Frontmatter of a role definition (`explorers|workers|reviewers/*.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleFrontmatter {
    /// Role slug (matches the station's reference list entry).
    pub name: String,
    /// The kind of role.
    pub agent_type: RoleKind,
    /// Optional model override; falls back to the factory default.
    #[serde(default)]
    pub model: Option<String>,
}

/// A fully-loaded role: its frontmatter plus its raw markdown instructions.
#[derive(Debug, Clone)]
pub struct Role {
    /// Parsed frontmatter.
    pub frontmatter: RoleFrontmatter,
    /// Raw markdown body — the role's instructions, handed to an agent verbatim.
    pub body: String,
}

impl Role {
    /// The role's slug.
    pub fn name(&self) -> &str {
        &self.frontmatter.name
    }

    /// The role kind.
    pub fn kind(&self) -> RoleKind {
        self.frontmatter.agent_type
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
    /// The station's slug.
    pub fn name(&self) -> &str {
        &self.frontmatter.name
    }

    /// The checkpoint gate that ends this station.
    pub fn checkpoint(&self) -> CheckpointKind {
        self.frontmatter.checkpoint
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
