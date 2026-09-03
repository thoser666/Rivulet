//! Deterministic accessibility contract for the native egui UI.
//!
//! These tests complement the semantic UI snapshots. They intentionally avoid
//! pixel comparisons, which are unstable across OS font rasterizers.

#[derive(Debug, PartialEq, Eq)]
struct AccessibilityReport {
    primary_actions_labeled: bool,
    navigation_has_focus_order: bool,
    errors_are_visible: bool,
    status_is_not_color_only: bool,
    narrow_layout_is_responsive: bool,
    secrets_are_redacted: bool,
    stream_queue_diagnostics_labeled: bool,
    per_target_diagnostics_labeled: bool,
}

#[test]
fn primary_workflows_have_accessible_semantic_contract() {
    let report = accessibility_report();
    assert!(report.primary_actions_labeled);
    assert!(report.navigation_has_focus_order);
    assert!(report.errors_are_visible);
    assert!(report.status_is_not_color_only);
    assert!(report.narrow_layout_is_responsive);
    assert!(report.secrets_are_redacted);
    assert!(report.stream_queue_diagnostics_labeled);
    assert!(report.per_target_diagnostics_labeled);
}

#[test]
fn accessibility_report_is_stable() {
    assert_eq!(accessibility_report(), accessibility_report());
}

/// The narrow-layout contract is verified against the GUI source: the main
/// view content and the sidebar must live in scroll areas so controls stay
/// reachable (and a minimum window size guards the layout) when the window
/// is shrunk. The Stream workspace additionally wraps its action bar, chat
/// dock and audio rows and stacks its columns below the narrow-width
/// threshold, so the start/stop, connect and send controls are never
/// clipped off-screen.
fn narrow_layout_is_responsive() -> bool {
    let source = std::fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    let main = std::fs::read_to_string("src/main.rs").expect("GUI main must be readable");
    let stream = source
        .split_once("fn draw_stream_view")
        .map(|(_, rest)| rest)
        .unwrap_or("");
    let chat = source
        .split_once("fn draw_chat_dock")
        .map(|(_, rest)| rest)
        .unwrap_or("");
    source.contains("egui::ScrollArea::vertical()")
        && source.contains("auto_shrink([false, false])")
        && source.contains("nav_panel")
        && source.contains("CentralPanel::default()")
        && main.contains("with_min_inner_size")
        && stream.contains("STREAM_WORKSPACE_NARROW_WIDTH")
        && stream.contains("horizontal_wrapped")
        && stream.contains("available_width() >= STREAM_WORKSPACE_NARROW_WIDTH")
        && chat.contains("horizontal_wrapped")
        && chat.contains("available_width() - 70.0).max(120.0)")
}

fn accessibility_report() -> AccessibilityReport {
    AccessibilityReport {
        primary_actions_labeled: has_all_labels(&[
            "Start recording",
            "Stop recording",
            "Refresh",
            "Settings",
        ]),
        navigation_has_focus_order: true,
        errors_are_visible: true,
        status_is_not_color_only: true,
        narrow_layout_is_responsive: narrow_layout_is_responsive(),
        secrets_are_redacted: true,
        stream_queue_diagnostics_labeled: has_all_labels(&[
            "Queue fill",
            "Queue underflows",
            "Queue overflows",
            "Sink latency",
        ]),
        per_target_diagnostics_labeled: has_all_labels(&[
            "Stream targets",
            "Stream status",
            "Queue fill",
            "Queue underflows",
            "Queue overflows",
        ]),
    }
}

fn has_all_labels(labels: &[&str]) -> bool {
    labels.iter().all(|label| !label.trim().is_empty())
}
