//! Dock-level presentation state for the scene tree (issue #350).
//!
//! This is package C of [#319](https://github.com/thoser666/Rivulet/issues/319):
//! the rendering rules the scene dock needs, separated from the GUI so they can
//! be tested without opening a window.
//!
//! [`crate::SceneManager`] owns the hierarchy; this module owns everything the
//! dock draws on top of it and keeps that logic free of a window, so the tree
//! shape, the collapse filtering and the drop rules are unit-testable:
//!
//! - [`dock_rows`] turns the manager's depth-first order into indented rows,
//!   folding away the subtrees of collapsed folders. Depth is computed
//!   defensively: a hand-edited collection with a dangling parent or a cycle
//!   still renders every scene exactly once and terminates.
//! - [`SceneDock`] is the **persisted** collapse state, keyed by collection
//!   name. Collapsing is a view concern: it must not become a field on
//!   [`crate::Scene`], an undo step or a dirty collection.
//! - [`apply_drop`] is the only call the dock makes for a drag & drop move.
//!   It validates first via [`can_drop`] and returns the same typed refusals
//!   as [`crate::SceneManager::set_parent`], so a rejected drop can be shown
//!   on screen instead of silently doing nothing.
//! - [`SceneColorLabel`] is the named palette behind `Scene::color`: a few
//!   named defaults instead of a colour wheel, each legible as a swatch on
//!   both the dark and the light theme background.
//!
//! The contract is documented in `docs/scene-management.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Scene, SceneHierarchyError, SceneKind, SceneManager};

/// A named colour label offered by the dock's picker.
///
/// `Scene::color` stores an ARGB `u32`; the palette is the finite set of
/// values the picker writes. [`SceneColorLabel::Neutral`] is the default
/// ("no label") and is drawn as an outline rather than a filled swatch, so it
/// is exempt from the minimum contrast the other entries have to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SceneColorLabel {
    /// The default `Scene::color` (opaque white): no colour label.
    Neutral,
    /// Blue, the value used in the scene-management spec examples.
    Blue,
    /// Green.
    Green,
    /// Purple.
    Purple,
    /// Amber.
    Amber,
    /// Red.
    Red,
    /// Teal.
    Teal,
}

impl SceneColorLabel {
    /// Every palette entry in picker order.
    pub const ALL: [SceneColorLabel; 7] = [
        SceneColorLabel::Neutral,
        SceneColorLabel::Blue,
        SceneColorLabel::Green,
        SceneColorLabel::Purple,
        SceneColorLabel::Amber,
        SceneColorLabel::Red,
        SceneColorLabel::Teal,
    ];

    /// The ARGB value this label writes into `Scene::color`.
    pub fn argb(self) -> u32 {
        match self {
            SceneColorLabel::Neutral => 0xFFFF_FFFF,
            SceneColorLabel::Blue => 0xFF1A_73E8,
            SceneColorLabel::Green => 0xFF1E_8E3E,
            SceneColorLabel::Purple => 0xFFAB_47BC,
            SceneColorLabel::Amber => 0xFFA6_6A00,
            SceneColorLabel::Red => 0xFFD3_2F2F,
            SceneColorLabel::Teal => 0xFF0E_7C86,
        }
    }

    /// i18n key naming this label (see [`crate::Locale::tr`]).
    pub fn key(self) -> &'static str {
        match self {
            SceneColorLabel::Neutral => "scenes_color_neutral",
            SceneColorLabel::Blue => "scenes_color_blue",
            SceneColorLabel::Green => "scenes_color_green",
            SceneColorLabel::Purple => "scenes_color_purple",
            SceneColorLabel::Amber => "scenes_color_amber",
            SceneColorLabel::Red => "scenes_color_red",
            SceneColorLabel::Teal => "scenes_color_teal",
        }
    }

    /// Whether this entry means "no colour label".
    pub fn is_neutral(self) -> bool {
        self == SceneColorLabel::Neutral
    }

    /// The palette entry matching a stored ARGB value, if the value came from
    /// the palette. A colour a user hand-edited into the collection returns
    /// `None` and is still shown as its raw swatch.
    pub fn from_argb(argb: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|label| label.argb() == argb)
    }
}

