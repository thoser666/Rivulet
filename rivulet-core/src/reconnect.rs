//! Per-target reconnect orchestration for streaming outputs.
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
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
        self.initial_backoff
            .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
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
            .find(|t| t.name == name)
            .map(|t| {
                t.mark_connecting();
                true
            })
            .unwrap_or(false)
    }
    pub fn mark_live(&mut self, name: &str) -> bool {
        self.targets
            .iter_mut()
            .find(|t| t.name == name)
            .map(|t| {
                t.mark_live();
                true
            })
            .unwrap_or(false)
    }
    pub fn mark_failed(&mut self, name: &str) -> Option<bool> {
        self.targets
            .iter_mut()
            .find(|t| t.name == name)
            .map(|t| t.mark_failed(&self.policy))
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
    /// Mark a target as failed only once while its retry is pending. This
    /// prevents duplicate bus messages from multiplying retry workers.
    pub fn mark_failed_once(&mut self, name: &str) -> Option<bool> {
        self.targets
            .iter_mut()
            .find(|target| target.name == name)
            .map(|target| {
                if target.retry_at.is_some() || target.state == StreamTargetState::Connecting {
                    false
                } else {
                    target.mark_failed(&self.policy)
                }
            })
    }

    pub fn due_retries(&mut self, elapsed: Duration) -> Vec<String> {
        self.targets
            .iter_mut()
            .filter_map(|t| {
                if t.retry_at.is_some_and(|at| elapsed >= at) {
                    t.mark_connecting();
                    Some(t.name.clone())
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn retryable_targets(&self) -> Vec<&str> {
        self.targets
            .iter()
            .filter(|t| t.state == StreamTargetState::Failed && !t.exhausted(&self.policy))
            .map(|t| t.name.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectCommand {
    RebuildBranch(String),
    Shutdown,
}

/// Resolve a GStreamer sink name (`stream_sink_<index>`) to the configured
/// target index. Unknown element names are ignored instead of affecting the
/// whole pipeline.
pub fn target_index_from_sink_name(name: &str) -> Option<usize> {
    name.strip_prefix("stream_sink_")?.parse().ok()
}

/// A small worker that owns timing only; pipeline mutation stays with the
/// caller, which can rebuild branches on the GStreamer context safely.
pub struct ReconnectWorker {
    commands: Receiver<ReconnectCommand>,
    stop: Sender<ReconnectCommand>,
    handle: Option<thread::JoinHandle<()>>,
}
impl ReconnectWorker {
    pub fn start(name: impl Into<String>, delay: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let target = name.into();
        let handle = thread::spawn(move || {
            if stop_rx.recv_timeout(delay).is_err() {
                let _ = tx.send(ReconnectCommand::RebuildBranch(target));
            }
        });
        Self {
            commands: rx,
            stop: stop_tx,
            handle: Some(handle),
        }
    }
    pub fn try_recv(&self) -> Option<ReconnectCommand> {
        self.commands.try_recv().ok()
    }

    /// Drain all commands currently queued without blocking.
    pub fn drain(&self) -> Vec<ReconnectCommand> {
        self.commands.try_iter().collect()
    }
    pub fn cancel(&mut self) {
        let _ = self.stop.send(ReconnectCommand::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for ReconnectWorker {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_is_exponential_and_bounded() {
        let p = RetryPolicy::new(5, Duration::from_secs(1), Duration::from_secs(3));
        assert_eq!(p.backoff(1), Duration::from_secs(1));
        assert_eq!(p.backoff(2), Duration::from_secs(2));
        assert_eq!(p.backoff(3), Duration::from_secs(3));
    }
    #[test]
    fn targets_are_isolated_during_retries() {
        let mut s = StreamingReconnectSupervisor::new(
            vec!["healthy".into(), "broken".into()],
            RetryPolicy::default(),
        );
        s.mark_live("healthy");
        assert_eq!(s.mark_failed("broken"), Some(true));
        assert_eq!(s.targets()[0].state, StreamTargetState::Live);
        assert_eq!(s.retryable_targets(), vec!["broken"]);
    }
    #[test]
    fn successful_reconnect_resets_attempts() {
        let mut s =
            StreamingReconnectSupervisor::new(vec!["target".into()], RetryPolicy::default());
        s.mark_failed("target");
        s.mark_live("target");
        assert_eq!(s.targets()[0].attempts, 0);
    }
    #[test]
    fn bus_events_only_fail_named_target() {
        let mut s = StreamingReconnectSupervisor::new(
            vec!["one".into(), "two".into()],
            RetryPolicy::default(),
        );
        assert_eq!(s.handle_bus_event("one", SinkBusEvent::Error), Some(true));
        assert_eq!(s.targets()[1].state, StreamTargetState::Offline);
        assert_eq!(s.handle_bus_event("two", SinkBusEvent::Warning), None);
    }
    #[test]
    fn state_change_marks_live() {
        let mut s =
            StreamingReconnectSupervisor::new(vec!["target".into()], RetryPolicy::default());
        s.mark_failed("target");
        assert_eq!(
            s.handle_bus_event("target", SinkBusEvent::StateChanged),
            Some(true)
        );
        assert_eq!(s.targets()[0].state, StreamTargetState::Live);
    }
    #[test]
    fn retry_stops_after_limit() {
        let mut s = StreamingReconnectSupervisor::new(
            vec!["target".into()],
            RetryPolicy::new(2, Duration::from_secs(1), Duration::from_secs(10)),
        );
        assert_eq!(s.mark_failed("target"), Some(true));
        assert_eq!(s.mark_failed("target"), Some(true));
        assert_eq!(s.mark_failed("target"), Some(false));
    }
    #[test]
    fn due_retry_claims_elapsed_target() {
        let mut s = StreamingReconnectSupervisor::new(
            vec!["target".into()],
            RetryPolicy::new(3, Duration::from_secs(1), Duration::from_secs(3)),
        );
        s.mark_failed("target");
        assert!(s.due_retries(Duration::from_millis(500)).is_empty());
        assert_eq!(s.due_retries(Duration::from_secs(1)), vec!["target"]);
        assert_eq!(s.targets()[0].state, StreamTargetState::Connecting);
    }
    #[test]
    fn duplicate_failures_do_not_schedule_duplicate_retries() {
        let mut supervisor =
            StreamingReconnectSupervisor::new(vec!["target".into()], RetryPolicy::default());
        assert_eq!(supervisor.mark_failed_once("target"), Some(true));
        assert_eq!(supervisor.mark_failed_once("target"), Some(false));
        assert_eq!(supervisor.targets()[0].attempts, 1);
    }

    #[test]
    fn sink_name_resolution_is_strict_and_target_local() {
        assert_eq!(target_index_from_sink_name("stream_sink_2"), Some(2));
        assert_eq!(target_index_from_sink_name("stream_sink_x"), None);
        assert_eq!(target_index_from_sink_name("other_sink_2"), None);
    }

    #[test]
    fn worker_emits_branch_rebuild_after_backoff() {
        let worker = ReconnectWorker::start("target", Duration::from_millis(10));
        thread::sleep(Duration::from_millis(30));
        assert_eq!(
            worker.try_recv(),
            Some(ReconnectCommand::RebuildBranch("target".into()))
        );
    }
}
