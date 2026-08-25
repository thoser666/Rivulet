# Rivulet UI / Design Guide

This document describes the GUI's structure and the egui conventions new
features must follow. It is the single reference for *where* UI code lives and
*how* it is written, so the growing feature set (M1 recording, M2 scenes,
M3 streaming, M9 assistant) stays consistent and navigable.

The GUI is a single egui/eframe application in `rivulet-gui/`:

- `src/main.rs` — window setup, icon, native options
- `src/app.rs` — `RivuletApp` (state + all rendering), platform capture
  handlers, and the pure-logic unit tests

## Window facts

| Aspect | Value |
| --- | --- |
| Default size | `800 × 600` (`main.rs`, `with_inner_size`) |
| App ID | `rivulet_main_window` |
| Title | set from the crate name at startup |
| Icon | embedded `assets/rivulet_logo.ico` (`create_icon`) |

## Navigation structure

**Current layout:** a fixed left navigation panel (`egui::Panel::left`) drives
the main navigation. Each sidebar entry maps to an `AppView` variant and the
central panel renders the corresponding section. A future compact/collapsed
sidebar may be added once iconography and keyboard navigation are specified.

| Sidebar section | AppView | Milestone | Content |
| --- | --- | --- | --- |
| Record | `Record` | M0/M1 | capture source (screen/window/camera/game), region, codec, preset, timer/FPS overlay |
| Mixer | `Mixer` | M1 | audio filters, monitoring, levels |
| Scenes | `Scenes` | M2 | placeholder (scenes, sources, transitions) |
| Stream | `Stream` | M3 | placeholder (RTMP, stream keys, dual output) |
| Assistant | `Assistant` | M9 | placeholder (local LLM chat) |
| Settings | `Settings` | — | hotkeys, updates, language, general |

The top bar contains only app-level actions (File and Language); feature
content stays in the central panel and is separated with `ui.separator()`.
The top bar and navigation use the shared translucent `theme::glass_frame()`.

## Layout conventions

- **Top bar** (`egui::Panel::top` + `egui::MenuBar`): app-level actions only
  (File, Language, later View/Help). Never put section content here.
- **Main content**: `egui::CentralPanel`. Sections are introduced with
  `ui.label(egui::RichText::new(self.tr("key")).strong())` followed by a
  `ui.separator()`.
- **Modal editors** (e.g. the region selector): floating
  `egui::Window::new(...)` rendered *after* the panels so it draws on top.
  Their content uses `ui.max_rect()`/available bounds and preserves aspect ratio
  instead of assuming a fixed desktop viewport.
  Modal windows must be drawn via a dedicated `draw_<name>_editor` method and
  gated by a `bool`/`Option` field on `RivuletApp`.
- **Live previews** (e.g. `RegionPreview`, `GamePreview`): a struct holding
  an `egui::TextureHandle` plus the frame size and the identity of what was
  captured (monitor name / window id). The frame is grabbed off the UI thread
  (via xcap) and refreshed on selection change or on a fixed interval
  (`GAME_PREVIEW_REFRESH_INTERVAL`), then drawn with
  `ui.painter().image(texture.id(), rect, uv, ...)`. Keep the refresh
  decision (`should_refresh`) as pure logic so it is unit-testable, and show
  a localized error when the frame cannot be captured.
- **Live lists** (e.g. the game-window picker): while the picker is open its
  underlying list is re-enumerated periodically (`GAME_WINDOWS_REFRESH_INTERVAL`)
  so new/closed windows appear without manual action. Re-resolve the current
  selection **by id** (`preserve_selected_game_window`) instead of by index so
  an auto-refresh never silently deselects the window the user chose; keep that
  re-mapping pure and unit-tested. Also expose a manual refresh button (🔄,
  localized `refresh_game_windows` hover) for an immediate, full re-scan that
  clears the selection.
- **Footers**: pin to the bottom of the panel with
  `ui.with_layout(egui::Layout::bottom_up(...), ...)`.
- **Theme entry point**: the app calls `theme::init(&ctx, pref)` once at
  startup (`RivuletApp::new`) and again whenever the preference changes in
  Settings. It applies the preference, loads the bundled Inter font
  (`assets/fonts/Inter-Regular.ttf` via `inter_font_definitions()`), and
  writes the palette visuals for **both** schemes (`set_visuals_of`), so a
  runtime switch keeps the branded look.
- **Status colors** (`theme::StatusColors::for_ui(ui)`, no custom hex unless
  necessary):
  - green — success / up-to-date
  - yellow — warning / paused
  - red — error
  - blue — informational state (e.g. muted, listening)
  - gray — secondary/hint text
  Each resolves per scheme (dark/light) and is WCAG-AA verified in
  `theme.rs` tests.
- **Structural palette** (`theme::ThemePalette::for_ui(ui)`):
  `accent`, `panel_fill`, `background`, `text`, `scrim`. Use the scrim for
  overlays that dim a captured preview (e.g. the region editor) instead of a
  hard-coded alpha color. Component fills and text colors should derive from
  this palette or `StatusColors`; image white is only valid as a texture tint.