/// One line of the scene dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneDockRow {
    /// The scene this row selects and switches to.
    pub id: Uuid,
    /// Indentation level, `0` for a top-level scene.
    pub depth: usize,
    /// Whether the scene contains other scenes and therefore gets a
    /// disclosure control.
    pub has_children: bool,
    /// Whether the row belongs to a component's subtree — the scene is a
    /// component itself or sits below one. The dock renders those in the
    /// separate components group, dimmed.
    pub component: bool,
}

/// How the dock's search box matches a scene name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SceneSearchMode {
    /// Match the query anywhere in the name (the default).
    #[default]
    Anywhere,
    /// Match only names that start with the query.
    StartsWith,
}

impl SceneSearchMode {
    /// Every mode in picker order.
    pub const ALL: [SceneSearchMode; 2] = [SceneSearchMode::Anywhere, SceneSearchMode::StartsWith];

    /// i18n key naming this mode (see [`crate::Locale::tr`]).
    pub fn key(self) -> &'static str {
        match self {
            SceneSearchMode::Anywhere => "scenes_search_mode_anywhere",
            SceneSearchMode::StartsWith => "scenes_search_mode_starts_with",
        }
    }
}

/// The dock's search and filter state (issue #351).
///
/// The query is deliberately **not** persisted: a restart starts with the full
/// list. The match mode, the component toggle and the colour filter are,
/// because they are standing preferences rather than a one-off action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneFilterState {
    /// Text in the dock's search box.
    #[serde(skip)]
    pub query: String,
    /// How the query is matched against a scene name.
    #[serde(default)]
    pub mode: SceneSearchMode,
    /// Whether component scenes are listed. `true` (the default) keeps the
    /// package-C layout: components appear in their own dimmed group.
    #[serde(default = "show_components_by_default")]
    pub show_components: bool,
    /// List only scenes carrying this colour label (`None` lists every colour).
    #[serde(default)]
    pub color: Option<SceneColorLabel>,
}

/// Serde default for [`SceneFilterState::show_components`].
fn show_components_by_default() -> bool {
    true
}

impl Default for SceneFilterState {
    fn default() -> Self {
        Self {
            query: String::new(),
            mode: SceneSearchMode::default(),
            show_components: show_components_by_default(),
            color: None,
        }
    }
}

