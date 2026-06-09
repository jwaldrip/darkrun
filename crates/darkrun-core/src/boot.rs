//! The boot recipe reader — `.darkrun/boot.md`.
//!
//! A repo can declare the services its gates depend on (a database, a queue, a
//! docker stack) and how to bring them up. When the gate classifier
//! ([`crate::gate_env`]) decides a gate failed because a dependency was down,
//! the engine reads this recipe and instructs the agent to best-effort boot the
//! declared services before retrying — or, if there's no recipe (or the tool is
//! missing), escalates to the operator.
//!
//! The engine is a pure read of on-disk state: it **parses and surfaces** the
//! recipe, it does not spawn or supervise processes. The agent has the shell and
//! runs the commands when it receives the boot instruction.
//!
//! ```markdown
//! ---
//! processes:
//!   - name: postgres
//!     command: [docker, compose, up, -d, db]
//!     service: true
//!     requires_tool: docker
//!     port: 5432
//!   - name: api
//!     command: [bin/rails, server]
//!     depends_on: [postgres]
//! primary: api
//! ---
//! Notes for the human/agent: gotchas, which routes matter.
//! ```

use std::path::Path;

use serde::Deserialize;

use crate::error::Result;
use crate::frontmatter;

/// One declared process in the boot recipe.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BootProcessSpec {
    /// A short name (`postgres`, `api`).
    pub name: String,
    /// The argv to run, e.g. `[docker, compose, up, -d, db]`.
    #[serde(default)]
    pub command: Vec<String>,
    /// Working directory to run the command in, relative to the repo root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// A TCP port that signals the service is up (for a reachability probe).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// An HTTP URL that returns 200 once the service is ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_url: Option<String>,
    /// Other process names this one depends on (boot order).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Whether this is a *service dependency* (booted best-effort) rather than
    /// the application itself.
    #[serde(default)]
    pub service: bool,
    /// The binary this process needs on `PATH` (fed to the classifier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_tool: Option<String>,
}

impl BootProcessSpec {
    /// The command rendered as a single shell-ish line, for surfacing to the
    /// agent in a boot instruction.
    pub fn command_line(&self) -> String {
        self.command.join(" ")
    }
}

/// A parsed boot recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootRecipe {
    /// Every declared process, in file order.
    pub processes: Vec<BootProcessSpec>,
    /// The application process name (never best-effort booted as a service).
    pub primary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BootFrontmatter {
    #[serde(default)]
    processes: Vec<BootProcessSpec>,
    #[serde(default)]
    primary: Option<String>,
}

/// Read `.darkrun/boot.md` under `darkrun_root` (which is [`crate::state::StateStore::root`]).
///
/// Returns `Ok(None)` when the file is absent or carries no processes (a
/// notes-only file is not an error); `Err` only on malformed frontmatter.
pub fn read_boot_recipe(darkrun_root: &Path) -> Result<Option<BootRecipe>> {
    let path = darkrun_root.join("boot.md");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    // A notes-only file with no frontmatter is fine — just no recipe.
    let fm: BootFrontmatter = match frontmatter::parse(&raw) {
        Ok((fm, _body)) => fm,
        Err(crate::error::CoreError::MissingFrontmatter) => return Ok(None),
        Err(e) => return Err(e),
    };
    if fm.processes.is_empty() {
        return Ok(None);
    }
    Ok(Some(BootRecipe {
        processes: fm.processes,
        primary: fm.primary,
    }))
}

/// The service-dependency processes — the ones a best-effort boot should start.
/// A process is a service if it's flagged `service: true`, or it declares a
/// `requires_tool` and isn't the `primary` application process.
pub fn service_processes(recipe: &BootRecipe) -> Vec<&BootProcessSpec> {
    recipe
        .processes
        .iter()
        .filter(|p| {
            let is_primary = recipe.primary.as_deref() == Some(p.name.as_str());
            p.service || (p.requires_tool.is_some() && !is_primary)
        })
        .collect()
}

/// The union of every `requires_tool` across the recipe's service processes —
/// the binaries the gate classifier should probe for on `PATH`.
pub fn required_tools(recipe: &BootRecipe) -> Vec<String> {
    let mut tools: Vec<String> = Vec::new();
    for p in service_processes(recipe) {
        if let Some(t) = &p.requires_tool {
            let t = t.trim().to_string();
            if !t.is_empty() && !tools.contains(&t) {
                tools.push(t);
            }
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn absent_file_is_none_not_error() {
        let dir = tmp();
        assert_eq!(read_boot_recipe(dir.path()).unwrap(), None);
    }

    #[test]
    fn notes_only_file_is_none() {
        let dir = tmp();
        std::fs::write(dir.path().join("boot.md"), "# how to boot\nrun the thing.\n").unwrap();
        assert_eq!(read_boot_recipe(dir.path()).unwrap(), None);
    }

    #[test]
    fn parses_a_multi_process_recipe_and_filters_services() {
        let dir = tmp();
        let md = concat!(
            "---\n",
            "processes:\n",
            "  - name: postgres\n",
            "    command: [docker, compose, up, -d, db]\n",
            "    service: true\n",
            "    requires_tool: docker\n",
            "    port: 5432\n",
            "  - name: api\n",
            "    command: [bin/rails, server]\n",
            "    depends_on: [postgres]\n",
            "primary: api\n",
            "---\n",
            "notes here\n",
        );
        std::fs::write(dir.path().join("boot.md"), md).unwrap();
        let recipe = read_boot_recipe(dir.path()).unwrap().expect("recipe");
        assert_eq!(recipe.processes.len(), 2);
        assert_eq!(recipe.primary.as_deref(), Some("api"));

        let services = service_processes(&recipe);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "postgres");
        assert_eq!(services[0].command_line(), "docker compose up -d db");

        assert_eq!(required_tools(&recipe), vec!["docker".to_string()]);
    }

    #[test]
    fn a_requires_tool_nonprimary_counts_as_a_service() {
        let dir = tmp();
        let md = concat!(
            "---\n",
            "processes:\n",
            "  - name: redis\n",
            "    command: [redis-server]\n",
            "    requires_tool: redis-server\n",
            "---\n",
        );
        std::fs::write(dir.path().join("boot.md"), md).unwrap();
        let recipe = read_boot_recipe(dir.path()).unwrap().expect("recipe");
        assert_eq!(service_processes(&recipe).len(), 1);
        assert_eq!(required_tools(&recipe), vec!["redis-server".to_string()]);
    }

    #[test]
    fn malformed_frontmatter_errors() {
        let dir = tmp();
        std::fs::write(
            dir.path().join("boot.md"),
            "---\nprocesses: [not, a, mapping, list]\n---\n",
        )
        .unwrap();
        assert!(read_boot_recipe(dir.path()).is_err());
    }
}
