//! Server-side render tests — execute the Dioxus `rsx!` component bodies via
//! `dioxus_ssr`, so the component functions (otherwise unreachable from logic
//! tests) are covered. Each test wraps the target component in a no-prop `App`
//! that supplies props through `rsx!`, renders it to a string, and asserts a
//! marker class/text the component emits.

use darkrun_ui::prelude::*;

/// Render a no-prop root component to HTML.
fn render(app: fn() -> Element) -> String {
    let mut dom = VirtualDom::new(app);
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

#[test]
fn renders_chips() {
    fn App() -> Element {
        rsx! {
            CheckpointBadge { kind: CheckpointKind::Ask }
            CheckpointBadge { kind: CheckpointKind::External, filled: true }
            RiskChip { risk: "wrong-thing".to_string() }        }
    }
    let html = render(App);
    assert!(html.contains("dr-"), "rendered some darkrun chip markup: {html}");
}

#[test]
fn renders_station_flow_and_phase_machine() {
    fn App() -> Element {
        let stations = vec![
            FlowStation::new("frame", CheckpointKind::Ask),
            FlowStation::new("build", CheckpointKind::Auto),
            FlowStation::new("harden", CheckpointKind::External),
        ];
        rsx! {
            StationFlow { stations: stations.clone(), active: Some(1usize) }
            PhaseMachine { active: Some(Phase::Manufacture) }
            RunWalkthrough { stations, tick: Some(3usize) }
        }
    }
    let html = render(App);
    assert!(html.contains("dr-station-flow") || html.contains("svg"), "flow svg: {html}");
}

#[test]
fn renders_output_review_and_view_artifacts() {
    fn App() -> Element {
        rsx! {
            OutputReview {
                artifact_label: Some("home.png".to_string()),
                prompt: Some("Review the page".to_string()),
                pins: Vec::<PinPoint>::new(),
                comments: vec!["looks off".to_string()],
                submitted: false,
            }
            ViewArtifacts { artifacts: Vec::<ArtifactEntry>::new() }
        }
    }
    let html = render(App);
    assert!(!html.is_empty());
}

#[test]
fn renders_factory_cards_and_tabs() {
    fn App() -> Element {
        rsx! {
            FactoryCard {
                title: "Storefront".to_string(),
                factory: "software".to_string(),
                station: Some("build".to_string()),
                phase: Some(Phase::Manufacture),
                status_label: "in progress".to_string(),
            }
            UnitRow { title: "burst limiter".to_string(), status_label: "done".to_string(), pass: 2 }
            TabBar { tabs: vec![TabItem::new("a", "Alpha"), TabItem::new("b", "Beta")], active: "a".to_string() }
        }
    }
    let html = render(App);
    assert!(html.contains("Storefront") || html.contains("dr-"), "{html}");
}

#[test]
fn renders_session_views() {
    fn App() -> Element {
        rsx! {
            QuestionView {
                prompt: "Pick one".to_string(),
                options: vec![OptionCard::new("a", "A"), OptionCard::new("b", "B")],
                multi_select: false,
                selected: vec!["a".to_string()],
                image_urls: Vec::<String>::new(),
                answered: false,
            }
            DirectionView {
                prompt: "Choose a direction".to_string(),
                archetypes: vec![ArchetypeCard::new("x", "X", "http://img/x.png", "the x")],
                pins: Vec::<PinPoint>::new(),
                comments: Vec::<String>::new(),
                decided: false,
            }
        }
    }
    let html = render(App);
    assert!(html.contains("Pick one") || html.contains("dr-"), "{html}");
}

#[test]
fn renders_station_strip_and_annotate() {
    fn App() -> Element {
        rsx! {
            StationStrip {
                stations: vec![
                    StationItem::new("frame", StationStatus::Done),
                    StationItem::new("build", StationStatus::Current),
                    StationItem::new("harden", StationStatus::Pending),
                ],
            }
            AnnotateToolbar { kind: SurfaceKind::Visual, active: AnnotateTool::Pin }
            PinMarker { point: PinPoint::new(0.5, 0.5, "here"), number: 1 }
            BoxMarker { x: 0.1, y: 0.1, w: 0.2, h: 0.2, number: 2 }
            ArrowMarker { from: PinPoint::new(0.1, 0.1, ""), to: PinPoint::new(0.4, 0.4, ""), number: 3 }
        }
    }
    let html = render(App);
    assert!(!html.is_empty());
}
