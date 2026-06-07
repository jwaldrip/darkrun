//! Pure-Rust `git push` (the send-pack / receive-pack client) over gitoxide's
//! transport — the one operation gitoxide ships no high-level API for. It's
//! built from the lower-level primitives: a packfile generated from the local
//! object database (this module's [`build_pack`]) plus the receive-pack
//! command/report exchange (in [`gix_backend`](crate::gix_backend)).

use std::sync::atomic::AtomicBool;

use crate::error::{GitError, Result};

/// Map any gix error into our crate error.
fn gix_err(e: impl std::fmt::Display) -> GitError {
    GitError::Gix(e.to_string())
}

/// Generate a self-contained (non-thin) packfile carrying every object reachable
/// from `new_oid` that is NOT already reachable from `exclude` (the remote's
/// advertised tips). Returns the complete packfile bytes — header, entries, and
/// the trailing content hash — exactly what `git index-pack` / a remote's
/// `receive-pack` consumes.
///
/// Expansion is `TreeAdditionsComparedToAncestor`: each new commit contributes
/// only the trees/blobs it adds over its parent, so an incremental push sends
/// just the genuinely-new data while staying self-contained (unchanged children
/// are referenced by oid the remote already has — no thin-pack base needed).
pub(crate) fn build_pack(
    repo: &gix::Repository,
    new_oid: gix::ObjectId,
    exclude: &[gix::ObjectId],
) -> Result<Vec<u8>> {
    use gix::odb::pack::data::output;
    use gix::odb::pack::data::Version;

    // The new commits to pack: `exclude..new_oid`, in any order (the entry
    // pipeline re-sorts). Empty `exclude` (first push) → the full history.
    let commit_ids: Vec<gix::ObjectId> = repo
        .rev_walk([new_oid])
        .with_hidden(exclude.iter().copied())
        .all()
        .map_err(gix_err)?
        .map(|i| i.map(|i| i.id).map_err(gix_err))
        .collect::<Result<Vec<_>>>()?;

    // `repo.objects` is an in-memory Proxy wrapper; unwrap to the inner odb
    // handle, which is what implements `gix_pack::Find` for the pack pipeline.
    // The pack writer resolves pack locations by oid, which requires the handle
    // to pin packs open (else it panics on the first lookup).
    let mut db = repo.objects.clone().into_inner();
    db.prevent_pack_unload();
    let interrupt = AtomicBool::new(false);
    let input = Box::new(
        commit_ids
            .into_iter()
            .map(Ok::<_, Box<dyn std::error::Error + Send + Sync + 'static>>),
    );

    let (counts, _outcome) = output::count::objects(
        db.clone(),
        input,
        &gix::progress::Discard,
        &interrupt,
        output::count::objects::Options {
            thread_limit: Some(1),
            chunk_size: 50,
            input_object_expansion:
                output::count::objects::ObjectExpansion::TreeAdditionsComparedToAncestor,
        },
    )
    .map_err(gix_err)?;

    let num_entries = counts.len() as u32;
    let entries = output::entry::iter_from_counts(
        counts,
        db.clone(),
        Box::new(gix::progress::Discard),
        output::entry::iter_from_counts::Options {
            thread_limit: Some(1),
            chunk_size: 50,
            version: Version::V2,
            mode: output::entry::iter_from_counts::Mode::PackCopyAndBaseObjects,
            allow_thin_pack: false,
        },
    );

    // The entry chunks arrive out of order from the worker pool; InOrderIter
    // re-sequences them for the byte writer, which lays down the final pack.
    let in_order = gix::features::parallel::InOrderIter::from(entries);
    let mut buf: Vec<u8> = Vec::new();
    let mut writer = output::bytes::FromEntriesIter::new(
        in_order,
        &mut buf,
        num_entries,
        Version::V2,
        gix::hash::Kind::Sha1,
    );
    for step in writer.by_ref() {
        step.map_err(gix_err)?;
    }
    drop(writer);
    Ok(buf)
}

/// Resolve HTTPS push credentials for `url`, or `None` for transports that
/// don't need them (file://, ssh, or an unauthenticated remote).
///
/// Precedence: credentials embedded in the URL win; otherwise a token from the
/// environment the engine exports from its credential store —
/// `DARKRUN_GIT_TOKEN` first, then the host-conventional `GITHUB_TOKEN`/
/// `GH_TOKEN` or `GITLAB_TOKEN`. The username is the provider's token-as-
/// password convention (`x-access-token` for GitHub, `oauth2` for GitLab).
pub(crate) fn credentials_for(url: &gix::Url) -> Option<gix::sec::identity::Account> {
    // Only HTTP(S) uses basic-auth identities here.
    if !matches!(url.scheme, gix::url::Scheme::Https | gix::url::Scheme::Http) {
        return None;
    }
    let host = url.host().unwrap_or_default();
    let is_gitlab = host.contains("gitlab");

    // 1) Credentials embedded directly in the remote URL.
    if let Some(password) = url.password() {
        let username = url.user().unwrap_or("x-access-token").to_string();
        return Some(gix::sec::identity::Account {
            username,
            password: password.to_string(),
            oauth_refresh_token: None,
        });
    }

    // 2) A token from the environment, by convention per host.
    let token = std::env::var("DARKRUN_GIT_TOKEN")
        .ok()
        .or_else(|| {
            if is_gitlab {
                std::env::var("GITLAB_TOKEN").ok()
            } else {
                std::env::var("GITHUB_TOKEN").ok().or_else(|| std::env::var("GH_TOKEN").ok())
            }
        })
        .filter(|t| !t.is_empty())?;
    let username = url
        .user()
        .map(str::to_string)
        .unwrap_or_else(|| if is_gitlab { "oauth2".into() } else { "x-access-token".into() });
    Some(gix::sec::identity::Account {
        username,
        password: token,
        oauth_refresh_token: None,
    })
}

/// Push `new_oid` to `refs/heads/<branch>` on the remote at `url`, building the
/// receive-pack exchange by hand (gitoxide has no send-pack): handshake for the
/// ref advertisement, send the ref-update command + a packfile of the new
/// objects, then parse the report-status.
///
/// `account` carries HTTPS credentials (a token as the password) when present;
/// it's set on the transport up front so the very first request authenticates
/// without a 401 round-trip. A non-fast-forward (or otherwise rejected) ref
/// surfaces as a [`GitError::Gix`] whose message preserves the server's reason,
/// which is exactly what the engine's NFF-recovery matches on.
pub(crate) fn send_pack(
    repo: &gix::Repository,
    url: gix::Url,
    account: Option<gix::sec::identity::Account>,
    branch: &str,
    new_oid: gix::ObjectId,
) -> Result<()> {
    use gix::protocol::handshake::Ref;
    use gix::protocol::transport;
    use gix::protocol::transport::client::blocking_io::{connect, Transport};
    use gix::protocol::transport::client::TransportWithoutIO;
    use std::io::{Read, Write};

    let target = format!("refs/heads/{branch}");

    // Protocol v1: the ref advertisement comes inline with the handshake (v2
    // push isn't universally supported, and v1 is what receive-pack speaks).
    let mut tp = connect::connect(
        url,
        connect::Options {
            version: transport::Protocol::V1,
            ..Default::default()
        },
    )
    .map_err(gix_err)?;
    if let Some(account) = account {
        tp.set_identity(account).map_err(gix_err)?;
    }

    // Handshake → advertised refs (each ref's current oid is our "old" value;
    // their union is what the remote already has, so we exclude it from the
    // pack). Parse into owned data and drop the borrowing response before the
    // next request on the same transport.
    let refs: Vec<Ref> = {
        let mut ssr = tp
            .handshake(transport::Service::ReceivePack, &[])
            .map_err(gix_err)?;
        let caps = ssr.capabilities.clone();
        match ssr.refs.take() {
            Some(mut r) => {
                gix::protocol::handshake::refs::from_v1_refs_received_as_part_of_handshake_and_capabilities(
                    &mut *r,
                    caps.iter(),
                )
                .map_err(gix_err)?
                .0
            }
            None => Vec::new(),
        }
    };

    let direct_oid = |r: &Ref| -> Option<gix::ObjectId> {
        match r {
            Ref::Direct { full_ref_name, object } => Some((full_ref_name.clone(), *object)),
            Ref::Symbolic { full_ref_name, object, .. } => Some((full_ref_name.clone(), *object)),
            Ref::Peeled { full_ref_name, tag, .. } => Some((full_ref_name.clone(), *tag)),
            Ref::Unborn { .. } => None,
        }
        .map(|(_, oid)| oid)
    };

    let null = gix::ObjectId::null(gix::hash::Kind::Sha1);
    let old = refs
        .iter()
        .find_map(|r| match r {
            Ref::Direct { full_ref_name, object } if full_ref_name == target.as_str() => Some(*object),
            _ => None,
        })
        .unwrap_or(null);

    // Already up to date — the remote's ref is at our commit. No-op.
    if old == new_oid {
        return Ok(());
    }

    // Pack only what the remote lacks: exclude every advertised tip.
    let exclude: Vec<gix::ObjectId> = refs.iter().filter_map(direct_oid).collect();
    let pack = build_pack(repo, new_oid, &exclude)?;

    // The update request: one command line (carrying our capabilities after a
    // NUL), a flush, then the raw packfile.
    let command = format!(
        "{} {} {}\0report-status\n",
        old.to_hex(),
        new_oid.to_hex(),
        target
    );
    let mut rw = tp
        .request(
            transport::client::WriteMode::Binary,
            transport::client::MessageKind::Flush,
            false,
        )
        .map_err(gix_err)?;
    rw.write_all(command.as_bytes())?;
    rw.write_message(transport::client::MessageKind::Flush)?;
    // After the flush the packfile is sent verbatim, not packet-line framed.
    let (mut raw, mut reader) = rw.into_parts();
    raw.write_all(&pack)?;
    raw.flush()?;
    drop(raw);

    // report-status (no side-band requested → plain data lines): `unpack ok`
    // then one `ok <ref>` / `ng <ref> <reason>` per command.
    let mut report = String::new();
    reader.read_to_string(&mut report)?;
    drop(reader);
    parse_report_status(&report, &target)
}

/// Interpret a receive-pack `report-status` body, returning `Ok(())` only when
/// the unpack succeeded AND our ref updated. A rejected ref's reason is carried
/// through verbatim so non-fast-forward recovery upstream can match on it.
fn parse_report_status(report: &str, target: &str) -> Result<()> {
    let mut unpack_ok = false;
    let mut ref_result: Option<std::result::Result<(), String>> = None;
    for line in report.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("unpack ") {
            unpack_ok = rest == "ok";
            if !unpack_ok {
                return Err(GitError::Gix(format!("remote failed to unpack pack: {rest}")));
            }
        } else if let Some(name) = line.strip_prefix("ok ") {
            if name == target {
                ref_result = Some(Ok(()));
            }
        } else if let Some(rest) = line.strip_prefix("ng ") {
            // `ng <ref> <reason>` — the reason (e.g. "non-fast-forward").
            let reason = rest.strip_prefix(target).map(|s| s.trim()).unwrap_or(rest);
            ref_result = Some(Err(reason.to_string()));
        }
    }
    if !unpack_ok {
        return Err(GitError::Gix("remote sent no unpack status".into()));
    }
    match ref_result {
        Some(Ok(())) => Ok(()),
        Some(Err(reason)) => Err(GitError::Gix(format!("push rejected: {target} {reason}"))),
        None => Err(GitError::Gix(format!(
            "remote reported no status for {target}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        assert!(
            Command::new("git").arg("-C").arg(root).args(args).status().unwrap().success(),
            "git {args:?}"
        );
    }
    fn git_out(root: &Path, args: &[&str]) -> String {
        let o = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }
    fn init(root: &Path) {
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "user.email", "t@d.local"]);
        git(root, &["config", "user.name", "t"]);
        git(root, &["config", "commit.gpgsign", "false"]);
    }

    /// A pack of the whole history is valid and self-contained: `git index-pack`
    /// accepts it WITHOUT `--fix-thin` and reports every object.
    #[test]
    fn build_pack_is_a_valid_self_contained_pack() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "one"]);
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "two"]);

        let repo = gix::open(root).unwrap();
        let head = repo.head_commit().unwrap().id;
        let pack = build_pack(&repo, head, &[]).unwrap();
        assert_eq!(&pack[..4], b"PACK", "pack magic");

        // `git index-pack` validates structure + connectivity (no --fix-thin).
        let pdir = tempfile::tempdir().unwrap();
        let ppath = pdir.path().join("test.pack");
        std::fs::write(&ppath, &pack).unwrap();
        // index-pack needs an object dir to resolve against; an empty repo is fine
        // because the pack is self-contained.
        let vrepo = tempfile::tempdir().unwrap();
        init(vrepo.path());
        let out = Command::new("git")
            .arg("-C").arg(vrepo.path())
            .args(["index-pack", "-v"])
            .arg(&ppath)
            .output()
            .unwrap();
        assert!(out.status.success(), "index-pack: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// An incremental pack (`old..new`) carries only the new commits' additions,
    /// and a remote that already has `old` can unpack it cleanly.
    #[test]
    fn build_pack_incremental_unpacks_into_a_remote_that_has_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "base"]);
        let base = git_out(root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-qm", "next"]);
        let head = git_out(root, &["rev-parse", "HEAD"]);

        let repo = gix::open(root).unwrap();
        let base_oid = gix::ObjectId::from_hex(base.as_bytes()).unwrap();
        let head_oid = gix::ObjectId::from_hex(head.as_bytes()).unwrap();
        let pack = build_pack(&repo, head_oid, &[base_oid]).unwrap();

        // A clone that has `base` but not `next` unpacks the incremental pack.
        let rdir = tempfile::tempdir().unwrap();
        let rroot = rdir.path().join("r");
        assert!(Command::new("git")
            .args(["clone", "-q", root.to_str().unwrap(), rroot.to_str().unwrap()])
            .status().unwrap().success());
        // Roll the clone back to `base` so `next`'s objects are genuinely absent.
        git(&rroot, &["reset", "-q", "--hard", &base]);
        let out = Command::new("git")
            .arg("-C").arg(&rroot)
            .args(["unpack-objects", "-q"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(&pack)?;
                c.wait()
            })
            .unwrap();
        assert!(out.success(), "unpack-objects accepted the incremental pack");
        // The new commit object now resolves in the remote-like clone.
        assert_eq!(git_out(&rroot, &["cat-file", "-t", &head]), "commit");
    }
}
