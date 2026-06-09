//! Lightweight informational tools: version, changelog, report.
//!
//! These back the `darkrun-version`, `darkrun-changelog`, and `darkrun-report`
//! skills. They don't touch run state — they report what build is running, the
//! release notes, and capture a feedback report.

use std::path::PathBuf;

use serde::Serialize;

/// What darkrun build is actually running.
#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    /// The engine version baked into the binary at build time.
    pub engine_version: String,
    /// The plugin's declared version (from `plugin.json`), or `unknown`.
    pub plugin_version: String,
    /// `debug` or `release`.
    pub build: String,
    /// The platform the engine was built for (`os-arch`).
    pub target: String,
    /// The binary the process launched, if resolvable.
    pub entry: String,
    /// Whether the plugin and engine versions disagree (a stale build hint).
    pub mismatch: bool,
}

/// A file under the installed plugin root (`CLAUDE_PLUGIN_ROOT`), if that env is
/// set and the file exists.
fn plugin_file(rel: &str) -> Option<PathBuf> {
    let root = std::env::var("CLAUDE_PLUGIN_ROOT").ok()?;
    let path = PathBuf::from(root).join(rel);
    path.exists().then_some(path)
}

/// Read the plugin's declared version from its manifest.
fn plugin_version() -> Option<String> {
    let path = plugin_file(".claude-plugin/plugin.json")?;
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("version")?.as_str().map(String::from)
}

/// Report the running engine/plugin version, build kind, target, and entry.
pub fn version_info() -> VersionInfo {
    let engine_version = env!("CARGO_PKG_VERSION").to_string();
    let plugin_version = plugin_version().unwrap_or_else(|| "unknown".to_string());
    let mismatch = plugin_version != "unknown" && plugin_version != engine_version;
    VersionInfo {
        engine_version,
        plugin_version,
        build: if cfg!(debug_assertions) { "debug" } else { "release" }.to_string(),
        target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        entry: std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_default(),
        mismatch,
    }
}

/// Read the changelog text — from the installed plugin root, else the current
/// directory. `None` when none is found.
fn read_changelog() -> Option<String> {
    if let Some(p) = plugin_file("CHANGELOG.md") {
        if let Ok(t) = std::fs::read_to_string(p) {
            return Some(t);
        }
    }
    std::fs::read_to_string("CHANGELOG.md").ok()
}

/// Return the changelog. With a `version`, return just that release's section
/// (falling back to the whole text if the version isn't found).
pub fn changelog(version: Option<&str>) -> String {
    let Some(text) = read_changelog() else {
        return "No changelog is available in this install.".to_string();
    };
    match version {
        Some(v) => section_for(&text, v)
            .unwrap_or_else(|| format!("No changelog entry for `{v}`. Full changelog:\n\n{text}")),
        None => text,
    }
}

/// Extract the `## <version> …` section (up to the next `## ` heading) from a
/// keep-a-changelog-style document.
fn section_for(text: &str, version: &str) -> Option<String> {
    let mut lines = text.lines().peekable();
    let mut out: Vec<&str> = Vec::new();
    let mut capturing = false;
    for line in &mut lines {
        if let Some(heading) = line.strip_prefix("## ") {
            if capturing {
                break; // next release heading ends the section
            }
            if heading.contains(version) {
                capturing = true;
                out.push(line);
                continue;
            }
        }
        if capturing {
            out.push(line);
        }
    }
    (!out.is_empty()).then(|| out.join("\n").trim_end().to_string())
}

/// A captured feedback report.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Where the report was saved locally.
    pub saved_to: String,
    /// The URL to file it with the maintainers (no hosted endpoint yet).
    pub file_at: String,
    /// The synthesized report body.
    pub message: String,
}

