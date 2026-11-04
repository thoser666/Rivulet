# Scene management

How Rivulet organizes scenes into folders, colours and reusable components, and
what the model guarantees. Tracked by issue
[#319](https://github.com/thoser666/Rivulet/issues/319).

All four delivery packages of #319 are implemented: the model, the scene dock
and its search. See [Delivery packages](#delivery-packages) for what shipped in
which package. For the user-facing walkthrough — building folders, colour
labels, components, drag & drop and search — see
[scene-management-tutorial.md](scene-management-tutorial.md).

## Why this exists

OBS users organize productions with scene folders, colour labels and reusable
sub-scenes. A scene list that only ever grows flat stops being usable at roughly
a dozen scenes: the streamer scrolls during a live show instead of switching.
Rivulet's model already carried `parent` and `color` per scene, but nothing
enforced the hierarchy, nothing distinguished a sub-scene from a scene the
streamer switches to, and the GUI rendered a flat list.

The work is therefore split: first make the model correct and testable, then
build the dock on top of it. This document is the contract between the two.

## Model

`rivulet_core::Scene` is the unit of the hierarchy:

| Field | Type | Meaning |
|---|---|---|
| `id` | `Uuid` | Stable identity, also the key of every binding that references the scene. |
| `name` | `String` | Display name; not unique. |
| `parent` | `Option<Uuid>` | The containing folder scene, or `None` for a top-level scene. |
| `color` | `u32` | UI colour label, ARGB (`0xFF1A73E8` = opaque blue). `0xFFFFFFFF` is the neutral default. |
| `kind` | `SceneKind` | `Primary` (live-switching list) or `Component` (reusable building block). |

A folder is not a separate type. It is an ordinary scene that happens to have
children, exactly like a directory that also holds files. This keeps export,
import, undo and the source bindings unchanged: a folder can still be switched
to, and it still has its own sources.

### Primary and component scenes

`SceneKind::Primary` is the default and matches every scene written before this
change. `SceneKind::Component` marks a reusable building block — an intro
layout, a BRB composition, a "just the chat" panel — that belongs to the
collection but should not inflate the live-switching list. A component is still
switchable, still exportable and still a legal folder parent; the distinction is
a presentation hint, not an access rule.

`Scene::component(name)` constructs one directly; `Scene::is_component()` reads
the flag back.

### Hierarchy rules

`SceneManager::set_parent(scene, parent)` is the only supported way to move a
scene, and it refuses anything that would produce a tree the UI cannot render:

| Situation | Result |
|---|---|
| Unknown scene | `SceneHierarchyError::UnknownScene` |
| Unknown parent | `SceneHierarchyError::UnknownParent` |
| A scene as its own parent | `SceneHierarchyError::SelfParent` |
| Parent is inside the moved scene's own subtree | `SceneHierarchyError::Cycle` |
| `parent = None` | Moves the scene back to the top level |

A refused move leaves the collection untouched — validation runs before the
snapshot is taken, so undo does not get an empty step either.

`SceneManager::remove(id)` does not orphan children: they are lifted to the
removed scene's own parent. Deleting `Show/Game` therefore leaves `Chat` under
`Show` rather than making it a scene with a parent that no longer exists.

`SceneManager::duplicate_scene` keeps the source's `parent` and `kind`, so
duplicating inside a folder does not eject the copy to the top level.

### Presentation order and broken input

`SceneManager::ordered_scenes()` returns every scene ID depth-first, with
children directly after their parent. Two properties matter because collections
are files users can edit:

- **Every scene appears exactly once.** A scene whose `parent` no longer exists
  is treated as a root instead of disappearing.
- **A cycle cannot hang or truncate the walk.** The traversal keeps a visited
  set, and a cycle that is unreachable from any root is emitted afterwards, so
  hand-edited or merged collections stay inspectable and repairable instead of
  making the dock loop.

`SceneManager::is_descendant_of` (used by the cycle check) is step-bounded for
the same reason.

## Persistence and compatibility

`SceneManager::export_json` / `import_json` are the on-disk and clipboard
format. Two compatibility guarantees hold:

- **Collections written before the `kind` field load unchanged.** `kind` is
  `#[serde(default)]`, so a missing field means `Primary`. Existing users must
  never be stranded by an added field.
- **Dangling parents and cycles are tolerated on import.** Rejecting them would
  turn a slightly broken file into an unopenable project; the model repairs the
  presentation instead.

Changing `parent` and `color` goes through `set_parent`, `set_color` and
`set_kind`, all of which are covered by the undo/redo history like `add`,
`rename` and `remove`.

## Folder defaults and transitions

Transitions used to be one global setting: a cut between two game scenes and a
fade from the intro into the show shared a duration, and the workaround was to
change the global transition before every switch during a live show. A folder's
default ([#368](https://github.com/thoser666/Rivulet/issues/368)) applies to
everything inside it instead.

It deliberately does **not** become a field on `Scene`: `Scene` is a public
struct a consumer can construct literally, so a new field would break every such
caller under [`api-migration.md`](api-migration.md). The value lives in the
collection as `folder_transitions` on `SceneManager`, keyed by the folder
scene's ID. It is exported with the collection, `#[serde(default)]` so a
collection written before the feature loads unchanged, and omitted from the JSON
while nothing is configured — an untouched collection exports exactly as it did
before.

Resolution order for a switch to `to`, written once in
`SceneManager::resolve_transition`:

| Step | Source |
|---|---|
| 1 | An override on the target scene itself — reserved for a later extension; per-scene overrides do not exist yet |
| 2 | The nearest ancestor folder of `to` that has a default, walking outwards |
| 3 | The collection default (the transition controls next to the dock) |
| 4 | The application default, which the collection default is initialised from |

An entry belongs to the folder's **contents**: a folder switched to directly is
a scene like any other and inherits from its own ancestors. That keeps one rule
— "the folder says how what is inside it switches" — instead of two, and it is
the grain OBS users expect from a folder rather than from a per-scene override.

`set_folder_transition` and `clear_folder_transition` are undoable like
`set_color`. Deleting a folder prunes its entry (the delete itself is undoable,
so the default returns with the folder), while folding one does not touch it:
fold state lives in the dock and never in the collection. Studio mode resolves
the same way, so a `Take` plays the preview scene's folder default; a `Cut`
completes at the instant it starts instead of showing a half-faded frame, and a
`Fade` uses the folder's duration rather than the global one. Reduced motion
still degrades the *runtime* transition to a cut without changing what the
collection stores. The walk is step-bounded, so a hand-edited collection whose
parents form a cycle falls back instead of hanging the switch.

## Scene dock

The dock (package C) renders the hierarchy on top of that model — in the Scenes
view and again in the Live workspace, both through the same `draw_scene_dock`, so
switching, selection and search behave identically in the two places. Its rules are
implemented in `rivulet_core::scene_dock`, which keeps them free of a window so
they can be unit-tested; the GUI in `rivulet-gui/src/app.rs` only draws rows and
forwards actions to the model API.

| Concern | Rule |
|---|---|
| Indentation | A row is indented by the number of existing ancestors above it. A dangling parent counts as a root, so hand-edited collections still render, and the depth walk is step-bounded so a cycle cannot loop. |
| Collapse | Every scene with children gets a disclosure control. The fold state is **per collection** and persisted with the app settings — not on `Scene` — so folding never dirties the collection or creates an undo step. |
| Live and preview | The scene on air and the studio preview scene keep their ancestor chain open even when a folder is folded, so a collapsed dock still shows what is live. |
| Colour labels | A named palette (`SceneColorLabel`) written through `set_color`. The neutral default is drawn as an outline rather than a fill; every other entry reaches WCAG AA 3:1 against both theme backgrounds, and the label name is always available as text, so colour is never the only signal. |
| Drag & drop | `apply_drop` validates with `can_drop` before the manager snapshots, then calls `set_parent`. A refused move — self-parent, unknown target, or a parent inside the dropped scene's own subtree — is highlighted as invalid while dragging and explained in the status line on release, and changes nothing. |
| Components | `set_kind` toggles a scene between primary and component. Components and everything below them are rendered in a separate, dimmed group but stay switchable. |

### Multi-select and bulk actions

Issue [#366](https://github.com/thoser666/Rivulet/issues/366) adds a selection on
top of the tree. The rules live in `rivulet_core::scene_dock::SceneSelection` and
the bulk mutations in `SceneManager`, so click/toggle/extend and the
all-or-nothing move are testable without a window.

| Concern | Rule |
|---|---|
| Click | A plain click selects that row alone and switches to it — the previous single-row behaviour. |
| Toggle | `Ctrl`/`Cmd`-click adds or removes one row and makes it the anchor. Removing the anchor leaves none, so the next shift-click starts a fresh range instead of guessing. |
| Range | `Shift`-click selects from the anchor to the clicked row over the rows **currently drawn**. A folded folder's children are not reachable by a range until it is open; without a usable anchor (none, or filtered away) the click degrades to a plain selection. |
| Keyboard | `Ctrl`/`Cmd+A` selects the visible rows, `Esc` clears the selection, `Delete` removes it. All three sit behind the text-input guard, so they keep their editing meaning in rename and chat fields; inside the Scenes view a non-empty selection owns `Delete` instead of the `delete_source` hotkey. |
| Dragging | A drag that starts on a selected row moves the **whole** selection, in one undo step. Dragging an unselected row never sweeps the selection along. |
| Atomicity | The batch is validated before anything moves. One member that would violate the hierarchy (self-parent, own subtree, unknown target) refuses the whole drop, changes nothing, and the status line names that scene. Dropping a selection onto one of its own members is refused for the same reason. |
| Colour and kind | The selection's `…` menu applies a colour label or `SceneKind` to every selected scene as one undo step. |
| Delete | Removes every selected scene as one undo step with the existing child-lifting rule; more than one scene is confirmed in a dialog that names the count. |
| Filtering | **The selection is sticky.** A filter can hide selected rows but never deselects them: the status line counts the hidden ones and bulk actions include them, so a filtered dock cannot silently act on half the selection. `Ctrl`/`Cmd+A` is the deliberate exception — it selects only what is visible, because "all" there means what the user can see. |
| Visuals | Selected rows get a tick and a bold name; the active scene keeps its *Active* badge and the studio program scene its *Live* badge, so colour is never the only signal and the three states cannot be confused. |

Search and filtering are delivered as package D (see
[Search and filter](#search-and-filter)).

### Search and filter

Package D ([#351](https://github.com/thoser666/Rivulet/issues/351)) adds the
search box and filters. The rules live in `rivulet_core::scene_dock` next to the
tree rules, so they are testable without a window:

| Concern | Rule |
|---|---|
| Matching | Case-insensitive on the scene name, against the trimmed query. `Match anywhere` is the default; `Starts with` only accepts a prefix. |
| Expansion | A match force-opens its ancestor chain **and** its own subtree for that render only. The persisted fold state is never mutated, so clearing the query restores exactly the fold state the user had, and a matched folder shows what is inside it. |
| Filters | The colour label narrows the rows and counts as a match for expansion. Component scenes can be hidden independently of the query, together with everything below them. |
| No matches | An active filter with an empty result says so (`scenes_search_no_matches`) instead of showing an empty dock. |
| Keyboard | `Ctrl/Cmd+F` focuses the box, `Esc` clears the query and keeps the focus, `Enter` switches to the first match in depth-first order; hidden components are skipped. |
| Persistence | Match mode, component toggle and colour filter are saved with the app settings; the query is not. |


## Live composition

The active scene is not only an authoring surface: it is composed into the
frames that reach the encoder, so recording and streaming show what the scene
dock says is live. The toggle sits next to **Save scene snapshot** in the
Scenes view and under the picture in the Live workspace; it is on by default
and persisted with the settings.

`rivulet-core/src/live_compositor.rs` owns the rule, so it is testable without a
window:

| Concern | Rule |
|---|---|
| Canvas | Every composed frame is 1920x1080, the scene editor's coordinate space. Layer transforms therefore mean the same thing the editor shows, and the encoder sees one stable resolution. |
| Capture layers | `ScreenCapture`, `GameCapture` and `Webcam` layers receive the picture the capture pipeline just produced, whatever its native size. |
| Browser layers | Receive the newest browser-source frame (`BrowserSource::latest_frame`). |
| Other kinds | `Image`, `Text`, `Media`, `Color` and `Audio` keep their stable layout tile until their platform renderers exist ([#217](https://github.com/thoser666/Rivulet/issues/217)); they are never silently blanked. |
| Empty scene | A scene with no visual layer — none at all, or only audio layers — returns `None` and the raw capture frame is recorded unchanged, so a misconfigured scene cannot black out a show. |
| Cost | Composition runs on the CPU once per captured frame. Unrotated layers — every live capture and browser layer — take a row-wise nearest-neighbour path instead of the rotation math, and an unrotated 1:1 layer copies rows directly. |

Recording resolution follows the canvas while composition is on: a 1440p
monitor is composed into 1080p. Turn the toggle off to record the capture's
native resolution.

Tests: `rivulet-core/src/live_compositor.rs` pins the layer-to-frame mapping,
the empty-scene fallback, the canvas size and the tile fallback for unsupported
kinds; `rivulet-core/src/scene_snapshot.rs` compares the axis-aligned fast path
against the general mapping; `rivulet-gui/src/app.rs` covers the toggle, the
passthrough and the composed output; the `ci_pinning` guard
`live_scene_composition_reaches_the_output` fails if the capture paths stop
going through the compositor or the docs lose this section.

## Migrating an OBS collection

The OBS scene collection JSON stores each scene with an optional group
association (`group` / `id` pair on the OBS side). Migration maps:

| OBS | Rivulet |
|---|---|
| Scene | `Scene` with `kind = Primary` |
| Group | `Scene` whose children point at it via `parent` |
| Nested group | `parent` points at the enclosing group's scene |
| Scene without a group | `parent = None` |

The importer is not part of this package; the mapping is documented here so the
GUI work and any future importer agree on one shape. The importer itself is
[#367](https://github.com/thoser666/Rivulet/issues/367).

## Delivery packages

| Package | Content | Status |
|---|---|---|
| **A. Spec and linking** | This document, README/quality-gate links, cross-references to #294–#297 | done |
| **B. Core model** | `SceneKind`, `set_parent`/`set_color`/`set_kind`, cycle guard, `remove` reparenting, `duplicate_scene` fixes, compatibility + regression tests | done |
| **C. Scene dock** | Tree with indentation, collapse state per collection, colour swatch and picker, drag & drop to re-parent, component toggle | done ([#350](https://github.com/thoser666/Rivulet/issues/350)) |
| **D. Search and filter** | Real-time search with "match anywhere"/"starts with", auto-expand of matches, component filter, persisted collapse state | done ([#351](https://github.com/thoser666/Rivulet/issues/351)) |
| **E. Multi-select and bulk actions** | Selection with click/toggle/range over the drawn rows, atomic bulk move, bulk colour, kind and delete as single undo steps, sticky filter semantics | done ([#366](https://github.com/thoser666/Rivulet/issues/366)) |

Packages C and D were tracked as separate issues
([#350](https://github.com/thoser666/Rivulet/issues/350),
[#351](https://github.com/thoser666/Rivulet/issues/351)) so the dock could be
reviewed as UI work and the model as engine work. Both are closed.

## Tests

`rivulet-core/tests/scene_selection.rs` pins the selection rules of
[#366](https://github.com/thoser666/Rivulet/issues/366): click/toggle/extend over
a folded tree, the sticky filter (hidden rows stay selected and counted),
anchors that survive or move with filtering, the atomic bulk move (including a
refusal that names the member and a selection dropped onto itself), one undo
step for colour, kind and delete, child-lifting and transition pruning on bulk
delete, and unknown IDs being skipped.

`rivulet-core/tests/scene.rs` pins the invariants above:

- primary/component defaults and constructors,
- moves, top-level re-parenting, self-parent and cycle refusal,
- unknown scene/parent reporting without side effects,
- lifting children when their folder is removed,
- duplication keeping folder and kind,
- undo for colour and kind,
- export/import round-trip of folder, colour and kind,
- loading a collection written before `kind` existed,
- dangling parent and cyclic import staying visible, unique and terminating.

`rivulet-core/tests/scene_dock.rs` covers the dock on top of that model:

- indentation for nested folders and the unchanged flat order for root scenes,
- folding a subtree, and the live/preview scene staying reachable inside a
  folded folder,
- fold state per collection, its serialization round-trip and pruning of
  removed scenes,
- a dangling parent and a cycle still rendering every row exactly once,
- legal drops (into a folder, back to the top level, undoable) and the refused
  ones (self-parent, unknown target, own subtree) with their messages in both
  locales,
- the component grouping and the palette's contrast on both theme backgrounds.
- search and filter: anywhere vs. starts-with, case-insensitivity, a match
  inside a folded folder (with the fold state asserted unchanged afterwards),
  a matched folder opening its subtree, the component and colour filters, the
  explicit no-match state, `first_match` order and the persisted match mode.

No GStreamer, hardware or filesystem access is involved; the suites run in
milliseconds.

## Related work

- [#294](https://github.com/thoser666/Rivulet/issues/294) — studio mode
  (preview/program), which builds on the same scene identity.
- [#295](https://github.com/thoser666/Rivulet/issues/295) — scene transitions.
- [#296](https://github.com/thoser666/Rivulet/issues/296) — scene snapshots.
- [#297](https://github.com/thoser666/Rivulet/issues/297) — exit evidence.
- [`feature-detection.md`](feature-detection.md) — how `FeatureInfo` and
  `CaptureBackendInfo` prepare the modern scene surface.
