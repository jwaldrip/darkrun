//! A pure-Rust `git diff <reference>` of the WORKING TREE against a commit —
//! the read the pre-checkpoint gate review shows the agent. gitoxide has no
//! `git diff` porcelain, so this assembles it: HEAD-tree vs on-disk content for
//! tracked paths, rendered as a unified diff (via imara-diff) with git's file
//! headers, plus a `--stat` summary. Read-only: no objects are written.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{GitError, Result};

/// Map any gix error into our crate error.
fn gix_err(e: impl std::fmt::Display) -> GitError {
    GitError::Gix(e.to_string())
}

/// Which rendering to produce.
#[derive(Clone, Copy)]
pub(crate) enum Format {
    /// `git diff --stat <ref>` — the changed-files + insertions/deletions summary.
    Stat,
    /// `git diff <ref>` — the full unified diff.
    Unified,
}

/// One tree entry's content identity: blob mode + bytes.
struct Side {
    mode: u32,
    data: Vec<u8>,
}

/// The per-file change we render.
struct FileChange {
    path: String,
    old: Option<Side>,
    new: Option<Side>,
}

/// Render the working tree's diff against `reference` in the requested format.
pub(crate) fn diff_worktree_against(
    repo: &gix::Repository,
    reference: &str,
    format: Format,
) -> Result<String> {
    let tree = repo
        .rev_parse_single(reference)
        .map_err(gix_err)?
        .object()
        .map_err(gix_err)?
        .peel_to_commit()
        .map_err(gix_err)?
        .tree()
        .map_err(gix_err)?;

    // HEAD side: path → (mode, oid).
    let mut head: BTreeMap<String, (u32, gix::ObjectId)> = BTreeMap::new();
    collect_tree(repo, &tree, "", &mut head)?;

    // Tracked set + the worktree root for reading on-disk content.
    let index = repo.index_or_empty().map_err(gix_err)?;
    let tracked: BTreeMap<String, ()> = index
        .entries()
        .iter()
        .filter(|e| e.stage() == gix::index::entry::Stage::Unconflicted)
        .map(|e| (e.path(&index).to_string(), ()))
        .collect();
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::Gix("diff: repository has no work tree".into()))?
        .to_path_buf();

    // Every path that could differ: union of HEAD and the tracked index.
    let mut paths: BTreeMap<String, ()> = BTreeMap::new();
    paths.extend(head.keys().map(|p| (p.clone(), ())));
    paths.extend(tracked.keys().map(|p| (p.clone(), ())));

    let mut changes: Vec<FileChange> = Vec::new();
    for path in paths.keys() {
        let old = head
            .get(path)
            .map(|(mode, oid)| -> Result<Side> {
                Ok(Side {
                    mode: *mode,
                    data: repo.find_object(*oid).map_err(gix_err)?.data.clone(),
                })
            })
            .transpose()?;

        // New side = on-disk content, but only for TRACKED paths (`git diff
        // <ref>` ignores untracked files). A tracked path missing on disk is a
        // deletion.
        let new = if tracked.contains_key(path) {
            read_worktree_side(&workdir, path)
        } else {
            None
        };

        // Unchanged (same mode + bytes) → not a change.
        let unchanged = match (&old, &new) {
            (Some(o), Some(n)) => o.mode == n.mode && o.data == n.data,
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            continue;
        }
        changes.push(FileChange {
            path: path.clone(),
            old,
            new,
        });
    }

    Ok(match format {
        Format::Stat => render_stat(&changes),
        Format::Unified => render_unified(repo, &changes)?,
    })
}

