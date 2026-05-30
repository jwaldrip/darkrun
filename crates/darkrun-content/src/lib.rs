//! darkrun-content — the embedded factory corpus, its loader, and its validator.
//!
//! The `content/factories/<name>/` tree (markdown + YAML frontmatter) is
//! embedded into the binary at compile time via [`rust_embed`], so the single
//! `darkrun` binary ships its factory definitions inline — no files beside the
//! executable.
//!
//! A **Factory** is a methodology composed of ordered **Station**s. Each station
//! runs the universal slot `Explore -> Decompose -> Pass-loop -> Review ->
//! Checkpoint -> Lock` and references named **Explorer**s, **Worker**s
//! (Make -> Challenge -> Resolve), and **Reviewer**s defined as sibling files.
//!
//! ## Loader API
//! - [`list_factories`] — slugs of every embedded factory.
//! - [`load_factory`] — parse a factory and its stations/roles into the model.
//! - [`load_validated`] — load, then enforce structural validity.
//! - [`validate`] — validate an already-loaded factory.
//!
//! ```
//! let factory = darkrun_content::load_validated("software").unwrap();
//! assert_eq!(factory.name(), "software");
//! ```

mod error;
mod loader;
mod model;
mod validate;

pub use error::{ContentError, Result};
pub use loader::{list_factories, load_factory, load_validated};
pub use model::{
    Factory, FactoryFrontmatter, Role, RoleFrontmatter, RoleKind, Station, StationFrontmatter,
};
pub use validate::validate;

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_core::domain::CheckpointKind;

    #[test]
    fn lists_the_software_factory() {
        let factories = list_factories();
        assert!(
            factories.contains(&"software".to_string()),
            "expected `software` in {factories:?}"
        );
    }

    #[test]
    fn loads_and_validates_software_factory() {
        let factory = load_validated("software").expect("software factory loads and validates");

        assert_eq!(factory.name(), "software");
        assert_eq!(factory.frontmatter.default_model, "sonnet");

        // The six redesigned stations, in cost-of-late-discovery order.
        let names: Vec<&str> = factory.stations.iter().map(Station::name).collect();
        assert_eq!(
            names,
            vec!["frame", "specify", "shape", "build", "prove", "harden"]
        );

        // Body carries the station purpose, not just frontmatter.
        assert!(factory.body.contains("class-of-risk-eliminated"));
    }

    #[test]
    fn frame_station_has_full_role_set() {
        let factory = load_factory("software").expect("load");
        let frame = factory.station("frame").expect("frame station");

        assert_eq!(frame.checkpoint(), CheckpointKind::Ask);
        assert_eq!(frame.frontmatter.locked_artifact, "frame.md");

        // Explorers: context + value.
        let explorers: Vec<&str> = frame.explorers.iter().map(Role::name).collect();
        assert_eq!(explorers, vec!["context", "value"]);

        // Workers in Make -> Challenge -> Resolve order, terminal Distiller.
        let workers: Vec<&str> = frame.workers.iter().map(Role::name).collect();
        assert_eq!(workers, vec!["framer", "challenger", "distiller"]);

        // Reviewers.
        let reviewers: Vec<&str> = frame.reviewers.iter().map(Role::name).collect();
        assert_eq!(reviewers, vec!["value", "feasibility"]);

        // Each role kind is correctly tagged, and bodies carry instructions.
        assert_eq!(frame.workers[0].kind(), RoleKind::Worker);
        assert!(frame.workers[0].body.contains("Framer"));
        assert!(frame.explorers[0].body.to_lowercase().contains("context"));
    }

    #[test]
    fn shape_station_has_visual_designer_spiker_and_five_workers() {
        let factory = load_factory("software").expect("load");
        let shape = factory.station("shape").expect("shape station");

        // Shape's pass-loop is Designer -> VisualDesigner -> Spiker ->
        // PressureTester -> Resolver. The VisualDesigner beat owns the visual/UX
        // facet for user-facing work.
        let workers: Vec<&str> = shape.workers.iter().map(Role::name).collect();
        assert_eq!(
            workers,
            vec![
                "designer",
                "visual_designer",
                "spiker",
                "pressure_tester",
                "resolver"
            ]
        );
        // The Spiker builds a throwaway proof of risky assumptions.
        let spiker = shape
            .workers
            .iter()
            .find(|w| w.name() == "spiker")
            .expect("spiker present");
        assert!(spiker.body.to_lowercase().contains("throwaway"));

        // The VisualDesigner directs the operator to generate options and use the
        // visual-decision tools before any UI is built.
        let visual = shape
            .workers
            .iter()
            .find(|w| w.name() == "visual_designer")
            .expect("visual_designer present");
        let body = visual.body.to_lowercase();
        assert!(body.contains("darkrun_question") || body.contains("darkrun_direction"));
    }

    #[test]
    fn checkpoints_match_the_design() {
        let factory = load_factory("software").expect("load");
        let kind = |s: &str| factory.station(s).unwrap().checkpoint();
        assert_eq!(kind("frame"), CheckpointKind::Ask);
        assert_eq!(kind("specify"), CheckpointKind::Ask);
        assert_eq!(kind("shape"), CheckpointKind::Ask);
        assert_eq!(kind("build"), CheckpointKind::Auto);
        assert_eq!(kind("prove"), CheckpointKind::Ask);
        assert_eq!(kind("harden"), CheckpointKind::External);
    }

    #[test]
    fn missing_factory_is_an_error() {
        let err = load_factory("nonexistent").unwrap_err();
        assert!(matches!(err, ContentError::FactoryNotFound(_)));
    }
}
