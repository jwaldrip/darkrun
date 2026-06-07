//! Pure-Rust [`GitBackend`] over **gitoxide** (`gix`) — no C, no `git` CLI.
//!
//! Built incrementally behind the [`GitBackend`](crate::GitBackend) trait and
//! validated against the same real-git conformance fixtures as the libgit2 and
//! shell backends (they must AGREE). Operations gitoxide doesn't yet provide
//! (push/rebase/merge/worktree-create) return [`GitError::Unsupported`] until
//! their build-out phase lands, so the migration flips on one operation at a
//! time without ever breaking the working engine.

use std::path::{Path, PathBuf};

use crate::backend::{CreateOptions, GitBackend, MergeOutcome, WorktreeInfo};
use crate::error::{GitError, Result};

/// A pure-Rust git backend driven by gitoxide.
pub struct GixBackend {
    repo_root: PathBuf,
}

/// Map any gix error into our crate error.
fn gix_err(e: impl std::fmt::Display) -> GitError {
    GitError::Gix(e.to_string())
}

/// The short branch name HEAD points at, or `None` when HEAD is detached.
fn head_branch_of(repo: &gix::Repository) -> Option<String> {
    repo.head_name()
        .ok()
        .flatten()
        .map(|n| n.shorten().to_string())
}

// ── write-tree (built ourselves: gix has no `git write-tree`) ────────────────

/// A node in the tree assembled from index entries: a blob leaf or a subtree.
enum TreeNode {
    Blob {
        mode: gix::objs::tree::EntryMode,
        oid: gix::ObjectId,
    },
    Dir(std::collections::BTreeMap<Vec<u8>, TreeNode>),
}

/// Map an index entry's mode to a tree entry mode.
fn index_mode_to_tree(mode: gix::index::entry::Mode) -> Result<gix::objs::tree::EntryMode> {
    use gix::objs::tree::EntryKind;
    let kind = match mode.bits() {
        0o100644 => EntryKind::Blob,
        0o100755 => EntryKind::BlobExecutable,
        0o120000 => EntryKind::Link,
        0o160000 => EntryKind::Commit, // gitlink / submodule
        other => return Err(GitError::Gix(format!("unsupported index mode {other:o}"))),
    };
    Ok(kind.into())
}

/// Insert a slash-separated `path` into the nested tree node map.
fn insert_path(
    dir: &mut std::collections::BTreeMap<Vec<u8>, TreeNode>,
    path: &[u8],
    mode: gix::objs::tree::EntryMode,
    oid: gix::ObjectId,
) {
    match path.iter().position(|&b| b == b'/') {
        Some(i) => {
            let name = path[..i].to_vec();
            let rest = &path[i + 1..];
            let sub = dir
                .entry(name)
                .or_insert_with(|| TreeNode::Dir(std::collections::BTreeMap::new()));
            if let TreeNode::Dir(m) = sub {
                insert_path(m, rest, mode, oid);
            }
        }
        None => {
            dir.insert(path.to_vec(), TreeNode::Blob { mode, oid });
        }
    }
}

/// Git's canonical tree-entry ordering: byte order on the name, but a tree
/// (directory) entry sorts as if its name had a trailing `/`.
fn tree_sort_key(name: &[u8], mode: gix::objs::tree::EntryMode) -> Vec<u8> {
    let mut k = name.to_vec();
    if mode.is_tree() {
        k.push(b'/');
    }
    k
}

/// Write a single tree node (recursively writing its subtrees) and return its id.
fn write_tree_node(
    repo: &gix::Repository,
    dir: &std::collections::BTreeMap<Vec<u8>, TreeNode>,
) -> Result<gix::ObjectId> {
    let mut entries: Vec<gix::objs::tree::Entry> = Vec::with_capacity(dir.len());
    for (name, node) in dir {
        let (mode, oid) = match node {
            TreeNode::Blob { mode, oid } => (*mode, *oid),
            TreeNode::Dir(sub) => (
                gix::objs::tree::EntryKind::Tree.into(),
                write_tree_node(repo, sub)?,
            ),
        };
        entries.push(gix::objs::tree::Entry {
            mode,
            filename: name.clone().into(),
            oid,
        });
    }
    entries.sort_by(|a, b| {
        tree_sort_key(&a.filename, a.mode).cmp(&tree_sort_key(&b.filename, b.mode))
    });
    let tree = gix::objs::Tree { entries };
    Ok(repo.write_object(&tree).map_err(gix_err)?.detach())
}