- **Scene history**: expose undo/redo actions near scene management controls,
  disable them when unavailable, and keep `Ctrl+Z` / `Ctrl+Y` reserved for scene
  history unless a text field currently owns keyboard input.
- **Interaction feedback**: use `theme::accent_button` for ordinary buttons and
  `theme::paint_interaction_stroke` for custom/selectable controls. Focus gets
  the opaque accent stroke; hover uses
  the palette accent at 60% alpha and a 1.5 px stroke; active press uses the
  opaque accent and a 2 px stroke. Do not replace egui's default disabled
  visuals, focus indicators, or contrast behavior. These helpers are covered by
  unit tests in `theme.rs`.

## Naming conventions

- Rendering methods: `fn draw_<thing>(&mut self, ui: &mut egui::Ui)` (or
  `&egui::Context` for floating windows). See `draw_update_status`,
  `draw_region_editor`.
- Background work: `fn spawn_<action>(&self, ctx: egui::Context, ...)` — the
  method starts a thread, stores a shared state handle, and ends with
  `ctx.request_repaint()`. See `spawn_update_check`, `spawn_update_download`.
- UI-triggered actions consumed once per frame:
  `fn handle_<action>(&mut self, ctx: egui::Context)` — a `bool` flag is set
  by the button click and consumed here (see `handle_update_actions`).

## Localization (mandatory)

**Every user-visible string goes through the i18n layer** in
`rivulet-core/src/i18n.rs` — never hard-code UI text.

- `self.tr("key")` — plain translation
- `self.tr_fmt("key", &[arg1, arg2])` — positional `{0}`/`{1}` substitution
  (args are `String`)

Rules:

1. Add the key to **both** locales (`Locale::En`, `Locale::De`) at the same
   relative position — the test `all_locales_share_the_same_keys` enforces
   identical key sets *and order*.
2. Use semantic keys (`update_downloading`, `region_status`), not literal
   sentences.
3. Platform logs (stdout/stderr) stay in English; only GUI strings are
   localized.

## State management

- All UI state lives as fields on `RivuletApp`.
- Long-running operations (update check, download, install) follow the
  `UpdateUi` pattern: a `#[derive(Debug, Clone, PartialEq, Default)]` enum
  inside an `Arc<Mutex<...>>`, snapshotted per frame via
  `update_ui_snapshot()` and matched in `draw_*`. Busy states are derived with
  `matches!(...)`.
- Keep pure logic (formatting, geometry, state transitions) in plain
  functions/structs *outside* the egui code so it is unit-testable.

## Platform-specific UI

- Platform code is gated with `#[cfg(target_os = "windows")]` /
  `#[cfg(target_os = "linux")]` blocks *inside* the render method, with a
  fallback `#[cfg(not(any(...)))]` arm for unsupported platforms.
- Never break the build on one platform to serve another: the CI builds all
  three OSes, and the GUI compiles on macOS even where capture is absent.

## Async / busy UI

- Never block the UI thread: use `std::thread::spawn` + shared state
  (`Arc<Mutex<...>>` or atomics) + `ctx.request_repaint()` when done.
- Progress: `egui::ProgressBar::new(fraction)` with a human-readable text
  (see the update download bar; use the `format_bytes` helper for sizes).

## Testing conventions

Unit tests live in `#[cfg(test)] mod tests` at the bottom of `app.rs`
(pure-logic only — no UI automation). Existing categories to follow:

- scene history transitions (`SceneManager::undo` / `redo`, including redo
  invalidation after a new edit)
- `format_bytes` (size formatting)
- stalled-frames / no-frame-timeout logic (`should_abort_for_stalled_frames`,
  `--no-frame-timeout` argument parsing)
- frame-drain and error-receiver helpers
- skipped-filter warning formatting (`format_skipped_filters`)
- region geometry (`region_rect_in`, crop + drag-region math)

Add a test for any new pure function; the CI runs these on all three
platforms.

## Checklist for a new feature

- [ ] New section content is wrapped in its `AppView` variant (or, until the
      sidebar lands, placed below the recording controls with a separator).
- [ ] All strings added to `i18n.rs` in both locales, same relative order.
- [ ] No hard-coded text; colors from `StatusColors` / `ThemePalette`.
- [ ] Buttons use `theme::accent_button` and custom interactive controls use
      `theme::paint_interaction_stroke` for consistent hover/active feedback.
- [ ] `draw_*` for rendering, `spawn_*` for background work, `handle_*` for
      one-shot actions — no inline `thread::spawn` in the panel code.
- [ ] Long-running work updates a shared state enum and calls
      `ctx.request_repaint()`.
- [ ] Platform-specific code is `cfg`-gated with a fallback arm.
- [ ] Pure logic extracted and covered by a unit test in `app.rs`.
- [ ] `cargo fmt`, `cargo clippy -p rivulet-gui --no-deps` (no new warnings),
      `cargo test` green.
