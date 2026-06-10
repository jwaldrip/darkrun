//! Pure-Rust [`GitBackend`] over **gitoxide** (`gix`) — no C, no `git` CLI.
//!
//! The full trait is implemented and validated against the same real-git
//! conformance fixtures as the libgit2 and shell backends (they must AGREE).
//! Where gitoxide has no high-level API, the internals are built here over its
//! plumbing: `git write-tree` (tree assembly from the index), linked-worktree
//! creation (admin files + checkout), the engine-protected three-way merge, and
//! `git push`/`git rebase` (the receive-pack client in [`push`](crate::push)
//! and a cherry-pick replay loop). No operation falls back to a subprocess.

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

/// Check out every entry of `tree` into `dir` (recursively), writing blobs as
/// files, executables with the +x bit, and symlinks as real links — the
/// worktree-population half of `git worktree add`.
fn checkout_tree_into(repo: &gix::Repository, tree: &gix::Tree<'_>, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for entry in tree.iter() {
        let entry = entry.map_err(gix_err)?;
        let dest = dir.join(entry.filename().to_string());
        let mode = entry.mode();
        if mode.is_tree() {
            let sub = repo.find_tree(entry.oid()).map_err(gix_err)?;
            checkout_tree_into(repo, &sub, &dest)?;
        } else if mode.is_link() {
            let data = repo.find_object(entry.oid()).map_err(gix_err)?.data.clone();
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(&data), &dest)?;
            }
            #[cfg(not(unix))]
            std::fs::write(&dest, &data)?;
        } else {
            let data = repo.find_object(entry.oid()).map_err(gix_err)?.data.clone();
            std::fs::write(&dest, &data)?;
            #[cfg(unix)]
            if matches!(mode.kind(), gix::objs::tree::EntryKind::BlobExecutable) {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
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

/// Point the ref the worktree HEAD follows (a branch, or detached HEAD itself)
/// at `target`, then refresh the worktree index + files to `target`'s tree.
/// The move-the-tip-then-check-out step shared by rebase replay and abort.
fn move_branch_and_checkout(
    repo: &gix::Repository,
    branch: Option<&str>,
    target: gix::ObjectId,
) -> Result<()> {
    match branch {
        Some(name) => {
            repo.reference(
                format!("refs/heads/{name}"),
                target,
                gix::refs::transaction::PreviousValue::Any,
                "rebase: move branch",
            )
            .map_err(gix_err)?;
        }
        // Detached HEAD: write the oid straight into the worktree's HEAD file.
        None => {
            std::fs::write(repo.git_dir().join("HEAD"), format!("{target}\n"))?;
        }
    }

    let tree_id = repo
        .find_commit(target)
        .map_err(gix_err)?
        .tree_id()
        .map_err(gix_err)?
        .detach();
    let index = repo.index_from_tree(&tree_id).map_err(gix_err)?;
    let mut idx_out = std::fs::File::create(repo.git_dir().join("index"))?;
    index
        .write_to(&mut idx_out, gix::index::write::Options::default())
        .map_err(gix_err)?;

    let tree = repo
        .find_object(tree_id)
        .map_err(gix_err)?
        .peel_to_tree()
        .map_err(gix_err)?;
    let work_dir = repo
        .workdir()
        .ok_or_else(|| GitError::Gix("rebase: repository has no work tree".into()))?
        .to_path_buf();
    checkout_tree_into(repo, &tree, &work_dir)?;
    Ok(())
}

/// Write a linked worktree's admin files (`.git/worktrees/<name>/…`) + its
/// `.git` gitdir pointer at `path`, with `head_content` as its HEAD (a branch
/// ref or a detached oid), then check out `base`'s tree. The shared body of
/// both the attached and detached worktree-create paths.
fn materialize_worktree(
    repo: &gix::Repository,
    admin: &Path,
    name: &str,
    path: &Path,
    base: &gix::Commit<'_>,
    head_content: &str,
    branch: Option<String>,
) -> Result<WorktreeInfo> {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    // Like `git worktree add`: refuse a pre-existing NON-empty target so the
    // checkout never clobbers files already on disk. An empty dir is fine.
    if abs_path.exists() {
        let occupied = std::fs::read_dir(&abs_path)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true);
        if occupied {
            return Err(GitError::Gix(format!(
                "'{}' already exists and is not empty",
                abs_path.display()
            )));
        }
    }

    let tree_id = base.tree_id().map_err(gix_err)?.detach();

    // Admin files.
    std::fs::create_dir_all(admin)?;
    std::fs::write(admin.join("HEAD"), head_content)?;
    std::fs::write(admin.join("commondir"), "../..\n")?;
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", abs_path.join(".git").display()),
    )?;
    let idx = repo.index_from_tree(&tree_id).map_err(gix_err)?;
    let mut idx_out = std::fs::File::create(admin.join("index"))?;
    idx.write_to(&mut idx_out, gix::index::write::Options::default())
        .map_err(gix_err)?;

    // The worktree directory + its `.git` gitdir file, then the checkout.
    std::fs::create_dir_all(&abs_path)?;
    std::fs::write(
        abs_path.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )?;
    let tree = base.tree().map_err(gix_err)?;
    checkout_tree_into(repo, &tree, &abs_path)?;

    Ok(WorktreeInfo {
        name: name.to_string(),
        path: abs_path,
        branch,
        locked: false,
    })
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
        let mut iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_err)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(None)
            .map_err(gix_err)?;
        // Dirty as soon as the status iterator yields a single change.
        match iter.next() {
            Some(item) => {
                item.map_err(gix_err)?;
                Ok(false)
            }
            None => Ok(true),
        }
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        let repo = self.repo()?;
        let full = format!("refs/heads/{name}");
        Ok(repo.try_find_reference(&full).map_err(gix_err)?.is_some())
    }

    // ── Not yet built (later phases) ─────────────────────────────────────────

    fn create_worktree(
        &self,
        name: &str,
        path: &Path,
        opts: &CreateOptions,
    ) -> Result<WorktreeInfo> {
        // gix can't `git worktree add`, so build it: write the linked-worktree
        // admin files (.git/worktrees/<name>/{HEAD,commondir,gitdir,index}) + the
        // worktree's `.git` gitdir file, then check out the base commit's tree.
        let repo = self.repo()?;
        let admin = repo.common_dir().join("worktrees").join(name);
        if admin.exists() {
            return Err(GitError::WorktreeExists(name.to_string()));
        }
        // `git worktree add -b <new>` refuses if the new branch already exists.
        if let Some(nb) = &opts.new_branch {
            if self.branch_exists(nb)? {
                return Err(GitError::Gix(format!("a branch named '{nb}' already exists")));
            }
        }

        // Resolve the base commit (a reference, else HEAD).
        let base = match &opts.reference {
            Some(r) => repo
                .rev_parse_single(r.as_str())
                .map_err(gix_err)?
                .object()
                .map_err(gix_err)?
                .peel_to_commit()
                .map_err(gix_err)?,
            None => repo.head_commit().map_err(gix_err)?,
        };
        let base_id = base.id;

        // Decide the worktree's HEAD + which branch it attaches to (if any).
        let (head_content, branch) = if let Some(nb) = &opts.new_branch {
            self.create_branch(nb, &base_id.to_string())?;
            (format!("ref: refs/heads/{nb}\n"), Some(nb.clone()))
        } else if opts.reference.as_deref().is_some_and(|r| {
            self.branch_exists(r).unwrap_or(false)
        }) {
            let r = opts.reference.clone().unwrap();
            (format!("ref: refs/heads/{r}\n"), Some(r))
        } else {
            (format!("{base_id}\n"), None) // detached
        };

        materialize_worktree(&repo, &admin, name, path, &base, &head_content, branch)
    }

    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let repo = self.repo()?;
        let mut out = Vec::new();

        // The primary working tree isn't enumerated by `worktrees()`; include it
        // explicitly as "(main)" so callers see the complete picture.
        if let Some(workdir) = repo.workdir() {
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

    fn remove_worktree(&self, name: &str, _force: bool) -> Result<()> {
        // Remove the worktree directory and prune its admin entry. (The engine
        // always removes with force; a dirty-tree refusal isn't modeled.)
        let repo = self.repo()?;
        let admin = repo.common_dir().join("worktrees").join(name);
        if !admin.exists() {
            return Err(GitError::WorktreeNotFound(name.to_string()));
        }
        // `gitdir` admin file points at `<worktree>/.git`; its parent is the tree.
        if let Ok(gitdir) = std::fs::read_to_string(admin.join("gitdir")) {
            if let Some(wt_dir) = std::path::PathBuf::from(gitdir.trim()).parent() {
                let _ = std::fs::remove_dir_all(wt_dir);
            }
        }
        std::fs::remove_dir_all(&admin)?;
        Ok(())
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

    fn merge_no_commit(&self, worktree_path: &Path, source_ref: &str) -> Result<MergeOutcome> {
        // `--no-ff --no-commit`: three-way merge `source_ref` into the worktree's
        // branch, leaving the result staged (conflict stages in the index,
        // conflict markers in the worktree) with MERGE_HEAD set, but NOT
        // committed. The caller re-scans conflicts via `unresolved_paths`.
        use gix::merge::tree::TreatAsUnresolved;
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let ours = repo.head_commit().map_err(gix_err)?;
        let ours_id = ours.id;
        let theirs = repo
            .rev_parse_single(source_ref)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?;
        let theirs_id = theirs.id;

        // Already up to date: `theirs` is an ancestor of `ours` (the merge base
        // IS theirs) → nothing to merge, no MERGE_HEAD.
        let base_id = repo.merge_base(ours_id, theirs_id).map_err(gix_err)?.detach();
        if base_id == theirs_id {
            return Ok(MergeOutcome {
                ok: true,
                performed: false,
                conflict_paths: Vec::new(),
                message: None,
            });
        }

        let ours_tree = ours.tree_id().map_err(gix_err)?.detach();
        let theirs_tree = theirs.tree_id().map_err(gix_err)?.detach();
        let base_tree = repo
            .find_commit(base_id)
            .map_err(gix_err)?
            .tree_id()
            .map_err(gix_err)?
            .detach();

        let options = repo.tree_merge_options().map_err(gix_err)?;
        let labels = gix::merge::blob::builtin_driver::text::Labels::default();
        let mut outcome = repo
            .merge_trees(base_tree, ours_tree, theirs_tree, labels, options)
            .map_err(gix_err)?;
        let how = TreatAsUnresolved::git();
        let has_conflicts = outcome.has_unresolved_conflicts(how);
        let merged_tree = outcome.tree.write().map_err(gix_err)?.detach();

        // Worktree index = merged tree + conflict stages applied.
        let mut index = repo.index_from_tree(&merged_tree).map_err(gix_err)?;
        outcome.index_changed_after_applying_conflicts(
            &mut index,
            how,
            gix::merge::tree::apply_index_entries::RemovalMode::Mark,
        );
        let mut idx_out = std::fs::File::create(repo.git_dir().join("index"))?;
        index
            .write_to(&mut idx_out, gix::index::write::Options::default())
            .map_err(gix_err)?;

        // Worktree files = merged tree (conflict-marked blobs for conflicts).
        let merged = repo
            .find_object(merged_tree)
            .map_err(gix_err)?
            .peel_to_tree()
            .map_err(gix_err)?;
        checkout_tree_into(&repo, &merged, worktree_path)?;

        // MERGE_HEAD records the in-progress merge for the caller.
        std::fs::write(repo.git_dir().join("MERGE_HEAD"), format!("{theirs_id}\n"))?;

        Ok(MergeOutcome {
            ok: !has_conflicts,
            performed: true,
            conflict_paths: Vec::new(),
            message: None,
        })
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

    fn add_all_under(&self, worktree_path: &Path, prefix: &str) -> Result<()> {
        // Status-driven `git add -A -- <prefix>`: walk the SAME status iterator
        // `is_clean` uses (untracked included, gitignore respected), filter to
        // the prefix, then stage what exists on disk and drop what was deleted.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_err)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(None)
            .map_err(gix_err)?;
        let prefix = prefix.trim_matches('/');
        let dir_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };
        let mut add: Vec<String> = Vec::new();
        let mut del: Vec<Vec<u8>> = Vec::new();
        for item in iter {
            let item = item.map_err(gix_err)?;
            let loc = item.location().to_string();
            if !prefix.is_empty() && loc != prefix && !loc.starts_with(&dir_prefix) {
                continue;
            }
            if std::fs::symlink_metadata(worktree_path.join(&loc)).is_ok() {
                add.push(loc);
            } else {
                del.push(loc.into_bytes());
            }
        }
        self.add_paths(worktree_path, &add)?;
        if !del.is_empty() {
            let mut index = repo.open_index().map_err(gix_err)?;
            let gone: std::collections::HashSet<&[u8]> =
                del.iter().map(|p| p.as_slice()).collect();
            index.remove_entries(|_, p, _| gone.contains(p.as_ref() as &[u8]));
            index
                .write(gix::index::write::Options::default())
                .map_err(gix_err)?;
        }
        Ok(())
    }

    fn status_dirty_under(&self, worktree_path: &Path, prefix: &str) -> Result<bool> {
        // First status item under `prefix` → dirty. Same iterator as
        // `add_all_under`, stopped at the first hit.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_err)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(None)
            .map_err(gix_err)?;
        let prefix = prefix.trim_matches('/');
        let dir_prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };
        for item in iter {
            let item = item.map_err(gix_err)?;
            let loc = item.location().to_string();
            if prefix.is_empty() || loc == prefix || loc.starts_with(&dir_prefix) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn dirty_paths_excluding(
        &self,
        worktree_path: &Path,
        exclude_prefixes: &[&str],
    ) -> Result<Vec<String>> {
        // Same status iterator as `add_all_under`, filtered the other way:
        // collect everything that is NOT engine bookkeeping.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(gix_err)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(None)
            .map_err(gix_err)?;
        let excludes: Vec<(String, String)> = exclude_prefixes
            .iter()
            .map(|p| {
                let p = p.trim_matches('/').to_string();
                let dir = format!("{p}/");
                (p, dir)
            })
            .collect();
        let mut out: Vec<String> = Vec::new();
        for item in iter {
            let item = item.map_err(gix_err)?;
            let loc = item.location().to_string();
            let excluded = excludes
                .iter()
                .any(|(p, dir)| !p.is_empty() && (loc == *p || loc.starts_with(dir.as_str())));
            if !excluded {
                out.push(loc);
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    fn changed_paths_between(&self, base_ref: &str, head_ref: &str) -> Result<Vec<String>> {
        // `git diff --name-only base...head`: diff head's tree against the
        // merge-base tree, so only the child branch's OWN work is named.
        let repo = self.repo()?;
        let base_id = repo
            .rev_parse_single(base_ref)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        let head_id = repo
            .rev_parse_single(head_ref)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        let merge_base = repo.merge_base(base_id, head_id).map_err(gix_err)?;
        crate::diff::changed_paths_between_trees(&repo, merge_base.detach(), head_id)
    }

    fn checkout_branch(&self, branch: &str) -> Result<()> {
        // `git checkout <branch>` for the MAIN tree, on a clean working tree
        // (the caller's contract): remove files tracked in the current HEAD but
        // absent in the target, materialize the target tree, rebuild the index
        // from it, and flip HEAD to a symbolic ref of the branch.
        let repo = self.repo()?;
        let root = repo
            .workdir()
            .ok_or_else(|| GitError::Gix("checkout: repository has no work tree".into()))?
            .to_path_buf();
        let target = repo
            .rev_parse_single(format!("refs/heads/{branch}").as_str())
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?;
        let target_tree = target.tree().map_err(gix_err)?;

        // (1) delete tracked-in-old, absent-in-new files (then prune any dirs
        // those deletions emptied — git does the same).
        let mut new_paths = Vec::new();
        collect_tree_paths(&repo, &target_tree, "", &mut new_paths)?;
        let keep: std::collections::HashSet<&str> =
            new_paths.iter().map(String::as_str).collect();
        let mut old_paths = Vec::new();
        if let Ok(head) = repo.head_commit() {
            let old_tree = head.tree().map_err(gix_err)?;
            collect_tree_paths(&repo, &old_tree, "", &mut old_paths)?;
        }
        for p in &old_paths {
            if !keep.contains(p.as_str()) {
                let abs = root.join(p);
                let _ = std::fs::remove_file(&abs);
                let mut dir = abs.parent().map(Path::to_path_buf);
                while let Some(d) = dir {
                    if d == root || std::fs::remove_dir(&d).is_err() {
                        break;
                    }
                    dir = d.parent().map(Path::to_path_buf);
                }
            }
        }

        // (2) materialize the target tree + (3) the index from it.
        checkout_tree_into(&repo, &target_tree, &root)?;
        let tree_id = target_tree.id().detach();
        let index = repo.index_from_tree(&tree_id).map_err(gix_err)?;
        let mut idx_out = std::fs::File::create(repo.git_dir().join("index"))?;
        index
            .write_to(&mut idx_out, gix::index::write::Options::default())
            .map_err(gix_err)?;

        // (4) HEAD -> symbolic ref of the branch.
        std::fs::write(
            repo.git_dir().join("HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )?;
        Ok(())
    }

    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()> {
        // gix has commit()/write_blob() but no `git write-tree`, so build the
        // tree from the staged index ourselves, then commit it on HEAD with the
        // current HEAD as parent (no parent on an unborn branch).
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let index = repo.index_or_empty().map_err(gix_err)?;
        let tree_id = write_index_tree(&repo, &index)?;
        let mut parents: Vec<gix::ObjectId> =
            repo.head_commit().ok().map(|c| c.id).into_iter().collect();
        // Merge-aware (like `git commit` after a merge): a present MERGE_HEAD is
        // the second parent, making this a real merge commit. Cleared after.
        let merge_head = repo.git_dir().join("MERGE_HEAD");
        if let Ok(raw) = std::fs::read_to_string(&merge_head) {
            if let Ok(id) = gix::ObjectId::from_hex(raw.trim().as_bytes()) {
                parents.push(id);
            }
        }
        repo.commit("HEAD", message, tree_id, parents)
            .map_err(gix_err)?;
        let _ = std::fs::remove_file(&merge_head);
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

    fn push(&self, worktree_path: &Path, branch: &str) -> Result<()> {
        // Pure-Rust send-pack: resolve origin's push URL + the worktree's HEAD,
        // then drive the receive-pack exchange in `push::send_pack`. HTTPS auth
        // (a token) is read from the environment the engine already exports for
        // its shell pushes, so both backends authenticate the same way.
        // Bounded: the whole exchange runs under the network deadline so an
        // unresponsive remote can never wedge a tick (the predecessor's #333).
        let wt = worktree_path.to_path_buf();
        let branch = branch.to_string();
        crate::net::with_deadline("push", move || {
            let repo = gix::open(&wt).map_err(gix_err)?;
            let new_oid = repo.head_commit().map_err(gix_err)?.id;
            let url = repo
                .find_remote("origin")
                .map_err(gix_err)?
                .url(gix::remote::Direction::Push)
                .ok_or_else(|| GitError::Gix("origin has no push URL".into()))?
                .to_owned();
            let account = crate::push::credentials_for(&url);
            crate::push::send_pack(&repo, url, account, &branch, new_oid)
        })
    }

    fn fetch(&self, worktree_path: &Path, branch: &str) -> Result<()> {
        // `git fetch origin <branch>` — pure-Rust transport (local/file + rustls
        // HTTPS). Under `blocking-network-client`, gix's async fetch fns are
        // maybe_async-stripped to blocking, so there's no `.await`.
        // Bounded under the network deadline, same as push.
        let wt = worktree_path.to_path_buf();
        let branch = branch.to_string();
        crate::net::with_deadline("fetch", move || {
            let repo = gix::open(&wt).map_err(gix_err)?;
            let remote = repo
                .find_remote("origin")
                .map_err(gix_err)?
                .with_refspecs(
                    Some(format!("+refs/heads/{branch}:refs/remotes/origin/{branch}").as_str()),
                    gix::remote::Direction::Fetch,
                )
                .map_err(gix_err)?;
            remote
                .connect(gix::remote::Direction::Fetch)
                .map_err(gix_err)?
                .prepare_fetch(gix::progress::Discard, Default::default())
                .map_err(gix_err)?
                .receive(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .map_err(gix_err)?;
            Ok(())
        })
    }

    fn rebase_onto(&self, worktree_path: &Path, upstream: &str) -> Result<()> {
        // `git rebase <upstream>`: replay `upstream..HEAD` onto `upstream`, then
        // move the current branch to the new tip. gix has no rebase, so build it
        // as a cherry-pick loop over the three-way merge primitive — each commit
        // re-applied with `merge_trees(parent, moving-base, commit)`.
        use gix::merge::tree::TreatAsUnresolved;
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let ours_id = repo.head_commit().map_err(gix_err)?.id;
        let upstream_id = repo
            .rev_parse_single(upstream)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;

        // The ref the worktree HEAD follows — moved once every pick lands clean.
        let branch = head_branch_of(&repo);
        // ORIG_HEAD lets a follow-up rebase_abort restore us (like `git rebase`).
        let git_dir = repo.git_dir().to_path_buf();
        std::fs::write(git_dir.join("ORIG_HEAD"), format!("{ours_id}\n"))?;

        // Trivial cases off the merge base.
        let base = repo
            .merge_base(ours_id, upstream_id)
            .map_err(gix_err)?
            .detach();
        if base == upstream_id {
            // Upstream is already an ancestor of HEAD → nothing to replay.
            return Ok(());
        }
        if base == ours_id {
            // HEAD is an ancestor of upstream → fast-forward onto upstream.
            move_branch_and_checkout(&repo, branch.as_deref(), upstream_id)?;
            return Ok(());
        }

        // Commits to replay: `upstream..HEAD`, reordered oldest-first.
        let mut todo: Vec<gix::ObjectId> = repo
            .rev_walk([ours_id])
            .with_hidden([upstream_id])
            .all()
            .map_err(gix_err)?
            .map(|info| info.map(|i| i.id).map_err(gix_err))
            .collect::<Result<Vec<_>>>()?;
        todo.reverse();

        // Cherry-pick each onto the moving base (starting at upstream). Commit
        // objects are built in memory; the branch only moves once every pick is
        // clean, so a conflict leaves the worktree untouched for a clean abort.
        let options = repo.tree_merge_options().map_err(gix_err)?;
        let how = TreatAsUnresolved::git();
        let mut base_id = upstream_id;
        for cid in todo {
            let commit = repo.find_commit(cid).map_err(gix_err)?;
            let commit_tree = commit.tree_id().map_err(gix_err)?.detach();
            // The pick's ancestor is the commit's own first parent; `merge_trees`
            // then re-applies the parent→commit diff onto the moving base.
            let parent_tree = match commit.parent_ids().next() {
                Some(p) => repo
                    .find_commit(p.detach())
                    .map_err(gix_err)?
                    .tree_id()
                    .map_err(gix_err)?
                    .detach(),
                None => repo.empty_tree().id().detach(),
            };
            let base_tree = repo
                .find_commit(base_id)
                .map_err(gix_err)?
                .tree_id()
                .map_err(gix_err)?
                .detach();
            let labels = gix::merge::blob::builtin_driver::text::Labels::default();
            let mut outcome = repo
                .merge_trees(parent_tree, base_tree, commit_tree, labels, options.clone())
                .map_err(gix_err)?;
            if outcome.has_unresolved_conflicts(how) {
                // Stop-on-conflict like `git rebase`: drop a rebase-in-progress
                // marker so rebase_abort restores ORIG_HEAD. Nothing moved yet.
                std::fs::create_dir_all(git_dir.join("rebase-merge")).ok();
                return Err(GitError::Gix(format!(
                    "rebase conflict replaying {cid} onto {base_id}"
                )));
            }
            let merged_tree = outcome.tree.write().map_err(gix_err)?.detach();
            // Preserve the original author/committer/message; only the parent
            // changes — that's exactly what re-parents the commit (new hash).
            let author = commit
                .author()
                .map_err(gix_err)?
                .to_owned()
                .map_err(gix_err)?;
            let committer = commit
                .committer()
                .map_err(gix_err)?
                .to_owned()
                .map_err(gix_err)?;
            let message = commit.message_raw().map_err(gix_err)?.to_owned();
            let new_commit = gix::objs::Commit {
                tree: merged_tree,
                parents: std::iter::once(base_id).collect(),
                author,
                committer,
                encoding: None,
                message,
                extra_headers: Vec::new(),
            };
            base_id = repo.write_object(&new_commit).map_err(gix_err)?.detach();
        }

        move_branch_and_checkout(&repo, branch.as_deref(), base_id)?;
        Ok(())
    }

    fn rebase_abort(&self, worktree_path: &Path) -> Result<()> {
        // Best-effort, like `git rebase --abort`: if a rebase is in flight (our
        // marker present), restore the branch + worktree to ORIG_HEAD; with
        // nothing in flight it's a no-op.
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let git_dir = repo.git_dir().to_path_buf();
        let marker = git_dir.join("rebase-merge");
        if !marker.exists() {
            return Ok(());
        }
        if let Ok(raw) = std::fs::read_to_string(git_dir.join("ORIG_HEAD")) {
            if let Ok(orig) = gix::ObjectId::from_hex(raw.trim().as_bytes()) {
                let branch = head_branch_of(&repo);
                move_branch_and_checkout(&repo, branch.as_deref(), orig)?;
            }
        }
        let _ = std::fs::remove_dir_all(&marker);
        Ok(())
    }

    fn create_worktree_detached(
        &self,
        name: &str,
        path: &Path,
        committish: &str,
    ) -> Result<WorktreeInfo> {
        // Always-detached worktree at `committish`'s commit — even when it names
        // a branch — so it works while that branch is checked out elsewhere.
        let repo = self.repo()?;
        let admin = repo.common_dir().join("worktrees").join(name);
        if admin.exists() {
            return Err(GitError::WorktreeExists(name.to_string()));
        }
        let base = repo
            .rev_parse_single(committish)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?;
        let head_content = format!("{}\n", base.id);
        materialize_worktree(&repo, &admin, name, path, &base, &head_content, None)
    }

    fn head_oid(&self, worktree_path: &Path) -> Result<String> {
        let repo = gix::open(worktree_path).map_err(gix_err)?;
        let oid = repo.head_commit().map_err(gix_err)?.id;
        Ok(oid.to_string())
    }

    fn set_branch_to(&self, name: &str, committish: &str) -> Result<()> {
        // `git branch -f <name> <committish>`: force-create-or-update the ref.
        let repo = self.repo()?;
        let target = repo
            .rev_parse_single(committish)
            .map_err(gix_err)?
            .object()
            .map_err(gix_err)?
            .peel_to_commit()
            .map_err(gix_err)?
            .id;
        repo.reference(
            format!("refs/heads/{name}"),
            target,
            gix::refs::transaction::PreviousValue::Any,
            format!("branch: reset to {committish}"),
        )
        .map_err(gix_err)?;
        Ok(())
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        // `git branch -D <name>`: delete the ref; absent is a no-op.
        let repo = self.repo()?;
        match repo.find_reference(&format!("refs/heads/{name}")) {
            Ok(r) => {
                r.delete().map_err(gix_err)?;
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }

    fn remote_url(&self, name: &str) -> Result<Option<String>> {
        let repo = self.repo()?;
        Ok(repo.find_remote(name).ok().and_then(|r| {
            r.url(gix::remote::Direction::Fetch)
                .map(|u| u.to_bstring().to_string())
        }))
    }

    fn default_branch(&self) -> Result<Option<String>> {
        // `git symbolic-ref refs/remotes/origin/HEAD` → the short branch name.
        let repo = self.repo()?;
        let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") else {
            return Ok(None);
        };
        // A symbolic ref points at refs/remotes/origin/<branch>; take the leaf.
        Ok(match reference.target() {
            gix::refs::TargetRef::Symbolic(name) => name
                .shorten()
                .to_string()
                .rsplit('/')
                .next()
                .map(str::to_string),
            gix::refs::TargetRef::Object(_) => None,
        })
    }

    fn diff_stat(&self, reference: &str) -> Result<String> {
        let repo = self.repo()?;
        crate::diff::diff_worktree_against(&repo, reference, crate::diff::Format::Stat)
    }

    fn diff(&self, reference: &str) -> Result<String> {
        let repo = self.repo()?;
        crate::diff::diff_worktree_against(&repo, reference, crate::diff::Format::Unified)
    }
}