/// Whether a worktree file's mode has any execute bit set (regular files only).
#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}
#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Upsert `(path, blob)` entries into the worktree index at stage 0 with `mode`,
/// then write the index back. Replaces any existing entries for those paths and
/// re-sorts to keep the index canonical. A no-op for an empty batch.
fn upsert_index_entries(
    repo: &gix::Repository,
    entries: &[(Vec<u8>, gix::ObjectId)],
    mode: gix::index::entry::Mode,
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    use gix::index::entry::{Flags, Stat};
    let mut index = repo.open_index().map_err(gix_err)?;
    let paths: std::collections::HashSet<&[u8]> =
        entries.iter().map(|(p, _)| p.as_slice()).collect();
    index.remove_entries(|_, p, _| paths.contains(p.as_ref() as &[u8]));
    for (path, oid) in entries {
        index.dangerously_push_entry(
            Stat::default(),
            *oid,
            Flags::empty(),
            mode,
            gix::bstr::BStr::new(path),
        );
    }
    index.sort_entries();
    index
        .write(gix::index::write::Options::default())
        .map_err(gix_err)?;
    Ok(())
}

/// Assemble and write the tree for the (stage-0) index — our `git write-tree`.
fn write_index_tree(repo: &gix::Repository, index: &gix::index::State) -> Result<gix::ObjectId> {
    let mut root = std::collections::BTreeMap::new();
    for entry in index.entries() {
        if entry.stage() != gix::index::entry::Stage::Unconflicted {
            return Err(GitError::Gix(
                "cannot write a tree from a conflicted index".into(),
            ));
        }
        let mode = index_mode_to_tree(entry.mode)?;
        insert_path(&mut root, entry.path(index).as_ref(), mode, entry.id);
    }
    write_tree_node(repo, &root)
}

/// Recursively collect blob (file) paths under `tree`, slash-joined relative to
/// the tree root — the building block for a recursive `ls-tree`.
fn collect_tree_paths(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    base: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    for entry in tree.iter() {
        let entry = entry.map_err(gix_err)?;
        let name = entry.filename().to_string();
        let full = if base.is_empty() {
            name
        } else {
            format!("{base}/{name}")
        };
        if entry.mode().is_tree() {
            let sub = repo.find_tree(entry.oid()).map_err(gix_err)?;
            collect_tree_paths(repo, &sub, &full, out)?;
        } else {
            out.push(full);
        }
    }
    Ok(())
}

impl GixBackend {
    /// Open the git repository that contains `repo_root` (walks up, like
    /// `Repository::discover` / running `git` from a subdirectory).
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref();
        gix::discover(root).map_err(|_| GitError::NotARepo(root.to_path_buf()))?;
        Ok(Self {
            repo_root: root.to_path_buf(),
        })
    }

    /// The repository root this backend was opened against.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Open the gix repository rooted at this backend's path.
    fn repo(&self) -> Result<gix::Repository> {
        gix::discover(&self.repo_root).map_err(|_| GitError::NotARepo(self.repo_root.clone()))
    }
}

impl GitBackend for GixBackend {
    // ── Reads (native in gitoxide) ───────────────────────────────────────────

    fn current_branch(&self) -> Result<Option<String>> {
        Ok(head_branch_of(&self.repo()?))
    }

