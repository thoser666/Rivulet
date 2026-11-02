use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransitionKind {
    #[default]
    Cut,
    Fade,
}

impl TransitionKind {
    pub const ALL: [Self; 2] = [Self::Cut, Self::Fade];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Fade => "Fade",
        }
    }
}

/// A configured transition: the kind to play and how long it lasts.
///
/// Folder defaults and the collection default store this one type, so
/// [`crate::SceneManager::resolve_transition`] can hand a single resolved value
/// to the switch instead of a loose `(kind, duration)` pair that every call site
/// would have to assemble itself. The duration is kept even for a `Cut`, which
/// ignores it, so switching a folder back to `Fade` restores the length the user
/// typed instead of a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionDefault {
    pub kind: TransitionKind,
    /// Duration in milliseconds. Ignored by a `Cut`, which completes at once.
    pub duration_ms: u64,
}

impl Default for TransitionDefault {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration_ms: 300,
        }
    }
}

impl TransitionDefault {
    pub fn new(kind: TransitionKind, duration_ms: u64) -> Self {
        Self { kind, duration_ms }
    }

    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// The same transition with motion removed: a `Fade` becomes a `Cut`.
    ///
    /// Reduced motion is applied to the *resolved* transition rather than to
    /// the stored one, so a folder default keeps its configured kind and length
    /// and only the switch that plays it degrades (WCAG 2.3.3).
    pub fn without_motion(self) -> Self {
        Self {
            kind: TransitionKind::Cut,
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct SceneTransition {
    pub kind: TransitionKind,
    pub duration: Duration,
    pub from: Option<Uuid>,
    pub to: Option<Uuid>,
    started_at: Option<Instant>,
}

impl Default for SceneTransition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration: Duration::ZERO,
            from: None,
            to: None,
            started_at: None,
        }
    }
}

impl SceneTransition {
    pub fn new(kind: TransitionKind, duration: Duration) -> Self {
        Self {
            kind,
            duration,
            ..Self::default()
        }
    }

    /// Build a transition from an already-resolved default (#368).
    pub fn from_default(transition: TransitionDefault) -> Self {
        Self::new(transition.kind, transition.duration())
    }

    pub fn start(&mut self, from: Option<Uuid>, to: Option<Uuid>, now: Instant) {
        self.from = from;
        self.to = to;
        self.started_at = Some(now);
    }

    pub fn progress_at(&self, now: Instant) -> f32 {
        if self.kind == TransitionKind::Cut || self.duration.is_zero() {
            return 1.0;
        }
        let Some(started) = self.started_at else {
            return 0.0;
        };
        (now.saturating_duration_since(started).as_secs_f32() / self.duration.as_secs_f32())
            .clamp(0.0, 1.0)
    }

    pub fn is_finished_at(&self, now: Instant) -> bool {
        self.progress_at(now) >= 1.0
    }

    pub fn is_active(&self) -> bool {
        self.started_at.is_some()
    }

    pub fn clear(&mut self) {
        self.from = None;
        self.to = None;
        self.started_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_finishes_immediately() {
        let mut transition = SceneTransition::new(TransitionKind::Cut, Duration::from_secs(1));
        transition.start(Some(Uuid::new_v4()), Some(Uuid::new_v4()), Instant::now());
        assert_eq!(transition.progress_at(Instant::now()), 1.0);
        assert!(transition.is_finished_at(Instant::now()));
    }

    #[test]
    fn fade_progress_is_clamped_and_time_based() {
        let start = Instant::now();
        let mut transition =
            SceneTransition::new(TransitionKind::Fade, Duration::from_millis(1000));
        transition.start(None, Some(Uuid::new_v4()), start);
        assert_eq!(transition.progress_at(start), 0.0);
        assert!((transition.progress_at(start + Duration::from_millis(500)) - 0.5).abs() < 0.01);
        assert_eq!(transition.progress_at(start + Duration::from_secs(2)), 1.0);
    }

    #[test]
    fn zero_duration_fade_is_safe() {
        let transition = SceneTransition::new(TransitionKind::Fade, Duration::ZERO);
        assert_eq!(transition.progress_at(Instant::now()), 1.0);
    }

    #[test]
    fn clear_removes_transition_endpoints() {
        let mut transition = SceneTransition::default();
        transition.start(Some(Uuid::new_v4()), Some(Uuid::new_v4()), Instant::now());
        transition.clear();
        assert!(transition.from.is_none());
        assert!(transition.to.is_none());
    }

    /// A configured default is what a collection stores, so it has to survive a
    /// JSON round trip unchanged — including the duration of a `Cut`, which the
    /// renderer ignores but the user gets back when they switch to `Fade`.
    #[test]
    fn a_transition_default_round_trips_through_json() {
        let original = TransitionDefault::new(TransitionKind::Fade, 750);
        let json = serde_json::to_string(&original).expect("serialisable");
        assert_eq!(json, r#"{"kind":"fade","duration_ms":750}"#);
        let restored: TransitionDefault = serde_json::from_str(&json).expect("deserialisable");
        assert_eq!(restored, original);
        assert_eq!(restored.duration(), Duration::from_millis(750));
    }

    /// Reduced motion degrades the kind and keeps the configured length, so
    /// turning the preference back off restores exactly what the user set.
    #[test]
    fn without_motion_turns_a_fade_into_a_cut_and_keeps_the_length() {
        let fade = TransitionDefault::new(TransitionKind::Fade, 900);
        let still = fade.without_motion();
        assert_eq!(still.kind, TransitionKind::Cut);
        assert_eq!(still.duration_ms, 900);
        // A cut resolves to a cut either way, so the second call is a no-op.
        assert_eq!(still.without_motion(), still);
    }

    #[test]
    fn a_resolved_default_builds_a_matching_scene_transition() {
        let start = Instant::now();
        let mut transition =
            SceneTransition::from_default(TransitionDefault::new(TransitionKind::Fade, 400));
        transition.start(None, Some(Uuid::new_v4()), start);
        assert_eq!(transition.duration, Duration::from_millis(400));
        assert!((transition.progress_at(start + Duration::from_millis(200)) - 0.5).abs() < 0.01);
    }
}
