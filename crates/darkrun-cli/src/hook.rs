//! Plugin hook handlers (`darkrun hook <name>`).
//!
//! Claude Code invokes these from `plugin/hooks/hooks.json` on tool use. They
//! are ADVISORY: fast, and they must **never block** a tool. On any unknown
//! hook, malformed input, or internal error they simply drain stdin and exit 0
//! (allow). Only `inject-state-file` emits anything — optional active-Run
//! context — and only when it can do so cleanly.

use std::io::Read;

use darkrun_core::StateStore;

/// Run a hook by name. Always succeeds; never blocks the triggering tool.
pub fn run(name: &str) {
    // Drain stdin so Claude Code's payload pipe closes cleanly. Content is
    // tolerated and (for most hooks) ignored.
    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);

    match name {
        // The only hook that emits: surface the active Run as extra context.
        "inject-state-file" => inject_state_file(),
        // redirect-plan-mode, guard-workflow-fields, prompt-guard, workflow-guard,
        // context-monitor, stamp-agent-write, edit-auto-read-hint — and anything
        // unrecognized — are advisory no-ops (allow). Richer behavior can be
        // layered in later without ever blocking a tool.
        _ => {}
    }
}

/// Emit the active Run's state as PreToolUse `additionalContext`, if there is
/// one. Silent on any error — it must never block the tool.
fn inject_state_file() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let store = StateStore::new(&cwd);
    let Ok(Some(slug)) = store.active_run() else {
        return;
    };
    let Ok(run) = store.read_run(&slug) else {
        return;
    };
    let ctx = format!(
        "Active darkrun Run `{slug}` — factory `{}`, station `{}`. State lives under \
         `.darkrun/{slug}/`; drive it with the darkrun_run_* tools.",
        run.frontmatter.factory, run.frontmatter.active_station,
    );
    let out = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": ctx,
        }
    });
    println!("{out}");
}
