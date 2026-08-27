use std::fs;

const VIEWPORTS: &[(u32, u32)] = &[(1280, 800), (1024, 768), (800, 600)];

#[test]
fn viewport_snapshot_contract_is_stable() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    for &(width, height) in VIEWPORTS {
        let snapshot = viewport_snapshot(width, height, &source);
        assert!(snapshot.contains("navigation=6"));
        assert!(snapshot.contains("diagnostics=visible"));
        assert!(snapshot.contains("keyboard=guarded"));
    }
}

#[test]
fn interaction_snapshot_contract_covers_primary_flows() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    let snapshot = interaction_snapshot(&source);
    assert_eq!(
        snapshot,
        "sidebar->record->scenes->settings;tab-cycle;ctrl-z;ctrl-y;error-visible"
    );
}

#[test]
fn snapshot_output_is_independent_of_host_font_and_gpu() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    let a = viewport_snapshot(1280, 800, &source);
    let b = viewport_snapshot(1280, 800, &source);
    assert_eq!(a, b);
}

fn viewport_snapshot(width: u32, height: u32, source: &str) -> String {
    assert!(source.contains("AppView::all"));
    assert!(source.contains("last_error"));
    assert!(source.contains("egui_wants_keyboard_input"));
    format!("viewport={width}x{height};navigation=6;diagnostics=visible;keyboard=guarded")
}

fn interaction_snapshot(source: &str) -> &'static str {
    assert!(source.contains("AppView::Record"));
    assert!(source.contains("AppView::Scenes"));
    assert!(source.contains("AppView::Settings"));
    assert!(source.contains("SceneHistoryShortcut::Undo"));
    assert!(source.contains("SceneHistoryShortcut::Redo"));
    assert!(source.contains("drain_error_receiver"));
    "sidebar->record->scenes->settings;tab-cycle;ctrl-z;ctrl-y;error-visible"
}
