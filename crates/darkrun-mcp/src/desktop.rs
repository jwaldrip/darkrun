//! Launching the darkrun desktop app — the only interactive surface the engine
//! brings up (never a browser). `darkrun_show` calls [`spawn`] to open the app
//! pointed at the running engine, when none is already connected.
//!
//! Resolution mirrors how the `bin/darkrun` shim resolves the CLI:
//! - **Dev checkout** (the engine is running from a cargo workspace's
//!   `target/<profile>/`): always use the local `target/<profile>/darkrun-desktop`,
//!   **building it for the host arch on demand** if it isn't built yet. So a dev
//!   build of the engine always drives a matching local desktop build.
//! - **Installed plugin**: the per-arch sub-package ships `darkrun-desktop` next
//!   to `darkrun`, so it's a sibling of the running engine binary.
//! - `DARKRUN_DESKTOP` overrides everything.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The desktop binary name for this platform.
fn bin_name() -> &'static str {
    if cfg!(windows) {
        "darkrun-desktop.exe"
    } else {
        "darkrun-desktop"
    }
}

/// If `exe` lives in a cargo workspace's `target/<profile>/`, return
/// `(workspace_root, profile)` — the dev-checkout signal. Recognizes the darkrun
/// workspace by its `desktop/` crate. Pure over `exe` so it's testable.
fn dev_workspace_from(exe: &Path) -> Option<(PathBuf, String)> {
    let profile_dir = exe.parent()?; // <ws>/target/<profile>
    let profile = profile_dir.file_name()?.to_str()?.to_string();
    if profile != "debug" && profile != "release" {
        return None;
    }
    let target = profile_dir.parent()?; // <ws>/target
    if target.file_name()?.to_str()? != "target" {
        return None;
    }
    let ws = target.parent()?.to_path_buf(); // <ws>
    let is_darkrun_ws =
        ws.join("Cargo.toml").is_file() && ws.join("desktop").join("Cargo.toml").is_file();
    is_darkrun_ws.then_some((ws, profile))
}

/// The dev workspace + profile the running engine was built in, if any.
fn dev_workspace() -> Option<(PathBuf, String)> {
    dev_workspace_from(&std::env::current_exe().ok()?)
}

/// The outcome of [`spawn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// The app was launched; carries the binary path.
    Launched(PathBuf),
    /// A dev build is in flight; the app will open when `cargo build` finishes.
    Building,
    /// No desktop binary could be resolved or built.
    NotFound,
}

/// Single-quote a path for a POSIX shell command.
#[cfg(not(windows))]
fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Spawn a **detached** `cargo build -p darkrun-desktop && <bin>` so the build
/// runs in the background and the app launches itself when it completes — the
/// `show` call doesn't block on the (one-time) compile. Returns whether the
/// builder process spawned.
fn spawn_build_then_launch(ws: &Path, profile: &str, bin: &Path, port: u16) -> bool {
    let rel = if profile == "release" { " --release" } else { "" };
    let mut cmd;
    #[cfg(windows)]
    {
        let script = format!(
            "cargo build -p darkrun-desktop{rel} && \"{}\"",
            bin.display()
        );
        cmd = Command::new("cmd");
        cmd.arg("/C").arg(script);
    }
    #[cfg(not(windows))]
    {
        let script = format!(
            "cargo build -p darkrun-desktop{rel} && exec {}",
            sh_quote(bin)
        );
        cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
    }
    cmd.current_dir(ws)
        .env("DARKRUN_PORT", port.to_string())
        .env_remove("DARKRUN_SESSION_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Locate the `darkrun-desktop` binary WITHOUT building — an explicit
/// `DARKRUN_DESKTOP` path, a sibling of the running engine binary (installed
/// plugin), then the project's `target/{release,debug}`. `None` when not found.
pub fn find(repo_root: &Path) -> Option<PathBuf> {
    let name = bin_name();
    if let Ok(p) = std::env::var("DARKRUN_DESKTOP") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(sib) = exe.parent().map(|d| d.join(name)) {
            if sib.is_file() {
                return Some(sib);
            }
        }
    }
    for prof in ["release", "debug"] {
        let p = repo_root.join("target").join(prof).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Launch a resolved binary (detached) pointed at the engine `port`. Unpinned
/// (`DARKRUN_SESSION_ID` cleared) so it opens the run-browser home, whose
/// `current`-focus poller navigates to the run the engine just raised.
fn launch(bin: PathBuf, port: u16) -> Launch {
    let ok = Command::new(&bin)
        .env("DARKRUN_PORT", port.to_string())
        .env_remove("DARKRUN_SESSION_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok();
    if ok {
        Launch::Launched(bin)
    } else {
        Launch::NotFound
    }
}

/// Bring up the desktop app pointed at the engine `port`.
///
/// `DARKRUN_DESKTOP` wins. In a **dev checkout** the local
/// `target/<profile>/darkrun-desktop` is always used — built on demand for the
/// host arch (in the background, so this doesn't block) when it isn't compiled
/// yet. Otherwise the installed sibling binary is launched.
pub fn spawn(repo_root: &Path, port: u16) -> Launch {
    // Explicit override.
    if let Ok(p) = std::env::var("DARKRUN_DESKTOP") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return launch(p, port);
        }
    }
    // Dev: always the local version — build it for this arch if absent.
    if let Some((ws, profile)) = dev_workspace() {
        let bin = ws.join("target").join(&profile).join(bin_name());
        if bin.is_file() {
            return launch(bin, port);
        }
        if spawn_build_then_launch(&ws, &profile, &bin, port) {
            return Launch::Building;
        }
    }
    // Installed plugin: sibling of the engine binary, or the project target dir.
    match find(repo_root) {
        Some(bin) => launch(bin, port),
        None => Launch::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn dev_workspace_detects_a_cargo_target_layout() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        // A darkrun-shaped workspace.
        touch(&ws.join("Cargo.toml"));
        touch(&ws.join("desktop").join("Cargo.toml"));
        let exe = ws.join("target").join("debug").join("darkrun");
        touch(&exe);

        let (got_ws, profile) = dev_workspace_from(&exe).expect("dev workspace");
        assert_eq!(got_ws, ws);
        assert_eq!(profile, "debug");
    }

    #[test]
    fn dev_workspace_rejects_non_target_and_non_darkrun_layouts() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        // Not under target/.
        let stray = ws.join("bin").join("darkrun");
        touch(&stray);
        assert!(dev_workspace_from(&stray).is_none());
        // Under target/ but not the darkrun workspace (no desktop/ crate).
        let exe = ws.join("target").join("release").join("darkrun");
        touch(&exe);
        touch(&ws.join("Cargo.toml"));
        assert!(dev_workspace_from(&exe).is_none());
    }

    // DARKRUN_DESKTOP is process-global; keep its mutation in one sequential test.
    #[test]
    fn find_resolves_env_then_target() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join(bin_name());
        touch(&fake);
        std::env::set_var("DARKRUN_DESKTOP", &fake);
        assert_eq!(find(dir.path()).as_deref(), Some(fake.as_path()));

        std::env::remove_var("DARKRUN_DESKTOP");
        let repo = tempfile::tempdir().unwrap();
        let bin = repo.path().join("target").join("release").join(bin_name());
        touch(&bin);
        assert_eq!(find(repo.path()), Some(bin));
    }
}