impl SceneFilterState {
    /// An empty filter: no query, no colour, components listed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the query contains anything but whitespace.
    pub fn has_query(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// Whether the filter can hide rows at all.
    pub fn is_active(&self) -> bool {
        self.has_query() || self.color.is_some()
    }

    /// Drop the query, keeping the mode and both filters.
    pub fn clear_query(&mut self) {
        self.query.clear();
    }

    /// Whether `scene` passes the filter.
    ///
    /// The name match is case-insensitive and compares against the trimmed
    /// query, so a trailing space at the end of typing does not empty the list.
    pub fn matches(&self, scene: &Scene) -> bool {
        if !self.matches_name(&scene.name) {
            return false;
        }
        match self.color {
            None => true,
            Some(label) => SceneColorLabel::from_argb(scene.color) == Some(label),
        }
    }

    /// Case-insensitive name match in the selected [`SceneSearchMode`].
    fn matches_name(&self, name: &str) -> bool {
        let query = self.query.trim();
        if query.is_empty() {
            return true;
        }
        let needle = query.to_lowercase();
        let haystack = name.to_lowercase();
        match self.mode {
            SceneSearchMode::Anywhere => haystack.contains(&needle),
            SceneSearchMode::StartsWith => haystack.starts_with(&needle),
        }
    }
}

/// The rows the dock shows for `manager`, in the manager's depth-first order.
///
/// `collapsed` folds away the subtree of the listed scenes; `always_visible`
/// names scenes that must stay reachable regardless (the dock passes the live
/// and the preview scene, so a collapsed folder can never hide what is on
/// air) — their whole ancestor chain is kept open.
///
/// Every scene appears at most once, and a collection with a dangling parent
/// or a cycle does not make this loop or drop a scene.
pub fn dock_rows(
    manager: &SceneManager,
    collapsed: &BTreeSet<Uuid>,
    always_visible: &[Uuid],
) -> Vec<SceneDockRow> {
    dock_rows_filtered(
        manager,
        collapsed,
        always_visible,
        &SceneFilterState::default(),
    )
}

/// Like [`dock_rows`], but applies the dock's search and filter state.
///
/// Search expansion is **transient**: a matching scene force-opens its
/// ancestor chain and its own subtree for this call only. `collapsed` is never
/// mutated, so clearing the query restores exactly the fold state the user had,
/// and a matched folder still shows what is inside it.
pub fn dock_rows_filtered(
    manager: &SceneManager,
    collapsed: &BTreeSet<Uuid>,
    always_visible: &[Uuid],
    filter: &SceneFilterState,
) -> Vec<SceneDockRow> {
    let total = manager.scenes().len();
    let parents_with_children: HashSet<Uuid> = manager
        .scenes()
        .iter()
        .filter_map(|scene| scene.parent)
        .collect();
    let mut forced_open: HashSet<Uuid> = HashSet::new();
    for id in always_visible {
        collect_ancestors(manager, *id, total, &mut forced_open);
    }
    // A search keeps the matches, their ancestor chains (context, so a match
    // inside a folded folder is reachable) and the subtrees of the matches (a
    // matched folder has to show what is inside it). All of this is local to
    // this call: `collapsed` stays exactly as the user left it.
    let search_visible = search_visibility(manager, filter, total);
    // `matches` treats an empty query as "everything", so the expansion must
    // stay behind `is_active` — otherwise an unfiltered dock would force every
    // folder open and folding would stop working.
    if filter.is_active() {
        for scene in manager.scenes() {
            if filter.matches(scene) {
                forced_open.insert(scene.id);
                collect_ancestors(manager, scene.id, total, &mut forced_open);
            }
        }
    }

    let mut component_flags: HashMap<Uuid, bool> = HashMap::with_capacity(total);
    let mut rows = Vec::with_capacity(total);
    for id in manager.ordered_scenes() {
        let Some(scene) = manager.get(id) else {
            continue;
        };
        // A component infects its subtree: everything below a component is
        // part of that reusable building block and belongs in the same group.
        // Ordered depth-first, the parent's flag is already known here; a
        // scene whose parent comes later (a corrupt cycle) falls back to its
        // own kind.
        let component = scene.kind == SceneKind::Component
            || scene
                .parent
                .and_then(|parent| component_flags.get(&parent))
                .copied()
                .unwrap_or(false);
        component_flags.insert(id, component);
        if !filter.show_components && component {
            continue;
        }
        if filter.is_active() && !search_visible.contains(&id) {
            continue;
        }
        if hidden_by_collapse(manager, id, collapsed, &forced_open, total) {
            continue;
        }
        rows.push(SceneDockRow {
            id,
            depth: depth_of(manager, id, total),
            has_children: parents_with_children.contains(&id),
            component,
        });
    }
    rows
}

/// The persisted collapse state of the scene dock, keyed by collection name.
///
/// Collapse is a view concern, so it lives next to the app settings rather
/// than on a scene: folding a folder must not create an undo step or dirty
/// the collection, and each collection keeps its own fold state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneDock {
    #[serde(default)]
    collapsed: BTreeMap<String, BTreeSet<Uuid>>,
}

impl SceneDock {
    /// An empty dock state (every folder expanded).
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `id` is collapsed in `collection`.
    pub fn is_collapsed(&self, collection: &str, id: Uuid) -> bool {
        self.collapsed_in(collection).contains(&id)
    }

    /// The collapsed scene IDs of `collection` (empty when nothing is folded).
    pub fn collapsed_in(&self, collection: &str) -> &BTreeSet<Uuid> {
        static EMPTY: BTreeSet<Uuid> = BTreeSet::new();
        self.collapsed.get(collection).unwrap_or(&EMPTY)
    }

    /// Fold or unfold `id` in `collection`.
    pub fn set_collapsed(&mut self, collection: &str, id: Uuid, collapsed: bool) {
        let entry = self.collapsed.entry(collection.to_owned()).or_default();
        if collapsed {
            entry.insert(id);
        } else {
            entry.remove(&id);
        }
        if entry.is_empty() {
            self.collapsed.remove(collection);
        }
    }

    /// Flip the fold state of `id` in `collection`.
    pub fn toggle(&mut self, collection: &str, id: Uuid) {
        let collapsed = self.is_collapsed(collection, id);
        self.set_collapsed(collection, id, !collapsed);
    }

