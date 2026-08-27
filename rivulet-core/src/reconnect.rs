//! Per-target reconnect orchestration for streaming outputs.
use std::time::Duration;

use crate::stream::StreamTargetState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkBusEvent {
    Error,
    Eos,
    StateChanged,
    Warning,
}

impl SinkBusEvent {
    pub fn requires_reconnect(self) -> bool {
        matches!(self, Self::Error | Self::Eos)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    pub fn new(max_attempts: u32, initial_backoff: Duration, max_backoff: Duration) -> Self {
        Self {
            max_attempts,
            initial_backoff: initial_backoff.max(Duration::from_millis(100)),
            max_backoff: max_backoff.max(initial_backoff),
        }
    }

    pub fn backoff(&self, attempt: u32) -> Duration {
        let multiplier = 2u32.saturating_pow(attempt.saturating_sub(1));
        self.initial_backoff
            .saturating_mul(multiplier)
            .min(self.max_backoff)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetReconnectState {
    pub name: String,
    pub state: StreamTargetState,
    pub attempts: u32,
    pub next_backoff: Duration,
    pub retry_at: Option<Duration>,
}

impl TargetReconnectState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: StreamTargetState::Offline,
            attempts: 0,
            next_backoff: Duration::ZERO,
            retry_at: None,
        }
    }

    pub fn mark_connecting(&mut self) {
        self.state = StreamTargetState::Connecting;
        self.next_backoff = Duration::ZERO;
        self.retry_at = None;
    }

    pub fn mark_live(&mut self) {
        self.state = StreamTargetState::Live;
        self.attempts = 0;
        self.next_backoff = Duration::ZERO;
        self.retry_at = None;
    }

    pub fn mark_failed(&mut self, policy: &RetryPolicy) -> bool {
        self.attempts = self.attempts.saturating_add(1);
        self.next_backoff = policy.backoff(self.attempts);
        self.retry_at = Some(self.next_backoff);
        self.state = StreamTargetState::Failed;
        self.attempts <= policy.max_attempts
    }

    pub fn exhausted(&self, policy: &RetryPolicy) -> bool {
        self.attempts > policy.max_attempts
    }
}

#[derive(Debug, Clone)]
pub struct StreamingReconnectSupervisor {
    policy: RetryPolicy,
    targets: Vec<TargetReconnectState>,
}

impl StreamingReconnectSupervisor {
    pub fn new(names: impl IntoIterator<Item = String>, policy: RetryPolicy) -> Self {
        Self {
            policy,
            targets: names.into_iter().map(TargetReconnectState::new).collect(),
        }
    }

    pub fn policy(&self) -> RetryPolicy {
        self.policy
    }
    pub fn targets(&self) -> &[TargetReconnectState] {
        &self.targets
    }

    pub fn mark_connecting(&mut self, name: &str) -> bool {
        self.targets
            .iter_mut()
            .find(|target| target.name == name)
            .map(|target| {
                target.mark_connecting();
                true
            })
            .unwrap_or(false)
    }

    pub fn mark_live(&mut self, name: &str) -> bool {
        self.targets
            .iter_mut()
            .find(|target| target.name == name)
            .map(|target| {
                target.mark_live();
                true
            })
            .unwrap_or(false)
    }

    pub fn mark_failed(&mut self, name: &str) -> Option<bool> {
        self.targets
            .iter_mut()
            .find(|target| target.name == name)
            .map(|target| target.mark_failed(&self.policy))
    }

    pub fn handle_bus_event(&mut self, name: &str, event: SinkBusEvent) -> Option<bool> {
        if event.requires_reconnect() {
            self.mark_failed(name)
        } else if event == SinkBusEvent::StateChanged {
            self.mark_live(name);
            Some(true)
        } else {
            None
        }
    }

