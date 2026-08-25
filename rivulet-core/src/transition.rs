use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
}
