//! Mockup generator: the desktop Units tab with the (existing) [`UnitGraph`]
//! wired in — rendered through the REAL `LayeredLayout` engine and the real
//! theme tokens, so what you see is what the shipped component draws.
//!
//! Emits `/tmp/darkrun-dag-mockup/{dark,light}.html`.

use darkrun_ui::prelude::{GraphEdge, GraphLayout, GraphNode, LayeredLayout, LayoutOptions};
use darkrun_ui::tokens::THEME_CSS;

/// A unit in the mocked build station: slug, label, status tone, type, passes.
struct U(&'static str, &'static str, &'static str, &'static str, u32, &'static [&'static str]);

fn main() {
    // A realistic Build-station wave: tones mirror the run UI conventions
    // (done=ok, in-progress=info, pending=muted, blocked=danger).
    let units = [
        U("contract-types", "contract-types", "ok", "feature", 3, &[]),
        U("parser", "parser", "ok", "feature", 2, &[]),
        U("state-engine", "state-engine", "info", "feature", 4, &["contract-types", "parser"]),
        U("api-routes", "api-routes", "info", "feature", 1, &["contract-types"]),
        U("cli-surface", "cli-surface", "pending", "feature", 0, &["parser"]),
        U("desktop-wire", "desktop-wire", "pending", "feature", 0, &["state-engine", "api-routes"]),
        U("e2e-tests", "e2e-tests", "danger", "test", 1, &["desktop-wire"]),
        U("docs-pass", "docs-pass", "pending", "doc", 0, &["state-engine"]),
    ];

    let nodes: Vec<GraphNode> = units.iter().map(|u| GraphNode::new(u.0, u.1)).collect();
    let mut edges: Vec<GraphEdge> = Vec::new();
    for u in &units {
        for dep in u.5 {
            edges.push(GraphEdge { from: (*dep).to_string(), to: u.0.to_string() });
        }
    }
    let result = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());

    let tone_var = |t: &str| match t {
        "ok" => "var(--dr-status-ok)",
        "info" => "var(--dr-status-info)",
        "danger" => "var(--dr-status-danger)",
        _ => "var(--dr-text-faint)",
    };
    let tone_label = |t: &str| match t {
        "ok" => "completed",
        "info" => "in_progress",
        "danger" => "blocked",
        _ => "pending",
    };

    // ── The SVG, mirroring graph/view.rs exactly ─────────────────────────
    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg class="dr-unit-graph" width="{w}" height="{h}" viewBox="0 0 {w} {h}" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="unit dependency graph" style="background:var(--dr-surface-raised);border:1px solid var(--dr-border);border-radius:8px;display:block;max-width:100%;height:auto;font-family:'JetBrains Mono','SF Mono',Menlo,monospace;">