    /// Return targets whose backoff has elapsed at `now` and claim their retry.
    pub fn due_retries(&mut self, elapsed: Duration) -> Vec<String> {
        self.targets
            .iter_mut()
            .filter_map(|target| {
                let due = target.retry_at.filter(|delay| elapsed >= *delay).is_some();
                if due {
                    target.retry_at = None;
                    target.state = StreamTargetState::Connecting;
                    Some(target.name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn retryable_targets(&self) -> Vec<&str> {
        self.targets
            .iter()
            .filter(|target| {
                target.state == StreamTargetState::Failed && !target.exhausted(&self.policy)
            })
            .map(|target| target.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_and_bounded() {
        let policy = RetryPolicy::new(5, Duration::from_secs(1), Duration::from_secs(3));
        assert_eq!(policy.backoff(1), Duration::from_secs(1));
        assert_eq!(policy.backoff(2), Duration::from_secs(2));
        assert_eq!(policy.backoff(3), Duration::from_secs(3));
    }

    #[test]
    fn targets_are_isolated_during_retries() {
        let mut supervisor = StreamingReconnectSupervisor::new(
            vec!["healthy".into(), "broken".into()],
            RetryPolicy::default(),
        );
        supervisor.mark_live("healthy");
        assert_eq!(supervisor.mark_failed("broken"), Some(true));
        assert_eq!(supervisor.targets()[0].state, StreamTargetState::Live);
        assert_eq!(supervisor.retryable_targets(), vec!["broken"]);
    }

    #[test]
    fn successful_reconnect_resets_attempts() {
        let mut supervisor =
            StreamingReconnectSupervisor::new(vec!["target".into()], RetryPolicy::default());
        supervisor.mark_failed("target");
        supervisor.mark_live("target");
        assert_eq!(supervisor.targets()[0].attempts, 0);
        assert!(supervisor.retryable_targets().is_empty());
    }

    #[test]
    fn bus_events_only_fail_the_named_target() {
        let mut supervisor = StreamingReconnectSupervisor::new(
            vec!["one".into(), "two".into()],
            RetryPolicy::default(),
        );
        assert_eq!(
            supervisor.handle_bus_event("one", SinkBusEvent::Error),
            Some(true)
        );
        assert_eq!(supervisor.targets()[0].state, StreamTargetState::Failed);
        assert_eq!(supervisor.targets()[1].state, StreamTargetState::Offline);
        assert_eq!(
            supervisor.handle_bus_event("two", SinkBusEvent::Warning),
            None
        );
    }

    #[test]
    fn state_change_marks_target_live_and_resets_retry_state() {
        let mut supervisor =
            StreamingReconnectSupervisor::new(vec!["target".into()], RetryPolicy::default());
        supervisor.mark_failed("target");
        assert_eq!(
            supervisor.handle_bus_event("target", SinkBusEvent::StateChanged),
            Some(true)
        );
        assert_eq!(supervisor.targets()[0].state, StreamTargetState::Live);
        assert_eq!(supervisor.targets()[0].attempts, 0);
    }

    #[test]
    fn retry_stops_after_policy_limit() {
        let policy = RetryPolicy::new(2, Duration::from_secs(1), Duration::from_secs(10));
        let mut supervisor = StreamingReconnectSupervisor::new(vec!["target".into()], policy);
        assert_eq!(supervisor.mark_failed("target"), Some(true));
        assert_eq!(supervisor.mark_failed("target"), Some(true));
        assert_eq!(supervisor.mark_failed("target"), Some(false));
        assert!(supervisor.retryable_targets().is_empty());
    }

    #[test]
    fn due_retry_claims_only_elapsed_targets() {
        let policy = RetryPolicy::new(3, Duration::from_secs(2), Duration::from_secs(10));
        let mut supervisor =
            StreamingReconnectSupervisor::new(vec!["one".into(), "two".into()], policy);
        supervisor.mark_failed("one");
        supervisor.mark_failed("two");
        assert!(supervisor.due_retries(Duration::from_secs(1)).is_empty());
        assert_eq!(
            supervisor.due_retries(Duration::from_secs(2)),
            vec!["one", "two"]
        );
        assert_eq!(supervisor.targets()[0].state, StreamTargetState::Connecting);
    }
}