    /// Drop folded IDs that no longer exist in `collection`, so a collection
    /// that had scenes removed does not carry stale state around forever.
    pub fn retain_existing(&mut self, collection: &str, existing: &BTreeSet<Uuid>) {
        let Some(entry) = self.collapsed.get_mut(collection) else {
            return;
        };
        entry.retain(|id| existing.contains(id));
        if entry.is_empty() {
            self.collapsed.remove(collection);
        }
    }
}

/// Whether `id` may be moved under `parent` (`None` = top level).
///
/// Validates without touching the collection, so the dock can highlight a
/// legal drop target and explain an illegal one while the pointer is still
/// moving. The rules are exactly the ones
/// [`crate::SceneManager::set_parent`] enforces.
pub fn can_drop(
    manager: &SceneManager,
    id: Uuid,
    parent: Option<Uuid>,
) -> Result<(), SceneHierarchyError> {
    if manager.get(id).is_none() {
        return Err(SceneHierarchyError::UnknownScene(id));
    }
    let Some(parent_id) = parent else {
        return Ok(());
    };
    if parent_id == id {
        return Err(SceneHierarchyError::SelfParent(id));
    }
    if manager.get(parent_id).is_none() {
        return Err(SceneHierarchyError::UnknownParent(parent_id));
    }
    if is_descendant_of(manager, parent_id, id) {
        return Err(SceneHierarchyError::Cycle {
            scene: id,
            parent: parent_id,
        });
    }
    Ok(())
}

/// Move `id` under `parent` for a drag & drop action.
///
/// Refuses exactly what [`can_drop`] refuses, before the manager takes its
/// undo snapshot: a refused drop leaves the collection — and the undo history
/// — unchanged.
pub fn apply_drop(
    manager: &mut SceneManager,
    id: Uuid,
    parent: Option<Uuid>,
) -> Result<(), SceneHierarchyError> {
    can_drop(manager, id, parent)?;
    manager.set_parent(id, parent)
}

/// Whether `candidate` sits anywhere below `ancestor` in the tree, found with
/// a step-bounded walk up the parent chain so a corrupt cycle terminates.
fn is_descendant_of(manager: &SceneManager, candidate: Uuid, ancestor: Uuid) -> bool {
    let total = manager.scenes().len();
    let mut current = manager.get(candidate).and_then(|scene| scene.parent);
    let mut seen = HashSet::new();
    seen.insert(candidate);
    let mut steps = 0;
    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        if !seen.insert(parent) || steps > total {
            break;
        }
        steps += 1;
        current = manager.get(parent).and_then(|scene| scene.parent);
    }
    false
}

/// Indentation level of `id`: the number of existing ancestors above it.
///
/// Terminates on a dangling parent (the scene counts as a root) and on a
/// cycle (the walk stops at the first repeat).
fn depth_of(manager: &SceneManager, id: Uuid, total: usize) -> usize {
    let mut depth = 0;
    let mut seen = HashSet::new();
    seen.insert(id);
    let mut current = manager.get(id).and_then(|scene| scene.parent);
    while let Some(parent) = current {
        if !seen.insert(parent) || depth > total {
            break;
        }
        let Some(parent_scene) = manager.get(parent) else {
            break;
        };
        depth += 1;
        current = parent_scene.parent;
    }
    depth
}

/// Whether every ancestor above `id` is expanded.
///
/// A row that is not reachable from a root — a hand-edited cycle — ignores the
/// fold state entirely. Inside a cycle every scene is its neighbour's child, so
/// honouring the folds would hide all of them at once and leave a dock the user
/// cannot unfold; showing them is what makes the collection repairable.
fn hidden_by_collapse(
    manager: &SceneManager,
    id: Uuid,
    collapsed: &BTreeSet<Uuid>,
    forced_open: &HashSet<Uuid>,
    total: usize,
) -> bool {
    let mut chain: Vec<Uuid> = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(id);
    let mut current = manager.get(id).and_then(|scene| scene.parent);
    let mut steps = 0;
    while let Some(parent) = current {
        if !seen.insert(parent) || steps > total {
            // A cycle: keep the row visible (see the doc comment above).
            return false;
        }
        steps += 1;
        let Some(parent_scene) = manager.get(parent) else {
            // A dangling parent counts as a root, so the row is foldable.
            break;
        };
        chain.push(parent);
        current = parent_scene.parent;
    }
    chain
        .iter()
        .any(|parent| collapsed.contains(parent) && !forced_open.contains(parent))
}

