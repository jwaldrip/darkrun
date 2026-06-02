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
//!
//! ## macOS: launch via LaunchServices, not a bare `exec`
//!
//! The MCP server is itself spawned by the harness (Claude Code) in a process
//! context that is *detached from the Aqua GUI session*. A GUI app `exec`'d
//! directly from there cannot reach the WindowServer and AppKit simply `exit()`s
//! it — so `Command::spawn().is_ok()` reports success (fork/exec worked) while the
//! window never appears and the process is gone a moment later. The fix is to hand
//! the launch to **LaunchServices** via `open`, which starts the app *in* the
//! login GUI session regardless of who asked. `open` needs an `.app` bundle, so we
//! materialize a tiny wrapper (Info.plist + a symlink to the real binary) next to
//! the binary on demand. `open --stdout/--stderr` captures the app's output to a
//! log so a launch is never silent again.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where a launched app's stdout/stderr is captured, so a failed launch leaves a
/// trace instead of vanishing silently. Lives under the project's state dir.
fn log_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".darkrun").join("desktop.log")
}

/// Open the launch log for append (creating `.darkrun/` if needed), for wiring a
/// child's stdout/stderr to it.
fn open_log(repo_root: &Path) -> Option<std::fs::File> {
    let path = log_path(repo_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

/// A child's stdout/stderr wired to the launch log, or null if it can't be opened.
fn log_stdio(repo_root: &Path) -> (Stdio, Stdio) {
    match open_log(repo_root) {
        Some(f) => match f.try_clone() {
            Ok(f2) => (Stdio::from(f), Stdio::from(f2)),
            Err(_) => (Stdio::from(f), Stdio::null()),
        },
        None => (Stdio::null(), Stdio::null()),
    }
}

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

/// The minimal `Info.plist` for the macOS launch wrapper. `CFBundleName` is what
/// the Dock/menu-bar show ("darkrun"); the window title is set by the app itself.
#[cfg(target_os = "macos")]
const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>darkrun-desktop</string>
  <key>CFBundleIdentifier</key><string>ai.darkrun.desktop</string>
  <key>CFBundleName</key><string>darkrun</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#;

/// The app icon, embedded so the launch wrapper always ships its own `.icns`
/// (referenced by `CFBundleIconFile` above) without depending on a sibling file.
#[cfg(target_os = "macos")]
const APP_ICON: &[u8] = include_bytes!("../assets/AppIcon.icns");

/// The bundle's inner executable path for a given `.app`.
#[cfg(target_os = "macos")]
fn bundle_exe(bundle: &Path) -> PathBuf {
    bundle.join("Contents").join("MacOS").join("darkrun-desktop")
}

/// Materialize (idempotently) a tiny `.app` wrapper next to `bin` so `open` can
/// hand the launch to LaunchServices. Writes only the plist + icon + dirs (cheap,
/// and valid even before the binary is built); the executable itself is placed by
/// [`sync_bundle_exe`] / the build script. Returns the `.app` path.
///
/// The `Contents/MacOS` executable must be a **real copy** of the binary, NOT a
/// symlink: macOS resolves the main bundle from the executable's path, so a
/// symlink pointing at a binary *outside* the `.app` loses the bundle association
/// and the Dock falls back to a generic icon.
#[cfg(target_os = "macos")]
fn ensure_bundle(bin: &Path) -> std::io::Result<PathBuf> {
    let dir = bin.parent().unwrap_or_else(|| Path::new("."));
    let bundle = dir.join("darkrun-desktop.app");
    let macos = bundle.join("Contents").join("MacOS");
    std::fs::create_dir_all(&macos)?;
    std::fs::write(bundle.join("Contents").join("Info.plist"), INFO_PLIST)?;
    let resources = bundle.join("Contents").join("Resources");
    std::fs::create_dir_all(&resources)?;
    std::fs::write(resources.join("AppIcon.icns"), APP_ICON)?;
    Ok(bundle)
}

/// Copy `bin` into the bundle's `Contents/MacOS` (replacing any stale symlink or
/// older copy) so the launched app is a self-contained bundle with our icon. A
/// no-op when the copy is already current (same size + mtime), to avoid a 40 MB
/// copy on every launch. `touch`es the bundle so LaunchServices re-reads it.
#[cfg(target_os = "macos")]
fn sync_bundle_exe(bundle: &Path, bin: &Path) -> std::io::Result<()> {
    if !bin.is_file() {
        return Ok(());
    }
    let exe = bundle_exe(bundle);
    let current = std::fs::symlink_metadata(&exe);
    let needs_copy = match (&current, bin.metadata()) {
        // Re-copy if the dest is a symlink, a different size, or older than `bin`.
        (Ok(c), Ok(b)) => {
            c.file_type().is_symlink()
                || c.len() != b.len()
                || c.modified().ok() < b.modified().ok()
        }
        _ => true,
    };
    if needs_copy {
        let _ = std::fs::remove_file(&exe);
        std::fs::copy(bin, &exe)?;
        // Bump the bundle mtime so LaunchServices refreshes the cached icon.
        let _ = Command::new("touch").arg(bundle).status();
    }
    Ok(())
}

/// Spawn a **detached** `cargo build -p darkrun-desktop && <launch>` so the build
/// runs in the background and the app launches itself when it completes — the
/// `show` call doesn't block on the (one-time) compile. Build + app output go to
/// the launch log. Returns whether the builder process spawned.
fn spawn_build_then_launch(
    ws: &Path,
    profile: &str,
    bin: &Path,
    port: u16,
    repo_root: &Path,
    session: Option<&str>,
) -> bool {
    let rel = if profile == "release" { " --release" } else { "" };
    let (out, err) = log_stdio(repo_root);
    let mut cmd;
    #[cfg(target_os = "macos")]
    {
        // Pre-create the wrapper (symlink may dangle until the build lands), then
        // launch through LaunchServices so the app reaches the GUI session.
        let bundle = ensure_bundle(bin).map(|b| b.to_string_lossy().into_owned());
        let log = log_path(repo_root);
        // Pin to the run so the post-build launch opens straight to its Review.
        let sess = session
            .map(|s| format!(" --env DARKRUN_SESSION_ID={s}"))
            .unwrap_or_default();
        let script = match bundle {
            // After the build, copy the freshly-built binary INTO the bundle (a
            // real executable, not a symlink) so macOS keeps the bundle/icon
            // association, then `touch` the .app so LaunchServices re-reads it
            // (busting a stale icon cache), then launch.
            Ok(bundle) => format!(
                "cargo build -p darkrun-desktop{rel} && rm -f {exe} && cp {bin} {exe} && touch {bnd} && \
                 exec open -n {bnd} --env DARKRUN_PORT={port}{sess} --stdout {log} --stderr {log}",
                bin = sh_quote(bin),
                // `rm` first so we replace any stale symlink instead of copying
                // *through* it onto the source binary (which would leave the
                // symlink — and the broken icon — in place).
                exe = sh_quote(&bundle_exe(Path::new(&bundle))),
                bnd = sh_quote(Path::new(&bundle)),
                log = sh_quote(&log),
            ),
            // Bundle couldn't be written — fall back to a direct exec.
            Err(_) => format!(
                "cargo build -p darkrun-desktop{rel} && exec {}",
                sh_quote(bin)
            ),
        };
        cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let script = format!(
            "cargo build -p darkrun-desktop{rel} && exec {}",
            sh_quote(bin)
        );
        cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
    }
    #[cfg(windows)]
    {
        let script = format!(
            "cargo build -p darkrun-desktop{rel} && \"{}\"",
            bin.display()
        );
        cmd = Command::new("cmd");
        cmd.arg("/C").arg(script);
    }
    cmd.current_dir(ws)
        .env("DARKRUN_PORT", port.to_string());
    // Non-macOS launches inherit the builder's env (macOS uses `open --env` above).
    match session {
        Some(s) => {
            cmd.env("DARKRUN_SESSION_ID", s);
        }
        None => {
            cmd.env_remove("DARKRUN_SESSION_ID");
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
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

/// Launch a resolved binary pointed at the engine `port`, unpinned
/// (`DARKRUN_SESSION_ID` cleared) so it opens the run-browser home, whose
/// `current`-focus poller navigates to the run the engine just raised. Output is
/// captured to the launch log.
///
/// On **macOS** this goes through LaunchServices (`open` on a generated `.app`
/// wrapper) so the app reaches the login GUI session even though the MCP server
/// is spawned outside it — a direct `exec` there is killed by AppKit before a
/// window appears. Elsewhere a direct detached spawn is fine.
#[cfg(target_os = "macos")]
fn launch(bin: PathBuf, port: u16, repo_root: &Path, session: Option<&str>) -> Launch {
    let bundle = match ensure_bundle(&bin) {
        Ok(b) => b,
        Err(_) => return launch_direct(bin, port, repo_root, session),
    };
    // Put a real copy of the binary inside the bundle (replacing any stale
    // symlink) so the Dock shows our icon; a no-op when already current.
    let _ = sync_bundle_exe(&bundle, &bin);
    let log = log_path(repo_root);
    let _ = open_log(repo_root); // ensure .darkrun/ exists for open's redirect
    // `open` blocks only until LaunchServices accepts the launch, so a non-zero
    // status is a real "couldn't start" signal — unlike a bare fork succeeding.
    let mut cmd = Command::new("open");
    cmd.arg("-n")
        .arg(&bundle)
        .arg("--env")
        .arg(format!("DARKRUN_PORT={port}"));
    // Pin to the run so the app opens straight to its Review (`open` launches in
    // a clean launchd env, so DARKRUN_SESSION_ID must be passed explicitly).
    if let Some(s) = session {
        cmd.arg("--env").arg(format!("DARKRUN_SESSION_ID={s}"));
    }
    let ok = cmd
        .arg("--stdout")
        .arg(&log)
        .arg("--stderr")
        .arg(&log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Launch::Launched(bin)
    } else {
        Launch::NotFound
    }
}

#[cfg(not(target_os = "macos"))]
fn launch(bin: PathBuf, port: u16, repo_root: &Path, session: Option<&str>) -> Launch {
    launch_direct(bin, port, repo_root, session)
}

/// Direct detached spawn (non-macOS, or the macOS bundle fallback). Output goes
/// to the launch log so a crash is traceable. With `session` set the app is
/// PINNED to that run (`DARKRUN_SESSION_ID`) so it opens straight to the Review;
/// `None` opens the unpinned projects home.
fn launch_direct(bin: PathBuf, port: u16, repo_root: &Path, session: Option<&str>) -> Launch {
    let (out, err) = log_stdio(repo_root);
    let mut cmd = Command::new(&bin);
    cmd.env("DARKRUN_PORT", port.to_string());
    match session {
        Some(s) => {
            cmd.env("DARKRUN_SESSION_ID", s);
        }
        None => {
            cmd.env_remove("DARKRUN_SESSION_ID");
        }
    }
    let ok = cmd
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
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
pub fn spawn(repo_root: &Path, port: u16, session: Option<&str>) -> Launch {
    // Explicit override.
    if let Ok(p) = std::env::var("DARKRUN_DESKTOP") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return launch(p, port, repo_root, session);
        }
    }
    // Dev: always the local version, and ALWAYS rebuild it first so the launched
    // app reflects the latest source — `cargo build` is incremental, so it's a
    // near-instant no-op when nothing changed, and correctly recompiles when the
    // UI did. (Previously a once-built binary was launched as-is, so edits to the
    // desktop/UI crates never showed up until the binary was manually deleted.)
    // If the builder can't even be spawned, fall back to launching whatever exists.
    if let Some((ws, profile)) = dev_workspace() {
        let bin = ws.join("target").join(&profile).join(bin_name());
        if spawn_build_then_launch(&ws, &profile, &bin, port, repo_root, session) {
            return Launch::Building;
        }
        if bin.is_file() {
            return launch(bin, port, repo_root, session);
        }
    }
    // Installed plugin: sibling of the engine binary, or the project target dir.
    match find(repo_root) {
        Some(bin) => launch(bin, port, repo_root, session),
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

    #[cfg(target_os = "macos")]
    #[test]
    fn ensure_bundle_writes_plist_and_icon() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("darkrun-desktop");
        touch(&bin);
        let bundle = ensure_bundle(&bin).expect("bundle");

        let plist = std::fs::read_to_string(bundle.join("Contents").join("Info.plist")).unwrap();
        assert!(plist.contains("<key>CFBundleIconFile</key><string>AppIcon</string>"));

        let icon = bundle
            .join("Contents")
            .join("Resources")
            .join("AppIcon.icns");
        let bytes = std::fs::read(&icon).expect("icon written");
        assert_eq!(bytes, APP_ICON);
        // .icns files begin with the "icns" magic.
        assert_eq!(&bytes[..4], b"icns");
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
