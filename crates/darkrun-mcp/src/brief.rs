//! Station briefs + closing outcomes — the durable narrative artifacts a
//! station emits around the operator gates (the predecessor's `BRIEF.md` with
//! `phase: pre` / `phase: post`).
//!
//! - A **pre** brief ("what I'm going to do") is written in the Review phase,
//!   before the pre-execution user gate — the record the operator reviews.
//! - A **post** outcome ("what the station actually produced") is written at the
//!   Checkpoint, before the station locks — the durable record of why it's
//!   allowed to lock.
//!
//! Storage mirrors reflections: one `briefs/<station>-<phase>.md` doc per
//! (station, phase), a YAML frontmatter fence (`station`, `phase`, `created_at`)
//! over a markdown body, on
//! [`read_briefs_raw`](darkrun_core::StateStore::read_briefs_raw) /
//! [`write_brief_raw`](darkrun_core::StateStore::write_brief_raw). The stable id
//! means a re-emitted brief overwrites in place.

use chrono::Utc;
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::Result;

/// Which side of the station's gates a brief belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BriefPhase {
    /// The pre-execution brief, surfaced before the review/user gate.
    Pre,
    /// The closing outcome, surfaced before the checkpoint locks.
    Post,
}

impl BriefPhase {
    /// The wire token (`pre` / `post`).
    pub fn as_str(self) -> &'static str {
        match self {
            BriefPhase::Pre => "pre",
            BriefPhase::Post => "post",
        }
    }

    /// Parse a token, accepting `pre`/`brief` and `post`/`outcome`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pre" | "brief" => Some(BriefPhase::Pre),
            "post" | "outcome" => Some(BriefPhase::Post),
            _ => None,
        }
    }
}

/// A captured station brief or outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Brief {
    /// The doc id (`<station>-<phase>`).
    pub id: String,
    /// The station this brief belongs to.
    pub station: String,
    /// Pre (brief) or post (outcome).
    pub phase: BriefPhase,
    /// RFC3339 capture time.
    pub created_at: String,
    /// The narrative prose.
    pub body: String,
}

/// Record a station brief/outcome, overwriting any prior doc for that
/// (station, phase). Returns the persisted [`Brief`].
pub fn record(
    store: &StateStore,
    slug: &str,
    station: &str,
    phase: BriefPhase,
    body: &str,
) -> Result<Brief> {
    let created_at = Utc::now().to_rfc3339();
    let id = format!("{station}-{}", phase.as_str());
    let doc = format!(
        "---\nstation: {station}\nphase: {}\ncreated_at: {created_at}\n---\n{}\n",
        phase.as_str(),
        body.trim()
    );
    store.write_brief_raw(slug, &id, &doc)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: brief {id}"));
    Ok(Brief {
        id,
        station: station.to_string(),
        phase,
        created_at,
        body: body.trim().to_string(),
    })
}

/// List every brief/outcome for a run, oldest id first.
pub fn list(store: &StateStore, slug: &str) -> Result<Vec<Brief>> {
    let raw = store.read_briefs_raw(slug)?;
    Ok(raw.into_iter().map(|(id, doc)| parse(id, &doc)).collect())
}

/// Parse a brief document. Tolerant of a missing frontmatter fence (everything
/// becomes the body; phase defaults to Pre).
fn parse(id: String, doc: &str) -> Brief {
    let doc = doc.trim_start_matches('\u{feff}');
    let field = |fm: &str, key: &str| -> String {
        fm.lines()
            .find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().trim_matches('"').to_string()))
            .unwrap_or_default()
    };
    if let Some(rest) = doc.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n').trim().to_string();
            return Brief {
                id,
                station: field(fm, "station:"),
                phase: BriefPhase::parse(&field(fm, "phase:")).unwrap_or(BriefPhase::Pre),
                created_at: field(fm, "created_at:"),
                body,
            };
        }
    }
    Brief {
        id,
        station: String::new(),
        phase: BriefPhase::Pre,
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
    fn record_persists_pre_and_post_then_round_trips() {
        let (_d, store) = store();
        record(&store, "r", "frame", BriefPhase::Pre, "  going to frame X  ").unwrap();
        record(&store, "r", "frame", BriefPhase::Post, "framed X, locked").unwrap();

        let all = list(&store, "r").unwrap();
        assert_eq!(all.len(), 2);
        let pre = all.iter().find(|b| b.phase == BriefPhase::Pre).unwrap();
        assert_eq!(pre.id, "frame-pre");
        assert_eq!(pre.station, "frame");
        assert_eq!(pre.body, "going to frame X");
        let post = all.iter().find(|b| b.phase == BriefPhase::Post).unwrap();
        assert_eq!(post.id, "frame-post");
        assert_eq!(post.body, "framed X, locked");
    }

    #[test]
    fn re_recording_a_phase_overwrites_in_place() {
        let (_d, store) = store();
        record(&store, "r", "frame", BriefPhase::Pre, "first").unwrap();
        record(&store, "r", "frame", BriefPhase::Pre, "revised").unwrap();
        let all = list(&store, "r").unwrap();
        assert_eq!(all.len(), 1, "same (station, phase) overwrites");
        assert_eq!(all[0].body, "revised");
    }

    #[test]
    fn phase_parse_accepts_aliases() {
        assert_eq!(BriefPhase::parse("pre"), Some(BriefPhase::Pre));
        assert_eq!(BriefPhase::parse("BRIEF"), Some(BriefPhase::Pre));
        assert_eq!(BriefPhase::parse("post"), Some(BriefPhase::Post));
        assert_eq!(BriefPhase::parse("Outcome"), Some(BriefPhase::Post));
        assert_eq!(BriefPhase::parse("nope"), None);
    }
}