/// Capture a feedback report. There is no hosted intake endpoint yet, so the
/// report is written under `.darkrun/reports/` and the caller is pointed at the
/// project's issue tracker to post it.
pub fn report(
    repo_root: &std::path::Path,
    message: &str,
    contact_email: Option<&str>,
    name: Option<&str>,
) -> std::io::Result<Report> {
    let dir = repo_root.join(".darkrun").join("reports");
    std::fs::create_dir_all(&dir)?;
    let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) + 1;
    let id = format!("report-{n:03}");
    let mut doc = String::from("---\n");
    if let Some(c) = contact_email {
        doc.push_str(&format!("contact_email: {c}\n"));
    }
    if let Some(nm) = name {
        doc.push_str(&format!("name: {nm}\n"));
    }
    doc.push_str("---\n");
    doc.push_str(message.trim());
    doc.push('\n');
    let path = dir.join(format!("{id}.md"));
    std::fs::write(&path, &doc)?;
    Ok(Report {
        saved_to: path.to_string_lossy().to_string(),
        file_at: "https://github.com/darkrun-ai/darkrun/issues/new".to_string(),
        message: message.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_info_reports_engine_and_build() {
        let v = version_info();
        assert!(!v.engine_version.is_empty());
        assert!(v.build == "debug" || v.build == "release");
        assert!(v.target.contains('-'));
    }

    #[test]
    fn section_for_extracts_one_release() {
        let text = "# Changelog\n\n## 0.2.0\n\n- new\n\n## 0.1.0\n\n- first\n";
        let s = section_for(text, "0.1.0").expect("section");
        assert!(s.contains("## 0.1.0"));
        assert!(s.contains("- first"));
        assert!(!s.contains("0.2.0"));
    }

    #[test]
    fn section_for_none_when_absent() {
        assert!(section_for("## 1.0.0\n- x\n", "9.9.9").is_none());
    }

    #[test]
    fn report_writes_a_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let r = report(dir.path(), "  it broke  ", Some("a@b.c"), None).unwrap();
        assert!(r.saved_to.ends_with("report-001.md"));
        assert_eq!(r.message, "it broke");
        let doc = std::fs::read_to_string(&r.saved_to).unwrap();
        assert!(doc.contains("contact_email: a@b.c"));
        assert!(doc.contains("it broke"));
    }

    #[test]
    fn report_records_the_reporter_name() {
        let dir = tempfile::tempdir().unwrap();
        let r = report(dir.path(), "msg", None, Some("Ada")).unwrap();
        let doc = std::fs::read_to_string(&r.saved_to).unwrap();
        assert!(doc.contains("name: Ada"));
    }

    #[test]
    fn changelog_without_a_file_explains_absence() {
        // From a temp cwd with no CHANGELOG.md, the reader reports the absence.
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::current_dir().unwrap();
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        std::env::remove_var("CLAUDE_PLUGIN_ROOT");
        let out = changelog(None);
        std::env::set_current_dir(prev).unwrap();
        assert!(out.contains("No changelog"));
    }

    #[test]
    fn plugin_version_resolves_from_the_plugin_root_and_flags_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude-plugin")).unwrap();
        std::fs::write(
            dir.path().join(".claude-plugin").join("plugin.json"),
            r#"{"version":"99.99.99"}"#,
        )
        .unwrap();
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAUDE_PLUGIN_ROOT", dir.path());
        let v = version_info();
        let pv = plugin_version();
        std::env::remove_var("CLAUDE_PLUGIN_ROOT");
        assert_eq!(pv.as_deref(), Some("99.99.99"));
        assert_eq!(v.plugin_version, "99.99.99");
        assert!(v.mismatch, "a differing plugin version flags a mismatch");
    }

    #[test]
    fn section_for_breaks_at_the_next_release_heading() {
        // Extracting a NON-last section exercises the break when the scan reaches
        // the following `## ` heading.
        let text = "# Changelog\n\n## 0.2.0\n\n- new\n\n## 0.1.0\n\n- first\n";
        let s = section_for(text, "0.2.0").expect("section");
        assert!(s.contains("- new"));
        assert!(!s.contains("- first"), "the next release is not absorbed");
    }

    #[test]
    fn changelog_reads_the_plugin_root_and_slices_a_version() {
        // Drive read_changelog's plugin-root branch (no cwd mutation, so this is
        // robust under parallel tests) + changelog's version dispatch.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CHANGELOG.md"),
            "# Changelog\n\n## 0.2.0\n\n- two\n\n## 0.1.0\n\n- one\n",
        )
        .unwrap();
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAUDE_PLUGIN_ROOT", dir.path());
        let sliced = changelog(Some("0.1.0"));
        let whole = changelog(None);
        let missing = changelog(Some("9.9.9"));
        std::env::remove_var("CLAUDE_PLUGIN_ROOT");
        assert!(sliced.contains("- one") && !sliced.contains("- two"));
        assert!(whole.contains("## 0.2.0") && whole.contains("## 0.1.0"));
        // An unknown version falls back to the whole text with a note.
        assert!(missing.contains("No changelog entry for `9.9.9`") && missing.contains("- one"));
    }

    /// Serializes the tests that mutate process-global env / cwd.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
