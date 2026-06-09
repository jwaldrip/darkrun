//! darkrun-desktop — the Dioxus cross-platform review app.
//!
//! The chrome is built entirely from the shared [`darkrun_ui`] design system so
//! the desktop app and the website stay visually identical (dark-only, the
//! darkrun brand). This binary connects to the local engine over a WebSocket
//! (`ws://127.0.0.1:PORT/ws/session/:id`), renders the live Review session — the
//! station pipeline, the unit list with completion criteria, declared outputs,
//! and a Checkpoint bar — and POSTs approve / request-changes decisions back to
//! `POST /review/:id/decide`.
//!
//! The session id and engine port are read from the environment so the engine
//! can launch the app pointed at a live run:
//!   - `DARKRUN_PORT`       (default `7878`)
//!   - `DARKRUN_SESSION_ID` (default `current`)

use darkrun_ui::prelude::*;

use darkrun_desktop::{map, wire};

mod home;
mod review;

use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use wire::ConnConfig;

fn main() {
    // Sentry for the desktop surface — the guard lives for the whole process.
    // The DSN is compiled into the distributed app; a no-DSN build is a no-op.
    let _sentry = darkrun_telemetry::init("desktop");
    // A titled, focused window so a launched app is recognizable and comes to
    // the front (the engine spawns this from the MCP server, not Finder, so it
    // must request focus or it opens hidden behind the terminal).
    let window = WindowBuilder::new()
        .with_title("darkrun")
        .with_inner_size(LogicalSize::new(1040.0, 760.0))
        .with_focused(true)
        .with_visible(true);
    // macOS: drop the separate title bar and let the content fill up to the top,
    // so the shell toolbar (the wordmark + theme control) IS the title bar, with
    // the traffic lights floating over its left. The toolbar carries an
    // `-webkit-app-region:drag` region so the window stays draggable by it.
    // Transparent window: macOS rounds the window corners, and with a fullsize
    // content view the square webview can't fill those rounded corners — the
    // window's own (appearance-tracking, so often dark) backing bleeds through
    // as a dark crescent when the in-app theme is light. Making the window
    // transparent removes that backing; the opaque, theme-painted `html,body`
    // (see SHELL_CSS) becomes the visible fill in every theme, so the corner
    // always matches the active theme instead of the OS appearance.
    #[cfg(target_os = "macos")]
    let window = {
        use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
        window
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
            .with_transparent(true)
    };
    // Persist the webview's storage (localStorage, where the theme override is
    // saved) under a stable per-user data directory. Without this the webview
    // gets an ephemeral store that's wiped each launch, so a pinned Light/Dark
    // theme would reset to System on every relaunch.
    let mut cfg = Config::new()
        .with_window(window)
        // Clear backing so nothing shows behind the theme-painted body.
        .with_background_color((0, 0, 0, 0));
    if let Some(data_dir) = dirs::data_dir() {
        cfg = cfg.with_data_directory(data_dir.join("darkrun").join("webview"));
    }
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(app);
}

/// Top-level app: reads the launch config from the environment and opens the
/// full desktop **shell** (toolbar + sidebar + main pane) in every case.
///
/// When the engine launches us **pinned** (`DARKRUN_SESSION_ID` set), we open
/// that run *inside* the shell — selected in the sidebar, its live Review in the
/// main pane — rather than a bare, chrome-less Review. Unpinned, the shell opens
/// on the project/run browser. Either way the user always gets the same native
/// shell (sidebar of projects + runs, Mine/All, search, theme control).
fn app() -> Element {
    let (cfg, pinned) = ConnConfig::from_env_pinned();
    // Pinned → pre-select that run so it opens immediately; unpinned → no
    // pre-selection (the shell's welcome / browser).
    let initial_session = pinned.then(|| cfg.session_id.clone());
    rsx! {
        style { "{darkrun_ui::tokens::THEME_CSS}" }
        home::HomeApp { cfg, project_path: None, initial_session }
    }
}
