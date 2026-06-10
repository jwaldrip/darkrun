//! Run-level reflections — the durable home for the Reflect phase.
//!
//! The Reflect phase runs an autonomous retrospective at the end of each
//! station; its learnings collect here so they survive the run and inform later
//! stations. Storage mirrors feedback: one `reflections/<id>.md` doc per
//! reflection, a YAML frontmatter fence (`station`, `created_at`) over a
//! markdown body, on
//! [`StateStore::read_reflections_raw`](darkrun_core::StateStore::read_reflections_raw)
//! / [`write_reflection_raw`](darkrun_core::StateStore::write_reflection_raw).

use chrono::Utc;
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::Result;

/// A captured retrospective.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reflection {
    /// The reflection id (`refl-NN`).
    pub id: String,
    /// The station this reflection came out of (empty for a run-level note).
    pub station: String,
    /// RFC3339 capture time.
    pub created_at: String,
    /// The reflection prose.
    pub body: String,
}

/// Record a reflection for a run, minting the next `refl-NN` id. The station is
/// optional — a run-level reflection carries an empty station.
pub fn record(
    store: &StateStore,
    slug: &str,
    station: Option<String>,
    body: &str,
) -> Result<Reflection> {
    let existing = store.read_reflections_raw(slug)?;
    let id = format!("refl-{:02}", existing.len() + 1);
    let created_at = Utc::now().to_rfc3339();
    let station = station.unwrap_or_default();
    let doc = format!(
        "---\nstation: {station}\ncreated_at: {created_at}\n---\n{}\n",
        body.trim()
    );
    store.write_reflection_raw(slug, &id, &doc)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: reflection {id}"));
    Ok(Reflection {
        id,
        station,
        created_at,
        body: body.trim().to_string(),
    })
}

/// List every reflection for a run, oldest id first.
pub fn list(store: &StateStore, slug: &str) -> Result<Vec<Reflection>> {
    let raw = store.read_reflections_raw(slug)?;
    Ok(raw.into_iter().map(|(id, doc)| parse(id, &doc)).collect())
}

/// Parse a reflection document into a [`Reflection`]. Tolerant of a missing
/// frontmatter fence (everything becomes the body).
fn parse(id: String, doc: &str) -> Reflection {
    let doc = doc.trim_start_matches('\u{feff}');
    let field = |fm: &str, key: &str| -> String {
        fm.lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix(key)
                    .map(|v| v.trim().trim_matches('"').to_string())
            })
            .unwrap_or_default()
    };
    if let Some(rest) = doc.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n').trim().to_string();
            return Reflection {
                id,
                station: field(fm, "station:"),
                created_at: field(fm, "created_at:"),
                body,
            };
        }
    }
    Reflection {
        id,
        station: String::new(),
        created_at: String::new(),
        body: doc.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().unwrap();
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn record_then_list_round_trips() {
        let (_d, store) = store();
        let r = record(&store, "r", Some("frame".into()), "  Frame fought back on scope.  ").unwrap();
        assert_eq!(r.id, "refl-01");
        assert_eq!(r.station, "frame");
        assert!(!r.created_at.is_empty());
        assert_eq!(r.body, "Frame fought back on scope.");

        let all = list(&store, "r").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], r);
    }

    #[test]
    fn ids_increment_across_reflections() {
        let (_d, store) = store();
        record(&store, "r", Some("frame".into()), "one").unwrap();
        record(&store, "r", Some("specify".into()), "two").unwrap();
        let r3 = record(&store, "r", None, "three").unwrap();
        assert_eq!(r3.id, "refl-03");
        assert_eq!(r3.station, "");
        assert_eq!(list(&store, "r").unwrap().len(), 3);
    }

    #[test]
    fn list_is_empty_when_none_recorded() {
        let (_d, store) = store();
        assert!(list(&store, "r").unwrap().is_empty());
    }

    #[test]
    fn parse_falls_back_for_a_doc_without_frontmatter() {
        // A reflection doc with no `---` fence parses to a body-only record.
        let r = parse("refl-99".into(), "  a bare note, no frontmatter  ");
        assert_eq!(r.id, "refl-99");
        assert_eq!(r.station, "");
        assert_eq!(r.created_at, "");
        assert_eq!(r.body, "a bare note, no frontmatter");
    }
}