    fn is_clean(&self) -> Result<bool> {
        // `is_dirty()` is tracked-only; the engine (like `git status --porcelain`
        // / libgit2) treats UNTRACKED non-ignored files as dirty too. Drive the
        // full status with untracked files included and stop at the first change.
        let repo = self.repo()?;
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_err)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(None)
            .map_err(gix_err)?;
        for item in iter {
            item.map_err(gix_err)?;
            return Ok(false);
        }
        Ok(true)
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        let repo = self.repo()?;
        let full = format!("refs/heads/{name}");
        Ok(repo.try_find_reference(&full).map_err(gix_err)?.is_some())
    }

    // ── Not yet built (later phases) ─────────────────────────────────────────

    fn create_worktree(
        &self,
        _name: &str,
        _path: &Path,
        _opts: &CreateOptions,
    ) -> Result<WorktreeInfo> {
        Err(GitError::Unsupported("create_worktree"))
    }

    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let repo = self.repo()?;
        let mut out = Vec::new();

        // The primary working tree isn't enumerated by `worktrees()`; include it
        // explicitly as "(main)" so callers see the complete picture.
        if let Some(workdir) = repo.work_dir() {
            out.push(WorktreeInfo {
                name: "(main)".to_string(),
                path: workdir.to_path_buf(),
                branch: head_branch_of(&repo),
                locked: false,
            });
        }

        for proxy in repo.worktrees().map_err(gix_err)? {
            let name = proxy.id().to_string();
            let path = proxy.base().map_err(gix_err)?;
            // Open the linked worktree to read its checked-out branch.
            let branch = proxy
                .clone()
                .into_repo_with_possibly_inaccessible_worktree()
                .ok()
                .and_then(|r| head_branch_of(&r));
            let locked = proxy.is_locked();
            out.push(WorktreeInfo {
                name,
                path,
                branch,
                locked,
            });
        }

        Ok(out)
    }

    fn remove_worktree(&self, _name: &str, _force: bool) -> Result<()> {
        Err(GitError::Unsupported("remove_worktree"))
    }

    fn create_branch(&self, name: &str, from_ref: &str) -> Result<()> {
        // Idempotent: a no-op when the branch already exists (matches the other
        // backends). Does not check the branch out.
        if self.branch_exists(name)? {
            return Ok(());
        }
        let repo = self.repo()?;
        let target = repo
            .rev_parse_single(from_ref)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        repo.reference(
            format!("refs/heads/{name}"),
            target,
            gix::refs::transaction::PreviousValue::MustNotExist,
            format!("branch: created from {from_ref}"),
        )
        .map_err(gix_err)?;
        Ok(())
    }

    fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> Result<bool> {
        let repo = self.repo()?;
        let a = repo
            .rev_parse_single(maybe_ancestor)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        let b = repo
            .rev_parse_single(descendant)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        if a == b {
            return Ok(true);
        }
        // `a` is an ancestor of `b` iff their merge base is `a` itself.
        let base = repo.merge_base(a, b).map_err(gix_err)?;
        Ok(base.detach() == a)
    }

    fn merge_no_commit(&self, _worktree_path: &Path, _source_ref: &str) -> Result<MergeOutcome> {
        Err(GitError::Unsupported("merge_no_commit"))
    }

    fn merge_in_progress(&self, worktree_path: &Path) -> Result<bool> {
        // A merge is in progress when MERGE_HEAD exists in the (worktree's) git
        // dir. `gix::open` resolves the correct git dir whether `worktree_path`
        // is the primary checkout or a linked worktree.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        Ok(repo.git_dir().join("MERGE_HEAD").exists())
    }

    fn checkout_paths(&self, worktree_path: &Path, from_ref: &str, paths: &[String]) -> Result<()> {
        // Restore each path's content from `from_ref` into the worktree AND the
        // index (the engine-protected restore: `git checkout <ref> -- <paths>`).
        // The engine only restores its own regular state files, so mode is FILE.
        use gix::index::entry::Mode;
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let mut staged: Vec<(Vec<u8>, gix::ObjectId)> = Vec::new();
        for rel in paths {
            // `<ref>:<path>` resolves the blob at that path; skip if absent there.
            let Ok(id) = repo.rev_parse_single(format!("{from_ref}:{rel}").as_str()) else {
                continue;
            };
            let oid = id.detach();
            let data = repo.find_object(oid).map_err(gix_err)?.data.clone();
            let abs = worktree_path.join(rel);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).map_err(|e| GitError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            std::fs::write(&abs, &data).map_err(|e| GitError::Io {
                path: abs.clone(),
                source: e,
            })?;
            staged.push((rel.as_bytes().to_vec(), oid));
        }
        upsert_index_entries(&repo, &staged, Mode::FILE)?;
        Ok(())
    }

    fn add_paths(&self, worktree_path: &Path, paths: &[String]) -> Result<()> {
        // Stage each path's current worktree content into the index
        // (`git add -- <paths>`). Writes a blob per file, then upserts the index.
        use gix::index::entry::Mode;
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        // Group by mode so each upsert batch shares one mode (engine state files
        // are regular; executables/symlinks handled in their own batches).
        let mut regular: Vec<(Vec<u8>, gix::ObjectId)> = Vec::new();
        let mut exec: Vec<(Vec<u8>, gix::ObjectId)> = Vec::new();
        let mut links: Vec<(Vec<u8>, gix::ObjectId)> = Vec::new();
        for rel in paths {
            let abs = worktree_path.join(rel);
            let meta = std::fs::symlink_metadata(&abs).map_err(|e| GitError::Io {
                path: abs.clone(),
                source: e,
            })?;
            let key = rel.as_bytes().to_vec();
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&abs).map_err(|e| GitError::Io {
                    path: abs.clone(),
                    source: e,
                })?;
                let oid = repo
                    .write_blob(target.to_string_lossy().as_bytes())
                    .map_err(gix_err)?
                    .detach();
                links.push((key, oid));
            } else {
                let bytes = std::fs::read(&abs).map_err(|e| GitError::Io {
                    path: abs.clone(),
                    source: e,
                })?;
                let oid = repo.write_blob(&bytes).map_err(gix_err)?.detach();
                if is_executable(&meta) {
                    exec.push((key, oid));
                } else {
                    regular.push((key, oid));
                }
            }
        }
        upsert_index_entries(&repo, &regular, Mode::FILE)?;
        upsert_index_entries(&repo, &exec, Mode::FILE_EXECUTABLE)?;
        upsert_index_entries(&repo, &links, Mode::SYMLINK)?;
        Ok(())
    }

    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()> {
        // gix has commit()/write_blob() but no `git write-tree`, so build the
        // tree from the staged index ourselves, then commit it on HEAD with the
        // current HEAD as parent (no parent on an unborn branch).
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let index = repo.index_or_empty().map_err(gix_err)?;
        let tree_id = write_index_tree(&repo, &index)?;
        let parents: Vec<gix::ObjectId> =
            repo.head_commit().ok().map(|c| c.id).into_iter().collect();
        repo.commit("HEAD", message, tree_id, parents)
            .map_err(gix_err)?;
        Ok(())
    }

    fn ls_tree(&self, worktree_path: &Path, from_ref: &str, prefix: &str) -> Result<Vec<String>> {
        // Recursive tracked paths under `prefix` at `from_ref`, mirroring
        // `git ls-tree -r --name-only <from_ref> -- <prefix>`.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let tree = repo
            .rev_parse_single(from_ref)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_tree()
            .map_err(gix_err)?;
        let mut out = Vec::new();
        collect_tree_paths(&repo, &tree, "", &mut out)?;
        let prefix = prefix.trim_matches('/');
        if !prefix.is_empty() {
            out.retain(|p| p == prefix || p.starts_with(&format!("{prefix}/")));
        }
        Ok(out)
    }

    fn unresolved_paths(&self, worktree_path: &Path) -> Result<Vec<String>> {
        // Conflicted paths = index entries carrying a non-zero merge stage,
        // mirroring `git diff --name-only --diff-filter=U`.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let index = repo.index_or_empty().map_err(gix_err)?;
        let mut out: Vec<String> = Vec::new();
        for entry in index.entries() {
            if entry.stage() != gix::index::entry::Stage::Unconflicted {
                let path = entry.path(&index).to_string();
                if !out.contains(&path) {
                    out.push(path);
                }
            }
        }
        Ok(out)
    }

    fn refs_have_identical_trees(&self, ref_a: &str, ref_b: &str) -> Result<bool> {
        // Contract: never errors — any resolution failure is `false` (the merge-
        // debt short-circuit is an optimization, not a correctness gate).
        let Ok(repo) = self.repo() else {
            return Ok(false);
        };
        let tree_of = |spec: &str| -> Option<gix::ObjectId> {
            Some(
                repo.rev_parse_single(spec)
                    .ok()?
                    .object()
                    .ok()?
                    .peel_to_commit()
                    .ok()?
                    .tree_id()
                    .ok()?
                    .detach(),
            )
        };
        match (tree_of(ref_a), tree_of(ref_b)) {
            (Some(a), Some(b)) => Ok(a == b),
            _ => Ok(false),
        }
    }

    fn push(&self, _worktree_path: &Path, _branch: &str) -> Result<()> {
        Err(GitError::Unsupported("push"))
    }

    fn fetch(&self, _worktree_path: &Path, _branch: &str) -> Result<()> {
        Err(GitError::Unsupported("fetch"))
    }

    fn rebase_onto(&self, _worktree_path: &Path, _upstream: &str) -> Result<()> {
        Err(GitError::Unsupported("rebase_onto"))
    }

    fn rebase_abort(&self, _worktree_path: &Path) -> Result<()> {
        Err(GitError::Unsupported("rebase_abort"))
    }
}
