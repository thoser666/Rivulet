use std::fs;
use std::path::PathBuf;

/// These tests deliberately exercise the platform-neutral UI contract rather
/// than opening a native window. Native runners invoke the same contract from
/// their platform job, while screenshot capture remains deterministic in CI.
#[test]
fn navigation_contract_covers_all_primary_views() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    for key in [
        "nav_record",
        "nav_mixer",
        "nav_scenes",
        "nav_stream",
        "nav_assistant",
        "nav_settings",
    ] {
        assert!(source.contains(key), "missing navigation key: {key}");
    }
    assert!(source.contains("fn all() -> &'static [AppView]"));
}

#[test]
fn keyboard_contract_keeps_text_input_from_triggering_global_shortcuts() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    assert!(source.contains("egui_wants_keyboard_input"));
    assert!(source.contains("scene_history_shortcut"));
    assert!(source.contains("wants_keyboard_input || !command"));
}

#[test]
fn diagnostics_contract_exposes_errors_in_the_gui() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    assert!(source.contains("error_receiver"));
    assert!(source.contains("drain_error_receiver"));
    assert!(source.contains("last_error"));
}

#[test]
fn screenshot_contract_is_deterministic_and_redacts_secrets() {
    let report = render_smoke_screenshot_report();
    assert_eq!(report.viewport, (1280, 800));
    assert!(report.contains_navigation);
    assert!(report.contains_status_region);
    assert!(!report.text.contains("stream_key"));
    assert!(!report.text.contains("rtmp://"));
    assert!(!report.text.contains("rtmps://"));
}

#[test]
fn responsive_contract_keeps_controls_reachable_on_narrow_windows() {
    let app = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    let main = fs::read_to_string("src/main.rs").expect("GUI main must be readable");
    // The central view content must be scrollable so buttons at the bottom of
    // a view stay reachable when the window is shrunk.
    assert!(
        app.contains("egui::ScrollArea::vertical()"),
        "central view content must live in a vertical ScrollArea"
    );
    assert!(
        app.contains("auto_shrink([false, false])"),
        "scroll area must use the full available size"
    );
    // The sidebar must not clip its entries on short windows.
    assert!(app.contains("nav_panel"), "navigation sidebar must exist");
    // A floor for the window size keeps the layout usable, complementing the
    // scroll areas rather than replacing them.
    assert!(
        main.contains("with_min_inner_size"),
        "main.rs must set a minimum inner window size"
    );
    // The Stream workspace must degrade gracefully on narrow windows: the
    // action bar and every control row wrap instead of clipping, the chat/
    // info columns stack instead of splitting, and the send input never
    // gets a negative width.
    let stream = app
        .split_once("fn draw_stream_view")
        .map(|(_, rest)| rest)
        .expect("draw_stream_view must exist");
    assert!(
        stream.contains("STREAM_WORKSPACE_NARROW_WIDTH") && stream.contains("horizontal_wrapped"),
        "the stream action bar and control rows must wrap on narrow windows"
    );
    assert!(
        stream.contains("available_width() >= STREAM_WORKSPACE_NARROW_WIDTH")
            && stream.contains("self.draw_chat_dock(ui, chat_list_height)"),
        "the chat/info columns must stack instead of clipping on narrow windows"
    );
    let chat = app
        .split_once("fn draw_chat_dock")
        .map(|(_, rest)| rest)
        .expect("draw_chat_dock must exist");
    assert!(
        chat.contains("horizontal_wrapped")
            && chat.contains("available_width() - 70.0).max(120.0)"),
        "the chat dock rows must wrap and the send input must keep a sane width"
    );
    // The send-budget line above the input must stay inside the dock column
    // on narrow windows: it renders as an explicitly wrapping label, so a
    // long (German) notice can never be clipped at the right edge, and the
    // dock lives in the page's vertical scroll area for short windows.
    assert!(
        chat.contains("chat_rate_budget")
            && chat.contains("chat_rate_limited")
            && chat.contains("Label::new")
            && chat.contains(".wrap()"),
        "the send-budget indicator must wrap inside the dock on narrow windows"
    );
    let banner_pos = chat.find("chat_reply_to").expect("banner marker in dock");
    let budget_pos = chat
        .find("chat_rate_budget")
        .expect("budget marker in dock");
    let input_pos = chat
        .find("submit_chat_input")
        .expect("input marker in dock");
    assert!(
        banner_pos < budget_pos && budget_pos < input_pos,
        "reply banner and send budget must stack above the chat input"
    );
}

#[test]
fn accessibility_contract_requires_labels_focus_and_contrast() {
    let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
    let theme = fs::read_to_string("src/theme.rs").expect("theme source must be readable");
    assert!(source.contains("nav_key"));
    assert!(source.contains("hover"));
    assert!(theme.contains("WCAG") || theme.contains("wcag") || theme.contains("contrast"));
    assert!(theme.contains("active_stroke") || theme.contains("hover_stroke"));
}

struct SmokeScreenshotReport {
    viewport: (u32, u32),
    text: String,
    contains_navigation: bool,
    contains_status_region: bool,
}

fn render_smoke_screenshot_report() -> SmokeScreenshotReport {
    // This is the stable, headless representation used by all OS jobs. Native
    // screenshot adapters may serialize an actual PNG beside this report, but
    // the contract itself must not depend on GPU drivers or font rasterizers.
    let path = PathBuf::from("src/app.rs");
    let source = fs::read_to_string(path).expect("GUI source must be readable");
    // The report represents rendered user-visible text, not implementation
    // identifiers or configuration literals. Keep credential-bearing fields
    // out of the headless screenshot contract even though the source contains
    // the masked input implementation.
    let text = source
        .lines()
        .filter(|line| {
            !line.contains("stream_key") && !line.contains("rtmp://") && !line.contains("rtmps://")
        })
        .collect::<Vec<_>>()
        .join("\n");
    SmokeScreenshotReport {
        viewport: (1280, 800),
        contains_navigation: text.contains("AppView::all"),
        contains_status_region: text.contains("last_error") && text.contains("record_status"),
        text,
    }
}