<defs><marker id="dr-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="var(--dr-border-strong)"/></marker></defs>
"#,
        w = result.width,
        h = result.height,
    ));
    for e in &result.edges {
        let dx = (e.x2 - e.x1).abs().max(24.0) * 0.5;
        svg.push_str(&format!(
            r#"<path d="M {x1} {y1} C {c1x} {y1}, {c2x} {y2}, {x2} {y2}" fill="none" stroke="var(--dr-border-strong)" stroke-width="1.5" marker-end="url(#dr-arrow)"/>
"#,
            x1 = e.x1, y1 = e.y1, c1x = e.x1 + dx, c2x = e.x2 - dx, x2 = e.x2, y2 = e.y2,
        ));
    }
    for n in &result.nodes {
        let tone = units.iter().find(|u| u.0 == n.id).map(|u| u.2).unwrap_or("pending");
        svg.push_str(&format!(
            r#"<g class="dr-graph-node" data-id="{id}"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="var(--dr-surface-overlay)" stroke="{color}" stroke-width="1.5"/><text x="{cx}" y="{ly}" fill="var(--dr-text)" font-size="12" text-anchor="middle">{label}</text></g>
"#,
            id = n.id, x = n.x, y = n.y, w = n.width, h = n.height,
            color = tone_var(tone), cx = n.cx(), ly = n.cy() + 4.0, label = n.label,
        ));
    }
    svg.push_str("</svg>\n");

    // ── Unit rows beneath the graph (the existing UnitRow look) ─────────
    let mut rows = String::new();
    for u in &units {
        let pass = if u.4 > 0 { format!(r#"<span class="pass">pass {}</span>"#, u.4) } else { String::new() };
        rows.push_str(&format!(
            r#"<div class="unit-row"><span class="dot" style="background:{c}"></span><span class="title">{t}</span><span class="chip">{ty}</span>{pass}<span class="status" style="color:{c};border-color:{c}">{st}</span></div>
"#,
            c = tone_var(u.2), t = u.1, ty = u.3, st = tone_label(u.2),
        ));
    }

    let page = |theme: &str| format!(
        r#"<!doctype html><html data-theme="{theme}"><head><meta charset="utf-8"><style>
{THEME_CSS}
body {{ margin:0; background:var(--dr-surface-base); color:var(--dr-text); font-family:Inter,-apple-system,'Segoe UI',sans-serif; }}
.frame {{ max-width:980px; margin:28px auto; padding:0 24px; }}
.window {{ border:1px solid var(--dr-border); border-radius:14px; background:var(--dr-surface-base); overflow:hidden; box-shadow:0 18px 50px rgba(0,0,0,.25); }}
.titlebar {{ display:flex; align-items:center; gap:8px; padding:10px 14px; border-bottom:1px solid var(--dr-border); background:var(--dr-surface-raised); }}
.dotsys {{ width:11px; height:11px; border-radius:50%; opacity:.9 }}
.t {{ font-family:'JetBrains Mono',monospace; font-size:12px; color:var(--dr-text-muted); margin-left:8px }}
.ctx {{ display:flex; align-items:center; gap:10px; padding:14px 18px 0; font-family:'JetBrains Mono',monospace; font-size:12px; color:var(--dr-text-muted); }}
.ctx b {{ color:var(--dr-text); font-family:Inter,sans-serif; font-size:14px; }}
.badge {{ border:1px solid var(--dr-border); border-radius:999px; padding:2px 9px; font-size:11px; }}
.badge.fill {{ background:var(--dr-accent); color:var(--dr-on-accent); border-color:transparent; font-weight:600 }}
.tabs {{ display:flex; gap:4px; padding:12px 18px 0; border-bottom:1px solid var(--dr-border); }}
.tab {{ font-size:13px; padding:8px 14px; color:var(--dr-text-muted); border:1px solid transparent; border-bottom:none; border-radius:8px 8px 0 0; }}
.tab.active {{ color:var(--dr-text); background:var(--dr-surface-raised); border-color:var(--dr-border); font-weight:600; position:relative; top:1px; }}
.pane {{ padding:18px; background:var(--dr-surface-raised); }}
.graphhead {{ display:flex; align-items:center; justify-content:space-between; margin-bottom:10px; }}
.graphhead .k {{ font-family:'JetBrains Mono',monospace; font-size:11px; letter-spacing:.08em; text-transform:uppercase; color:var(--dr-accent); }}
.legend {{ display:flex; gap:14px; font-family:'JetBrains Mono',monospace; font-size:11px; color:var(--dr-text-muted); }}
.legend i {{ display:inline-block; width:9px; height:9px; border-radius:2px; margin-right:5px; vertical-align:-1px; border:1.5px solid; background:var(--dr-surface-overlay); }}
.unit-row {{ display:flex; align-items:center; gap:10px; padding:9px 12px; border:1px solid var(--dr-border); border-radius:8px; background:var(--dr-surface-overlay); margin-top:8px; }}
.unit-row .dot {{ width:8px; height:8px; border-radius:50%; }}
.unit-row .title {{ font-size:13px; font-weight:600; flex:0 0 auto; }}
.unit-row .chip {{ font-family:'JetBrains Mono',monospace; font-size:10px; color:var(--dr-text-faint); border:1px solid var(--dr-border); border-radius:999px; padding:1px 8px; }}
.unit-row .pass {{ font-family:'JetBrains Mono',monospace; font-size:10px; color:var(--dr-text-muted); }}
.unit-row .status {{ margin-left:auto; font-family:'JetBrains Mono',monospace; font-size:10px; border:1px solid; border-radius:999px; padding:2px 9px; }}
.note {{ margin:14px auto 0; max-width:980px; padding:0 24px; font-family:'JetBrains Mono',monospace; font-size:11px; color:var(--dr-text-faint); }}
</style></head><body>
<div class="frame"><div class="window">
  <div class="titlebar"><span class="dotsys" style="background:#ff5f57"></span><span class="dotsys" style="background:#febc2e"></span><span class="dotsys" style="background:#28c840"></span><span class="t">darkrun — darkrun-sim · review</span></div>
  <div class="ctx"><b>darkrun-sim</b><span class="badge">software</span><span>station:</span><span class="badge fill">build</span><span>· manufacture</span></div>
  <div class="tabs"><div class="tab active">Units</div><div class="tab">Outputs</div><div class="tab">Knowledge</div><div class="tab">Feedback</div><div class="tab">Overview</div></div>
  <div class="pane">
    <div class="graphhead"><span class="k">dependency graph · 4 waves</span>
      <span class="legend"><span><i style="border-color:var(--dr-status-ok)"></i>completed</span><span><i style="border-color:var(--dr-status-info)"></i>in&nbsp;progress</span><span><i style="border-color:var(--dr-text-faint)"></i>pending</span><span><i style="border-color:var(--dr-status-danger)"></i>blocked</span></span>
    </div>
    {svg}
    {rows}
  </div>
</div></div>
<div class="note">mockup — the existing darkrun-ui UnitGraph (real LayeredLayout output) wired into the desktop Units tab · {theme} theme</div>
</body></html>"#,
    );

    let out = std::path::Path::new("/tmp/darkrun-dag-mockup");
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(out.join("dark.html"), page("dark")).unwrap();
    std::fs::write(out.join("light.html"), page("light")).unwrap();
    println!("wrote /tmp/darkrun-dag-mockup/{{dark,light}}.html  ({} nodes, {} edges, {}x{})",
        result.nodes.len(), result.edges.len(), result.width, result.height);
}