/// Read a tracked worktree path into a [`Side`], or `None` when it's absent
/// (a deletion). Symlinks contribute their target text; regular files their
/// bytes, with the executable bit reflected in the mode.
fn read_worktree_side(workdir: &Path, rel: &str) -> Option<Side> {
    let abs = workdir.join(rel);
    let meta = std::fs::symlink_metadata(&abs).ok()?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(&abs).ok()?;
        return Some(Side {
            mode: 0o120000,
            data: target.to_string_lossy().into_owned().into_bytes(),
        });
    }
    let data = std::fs::read(&abs).ok()?;
    let mode = if is_executable(&meta) { 0o100755 } else { 0o100644 };
    Some(Side { mode, data })
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}
#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// The paths whose blob (mode or content id) differs between two COMMIT
/// trees — `git diff --name-only <a> <b>`. Pure tree walk; no worktree read.
pub(crate) fn changed_paths_between_trees(
    repo: &gix::Repository,
    base_commit: gix::ObjectId,
    head_commit: gix::ObjectId,
) -> Result<Vec<String>> {
    let tree_of = |id: gix::ObjectId| -> Result<BTreeMap<String, (u32, gix::ObjectId)>> {
        let tree = repo
            .find_object(id)
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .tree()
            .map_err(gix_err)?;
        let mut out = BTreeMap::new();
        collect_tree(repo, &tree, "", &mut out)?;
        Ok(out)
    };
    let base = tree_of(base_commit)?;
    let head = tree_of(head_commit)?;
    let mut paths: Vec<String> = Vec::new();
    for (path, entry) in &head {
        if base.get(path) != Some(entry) {
            paths.push(path.clone());
        }
    }
    for path in base.keys() {
        if !head.contains_key(path) {
            paths.push(path.clone());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Recursively collect a tree's blob entries into `out` as `path → (mode, oid)`.
fn collect_tree(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    base: &str,
    out: &mut BTreeMap<String, (u32, gix::ObjectId)>,
) -> Result<()> {
    for entry in tree.iter() {
        let entry = entry.map_err(gix_err)?;
        let name = entry.filename().to_string();
        let full = if base.is_empty() {
            name
        } else {
            format!("{base}/{name}")
        };
        let mode = entry.mode();
        if mode.is_tree() {
            let sub = repo.find_tree(entry.oid()).map_err(gix_err)?;
            collect_tree(repo, &sub, &full, out)?;
        } else {
            out.insert(full, (mode.value() as u32, entry.oid().to_owned()));
        }
    }
    Ok(())
}

/// `git diff --stat` rendering: a per-file `path | N +++--` line plus the
/// `N files changed, X insertions(+), Y deletions(-)` summary.
fn render_stat(changes: &[FileChange]) -> String {
    if changes.is_empty() {
        return String::new();
    }
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let (mut total_ins, mut total_del) = (0usize, 0usize);
    let name_w = changes.iter().map(|c| c.path.len()).max().unwrap_or(0);
    for c in changes {
        let (ins, del) = line_counts(c);
        total_ins += ins;
        total_del += del;
        rows.push((c.path.clone(), ins, del));
    }
    let mut out = String::new();
    for (path, ins, del) in &rows {
        let total = *ins + *del;
        // Cap the +/- graph so a huge file doesn't blow up the line.
        let graph_total = total.min(60);
        let pluses = match total {
            0 => 0,
            t => (*ins * graph_total + t / 2) / t,
        };
        let minuses = graph_total - pluses;
        out.push_str(&format!(
            " {path:<name_w$} | {total} {}{}\n",
            "+".repeat(pluses),
            "-".repeat(minuses),
        ));
    }
    let files = changes.len();
    out.push_str(&format!(
        " {files} file{} changed",
        if files == 1 { "" } else { "s" }
    ));
    if total_ins > 0 {
        out.push_str(&format!(
            ", {total_ins} insertion{}(+)",
            if total_ins == 1 { "" } else { "s" }
        ));
    }
    if total_del > 0 {
        out.push_str(&format!(
            ", {total_del} deletion{}(-)",
            if total_del == 1 { "" } else { "s" }
        ));
    }
    out.push('\n');
    out
}

/// Insertions/deletions for a single file change (for `--stat`).
fn line_counts(change: &FileChange) -> (usize, usize) {
    let old = change.old.as_ref().map(|s| s.data.as_slice()).unwrap_or(b"");
    let new = change.new.as_ref().map(|s| s.data.as_slice()).unwrap_or(b"");
    if is_binary(old) || is_binary(new) {
        return (0, 0);
    }
    let (os, ns) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
    let input = gix::diff::blob::InternedInput::new(os.as_ref(), ns.as_ref());
    let diff = gix::diff::blob::Diff::compute(gix::diff::blob::Algorithm::Histogram, &input);
    let mut ins = 0;
    let mut del = 0;
    for hunk in diff.hunks() {
        del += (hunk.before.end - hunk.before.start) as usize;
        ins += (hunk.after.end - hunk.after.start) as usize;
    }
    (ins, del)
}

/// `git diff` rendering: per file a `diff --git` header, mode/index lines, the
/// `---`/`+++` markers, and the unified hunks (or a binary-files notice).
fn render_unified(repo: &gix::Repository, changes: &[FileChange]) -> Result<String> {
    let hash = repo.object_hash();
    let mut out = String::new();
    for c in changes {
        let a = format!("a/{}", c.path);
        let b = format!("b/{}", c.path);
        out.push_str(&format!("diff --git {a} {b}\n"));

        let old_mode = c.old.as_ref().map(|s| s.mode);
        let new_mode = c.new.as_ref().map(|s| s.mode);
        match (old_mode, new_mode) {
            (None, Some(m)) => out.push_str(&format!("new file mode {m:06o}\n")),
            (Some(m), None) => out.push_str(&format!("deleted file mode {m:06o}\n")),
            (Some(o), Some(n)) if o != n => {
                out.push_str(&format!("old mode {o:06o}\nnew mode {n:06o}\n"));
            }
            _ => {}
        }

        let old_oid = blob_id(hash, c.old.as_ref())?;
        let new_oid = blob_id(hash, c.new.as_ref())?;
        let old_hex = short_hex(&old_oid);
        let new_hex = short_hex(&new_oid);
        if old_mode == new_mode {
            let m = new_mode.or(old_mode).unwrap_or(0o100644);
            out.push_str(&format!("index {old_hex}..{new_hex} {m:06o}\n"));
        } else {
            out.push_str(&format!("index {old_hex}..{new_hex}\n"));
        }

        let old = c.old.as_ref().map(|s| s.data.as_slice()).unwrap_or(b"");
        let new = c.new.as_ref().map(|s| s.data.as_slice()).unwrap_or(b"");
        if is_binary(old) || is_binary(new) {
            out.push_str(&format!("Binary files {a} and {b} differ\n"));
            continue;
        }

        out.push_str(&format!(
            "--- {}\n",
            if c.old.is_some() { a.as_str() } else { "/dev/null" }
        ));
        out.push_str(&format!(
            "+++ {}\n",
            if c.new.is_some() { b.as_str() } else { "/dev/null" }
        ));

        let (os, ns) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
        let input = gix::diff::blob::InternedInput::new(os.as_ref(), ns.as_ref());
        let mut diff = gix::diff::blob::Diff::compute(gix::diff::blob::Algorithm::Histogram, &input);
        diff.postprocess_lines(&input);
        let body = diff
            .unified_diff(
                &gix::diff::blob::BasicLineDiffPrinter(&input.interner),
                gix::diff::blob::UnifiedDiffConfig::default(),
                &input,
            )
            .to_string();
        out.push_str(&body);
    }
    Ok(out)
}

/// The git blob id of a side's bytes (the empty/absent side hashes the empty
/// blob, matching git's `0000000` short form only after slicing). Side-effect
/// free — computes the hash without writing to the object database.
fn blob_id(hash: gix::hash::Kind, side: Option<&Side>) -> Result<gix::ObjectId> {
    let data = side.map(|s| s.data.as_slice()).unwrap_or(b"");
    gix::objs::compute_hash(hash, gix::object::Kind::Blob, data).map_err(gix_err)
}

/// Git's 7-hex short id.
fn short_hex(oid: &gix::ObjectId) -> String {
    oid.to_hex().to_string().chars().take(7).collect()
}

/// Git's binary heuristic: a NUL byte in the first 8000 bytes.
fn is_binary(data: &[u8]) -> bool {
    data.iter().take(8000).any(|&b| b == 0)
}
