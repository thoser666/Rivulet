# Scene management

How Rivulet organizes scenes into folders, colours and reusable components, and
what the model guarantees. Tracked by issue
[#319](https://github.com/thoser666/Rivulet/issues/319).

The first part of #319 — this document plus the scene model it describes — is
implemented. The scene dock (tree rendering, drag & drop, search) follows in
separate packages; see [Delivery packages](#delivery-packages).

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

## Scene dock

The dock (package C) renders the hierarchy on top of that model. Its rules are
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

The dock deliberately does not search or filter (package D) and does not
multi-select.

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
GUI work and any future importer agree on one shape.

## Delivery packages

| Package | Content | Status |
|---|---|---|
| **A. Spec and linking** | This document, README/quality-gate links, cross-references to #294–#297 | done |
| **B. Core model** | `SceneKind`, `set_parent`/`set_color`/`set_kind`, cycle guard, `remove` reparenting, `duplicate_scene` fixes, compatibility + regression tests | done |
| **C. Scene dock** | Tree with indentation, collapse state per collection, colour swatch and picker, drag & drop to re-parent, component toggle | done ([#350](https://github.com/thoser666/Rivulet/issues/350)) |
| **D. Search and filter** | Real-time search with "match anywhere"/"starts with", auto-expand of matches, component filter, persisted collapse state | planned ([#351](https://github.com/thoser666/Rivulet/issues/351)) |

Packages C and D are tracked as separate issues ([#350](https://github.com/thoser666/Rivulet/issues/350),
[#351](https://github.com/thoser666/Rivulet/issues/351)) so the dock can be
reviewed as UI work and the model as engine work.

## Tests

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