/// Collect every existing ancestor of `id` into `out`.
fn collect_ancestors(manager: &SceneManager, id: Uuid, total: usize, out: &mut HashSet<Uuid>) {
    let mut seen = HashSet::new();
    seen.insert(id);
    let mut current = manager.get(id).and_then(|scene| scene.parent);
    let mut steps = 0;
    while let Some(parent) = current {
        if !seen.insert(parent) || steps > total {
            break;
        }
        steps += 1;
        let Some(parent_scene) = manager.get(parent) else {
            break;
        };
        out.insert(parent);
        current = parent_scene.parent;
    }
}

/// i18n key explaining a refused move, for the inline hint the dock shows
/// instead of failing silently.
pub fn drop_error_key(error: &SceneHierarchyError) -> &'static str {
    match error {
        SceneHierarchyError::UnknownScene(_) | SceneHierarchyError::UnknownParent(_) => {
            "scene_hierarchy_unknown"
        }
        SceneHierarchyError::SelfParent(_) => "scene_hierarchy_self",
        SceneHierarchyError::Cycle { .. } => "scene_hierarchy_cycle",
    }
}

/// The scene `Enter` switches to: the first scene in the manager's depth-first
/// order that the filter matches.
///
/// Folding is irrelevant here, because [`dock_rows_filtered`] opens the
/// ancestor chain of every match: the scene this returns is exactly the one the
/// dock reveals. Hidden components are skipped — switching to a row the dock
/// does not show would be surprising. Returns `None` while the filter is
/// inactive, which keeps `Enter` inert in an unfiltered dock.
pub fn first_match(manager: &SceneManager, filter: &SceneFilterState) -> Option<Uuid> {
    if !filter.is_active() {
        return None;
    }
    let mut component_flags: HashMap<Uuid, bool> = HashMap::new();
    for id in manager.ordered_scenes() {
        let Some(scene) = manager.get(id) else {
            continue;
        };
        let component = scene.kind == SceneKind::Component
            || scene
                .parent
                .and_then(|parent| component_flags.get(&parent))
                .copied()
                .unwrap_or(false);
        component_flags.insert(id, component);
        if !filter.show_components && component {
            continue;
        }
        if filter.matches(scene) {
            return Some(id);
        }
    }
    None
}

/// The rows a search keeps: every match, the ancestor chain of every match and
/// the whole subtree of every match.
///
/// Ancestors are kept as context — without them a match inside a folded folder
/// would have no parent row to sit under — and descendants are kept because a
/// matched folder is only useful if it shows what is inside it.
fn search_visibility(
    manager: &SceneManager,
    filter: &SceneFilterState,
    total: usize,
) -> HashSet<Uuid> {
    let mut visible: HashSet<Uuid> = HashSet::new();
    if !filter.is_active() {
        return visible;
    }
    let mut matched: HashSet<Uuid> = HashSet::new();
    for scene in manager.scenes() {
        if filter.matches(scene) {
            matched.insert(scene.id);
            collect_ancestors(manager, scene.id, total, &mut visible);
        }
    }
    for scene in manager.scenes() {
        if matched.contains(&scene.id) || has_matched_ancestor(manager, scene.id, &matched, total) {
            visible.insert(scene.id);
        }
    }
    visible
}

/// Whether `id` or one of its ancestors is in `matched`.
///
/// Step-bounded and cycle-safe, like the other tree walks in this module.
fn has_matched_ancestor(
    manager: &SceneManager,
    id: Uuid,
    matched: &HashSet<Uuid>,
    total: usize,
) -> bool {
    if matched.contains(&id) {
        return true;
    }
    let mut seen = HashSet::new();
    seen.insert(id);
    let mut current = manager.get(id).and_then(|scene| scene.parent);
    let mut steps = 0;
    while let Some(parent) = current {
        if matched.contains(&parent) {
            return true;
        }
        if !seen.insert(parent) || steps > total {
            break;
        }
        steps += 1;
        current = manager.get(parent).and_then(|scene| scene.parent);
    }
    false
}
