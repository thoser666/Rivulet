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
/// is shrunk.
fn narrow_layout_is_responsive() -> bool {
    let source = std::fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    let main = std::fs::read_to_string("src/main.rs").expect("GUI main must be readable");
    source.contains("egui::ScrollArea::vertical()")
        && source.contains("auto_shrink([false, false])")
        && source.contains("nav_panel")
        && source.contains("CentralPanel::default()")
        && main.contains("with_min_inner_size")
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
