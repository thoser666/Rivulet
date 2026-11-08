//! Per-target reconnect orchestration for streaming outputs.
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::stream::StreamTargetState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Event emitted by a stream sink's message bus.
pub enum SinkBusEvent {
    /// The sink failed fatally; the target has to be rebuilt.
    Error,
    /// The sink reached end-of-stream; the session is over.
    Eos,
    /// The sink announced a state change (for example playing).
    StateChanged,
    /// A non-fatal warning; the target keeps running.
    Warning,
}
impl SinkBusEvent {
    /// Whether this event ends the current sink session and needs a reconnect.
    pub fn requires_reconnect(self) -> bool {
        matches!(self, Self::Error | Self::Eos)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Bounded exponential backoff for reconnecting a single target.
pub struct RetryPolicy {
    /// Failed attempts allowed before the target is given up.
    pub max_attempts: u32,
    /// Delay before the first retry; clamped to at least 100 ms.
    pub initial_backoff: Duration,
    /// Upper bound for the exponential growth; never below `initial_backoff`.
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
/// Lower bound for any configured retry delay. A stream sink that fails
/// immediately (bad ingest URL, revoked stream key) would otherwise be retried
/// in a tight loop, burning CPU and flooding the platform with error events.
const MIN_BACKOFF: Duration = Duration::from_millis(100);
impl RetryPolicy {
    /// Build a policy with the delays clamped into a usable range.
    ///
    /// A 100 ms floor keeps an immediately failing sink from being retried in a
    /// tight loop, and the ceiling is raised to the floor so a lower ceiling
    /// cannot flatten the growth.
    pub fn new(max_attempts: u32, initial_backoff: Duration, max_backoff: Duration) -> Self {
        // Clamp first, then derive the ceiling from the clamped floor: a zero
        // `max_backoff` must never produce a zero backoff, and the ceiling must
        // never sit below the floor (which would silently flatten the
        // exponential growth to the ceiling from the very first attempt).
        let initial_backoff = initial_backoff.max(MIN_BACKOFF);
        Self {
            max_attempts,
            initial_backoff,
            max_backoff: max_backoff.max(initial_backoff),
        }
    }
    /// Delay before retry number `attempt` (1-based), doubling per attempt and
    /// clamped to [`RetryPolicy::max_backoff`]. Never returns zero.
    pub fn backoff(&self, attempt: u32) -> Duration {
        self.initial_backoff
            .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)))
            .min(self.max_backoff)
            .max(MIN_BACKOFF)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Reconnect bookkeeping for a single streaming target.
pub struct TargetReconnectState {
    /// Target name as configured by the user; the lookup key on the supervisor.
    pub name: String,
    /// Current lifecycle state of the target.
    pub state: StreamTargetState,
    /// Consecutive failed attempts since the last successful connection.
    pub attempts: u32,
    /// Delay computed for the most recent failure.
    pub next_backoff: Duration,
    /// Time since that failure at which the next retry becomes due, if one is scheduled.
    pub retry_at: Option<Duration>,
}
impl TargetReconnectState {
    /// Create a target in the offline state with no retry scheduled.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: StreamTargetState::Offline,
            attempts: 0,
            next_backoff: Duration::ZERO,
            retry_at: None,
        }
    }
    /// Mark the target as connecting and clear any scheduled retry.
    pub fn mark_connecting(&mut self) {
        self.state = StreamTargetState::Connecting;
        self.next_backoff = Duration::ZERO;
        self.retry_at = None;
    }
    /// Mark the target as live and reset the attempt counter.
    pub fn mark_live(&mut self) {
        self.state = StreamTargetState::Live;
        self.attempts = 0;
        self.next_backoff = Duration::ZERO;
        self.retry_at = None;
    }
    /// Record a failure and schedule the next retry.
    ///
    /// Returns whether the policy still allows another attempt.
    pub fn mark_failed(&mut self, policy: &RetryPolicy) -> bool {
        self.attempts = self.attempts.saturating_add(1);
        self.next_backoff = policy.backoff(self.attempts);
        self.retry_at = Some(self.next_backoff);
        self.state = StreamTargetState::Failed;
        self.attempts <= policy.max_attempts
    }
    /// Whether the policy's attempt budget is used up for this target.
    pub fn exhausted(&self, policy: &RetryPolicy) -> bool {
        self.attempts > policy.max_attempts
    }
}

#[derive(Debug, Clone)]
/// Per-target reconnect state machine covering every streaming output.
///
/// The supervisor owns timing and bookkeeping only; rebuilding a branch stays
/// with the caller, which can mutate the GStreamer pipeline safely.
pub struct StreamingReconnectSupervisor {
    policy: RetryPolicy,
    targets: Vec<TargetReconnectState>,
}
impl StreamingReconnectSupervisor {
    /// Create a supervisor tracking one target per given name under `policy`.
    pub fn new(names: impl IntoIterator<Item = String>, policy: RetryPolicy) -> Self {
        Self {
            policy,
            targets: names.into_iter().map(TargetReconnectState::new).collect(),
        }
    }
    /// Retry policy shared by all tracked targets.
    pub fn policy(&self) -> RetryPolicy {
        self.policy
    }
    /// Current state of every tracked target, in configuration order.
    pub fn targets(&self) -> &[TargetReconnectState] {
        &self.targets
    }
    /// Mark `name` as connecting; returns `false` for an unknown target.
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
    /// Mark `name` as live and reset its attempts; returns `false` for an unknown target.
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
    /// Record a failure for `name`.
    ///
    /// Returns whether another retry is allowed, or `None` when the target is unknown.
    pub fn mark_failed(&mut self, name: &str) -> Option<bool> {
        self.targets
            .iter_mut()
            .find(|t| t.name == name)
            .map(|t| t.mark_failed(&self.policy))
    }
    /// Apply a sink bus event to `name`.
    ///
    /// Errors and end-of-stream schedule a retry, a state change confirms the
    /// target is live, and warnings are ignored. Returns `None` when the event
    /// changes nothing or the target is unknown.
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

    /// Claim every target whose retry delay has elapsed by `elapsed`.
    ///
    /// Claimed targets are marked connecting and their names returned, so the
    /// caller rebuilds exactly those branches.
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

    /// Record a deterministic delay/reconnect observation for a target. The
    /// target remains failed until the pipeline owner explicitly confirms that
    /// its replacement sink is live.
    pub fn observe_retry_window(&self, name: &str, elapsed: Duration) -> Option<bool> {
        self.targets
            .iter()
            .find(|target| target.name == name)
            .map(|target| target.retry_at.is_some_and(|retry_at| elapsed >= retry_at))
    }
    /// Names of the targets that are failed and still within the attempt budget.
    pub fn retryable_targets(&self) -> Vec<&str> {
        self.targets
            .iter()
            .filter(|t| t.state == StreamTargetState::Failed && !t.exhausted(&self.policy))
            .map(|t| t.name.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Command emitted by a [`ReconnectWorker`] for the pipeline owner.
pub enum ReconnectCommand {
    /// Rebuild the branch of the named target.
    RebuildBranch(String),
    /// Stop the worker without emitting a rebuild.
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
    /// Start a worker that requests a branch rebuild for `name` after `delay`.
    ///
    /// The rebuild is skipped when [`ReconnectWorker::cancel`] runs, or the
    /// worker is dropped, before the delay elapses.
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
    /// Take the next queued command without blocking.
    pub fn try_recv(&self) -> Option<ReconnectCommand> {
        self.commands.try_recv().ok()
    }

    /// Drain all commands currently queued without blocking.
    pub fn drain(&self) -> Vec<ReconnectCommand> {
        self.commands.try_iter().collect()
    }
    /// Cancel a pending rebuild and join the worker thread.
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
    use crate::test_helpers::wait_until;

    #[test]
    fn backoff_is_exponential_and_bounded() {
        let p = RetryPolicy::new(5, Duration::from_secs(1), Duration::from_secs(3));
        assert_eq!(p.backoff(1), Duration::from_secs(1));
        assert_eq!(p.backoff(2), Duration::from_secs(2));
        assert_eq!(p.backoff(3), Duration::from_secs(3));
    }
    #[test]
    fn zero_max_backoff_cannot_flatten_the_backoff_to_zero() {
        // Regression: `max_backoff` used to be clamped against the *unclamped*
        // `initial_backoff`, so a zero ceiling produced a zero backoff and the
        // supervisor retried a dead sink in a tight loop.
        let p = RetryPolicy::new(5, Duration::ZERO, Duration::ZERO);
        assert_eq!(p.initial_backoff, Duration::from_millis(100));
        assert_eq!(p.max_backoff, Duration::from_millis(100));
        for attempt in 1..=5 {
            assert_eq!(p.backoff(attempt), Duration::from_millis(100));
        }
    }
    #[test]
    fn ceiling_below_the_clamped_floor_is_raised_to_the_floor() {
        // A 50 ms ceiling under a 100 ms floor must not flatten the growth: the
        // first attempt waits the floor, not the (lower) requested ceiling.
        let p = RetryPolicy::new(5, Duration::ZERO, Duration::from_millis(50));
        assert_eq!(p.max_backoff, Duration::from_millis(100));
        assert_eq!(p.backoff(1), Duration::from_millis(100));
        assert_eq!(p.backoff(4), Duration::from_millis(100));
    }
    #[test]
    fn backoff_never_reports_zero_for_degenerate_attempts() {
        let p = RetryPolicy::new(5, Duration::from_millis(1), Duration::from_secs(30));
        for attempt in [0, 1, 2, 31, 32, u32::MAX] {
            assert!(
                p.backoff(attempt) >= Duration::from_millis(100),
                "attempt {attempt} produced {:?}",
                p.backoff(attempt)
            );
        }
        // Growth still doubles until the ceiling, then saturates there.
        assert_eq!(p.backoff(1), Duration::from_millis(100));
        assert_eq!(p.backoff(2), Duration::from_millis(200));
        assert_eq!(p.backoff(64), Duration::from_secs(30));
    }
    #[test]
    fn a_zero_backoff_never_leaves_the_target_immediately_due() {
        // The user-visible consequence of the zero-backoff bug: the very next
        // poll would already see the retry as due.
        let p = RetryPolicy::new(3, Duration::ZERO, Duration::ZERO);
        let mut s = StreamingReconnectSupervisor::new(vec!["target".into()], p);
        assert_eq!(s.mark_failed("target"), Some(true));
        let retry_at = s.targets()[0].retry_at.expect("retry scheduled");
        assert!(
            retry_at >= Duration::from_millis(100),
            "retry scheduled after {retry_at:?}"
        );
        assert!(s.due_retries(Duration::ZERO).is_empty());
        assert_eq!(s.due_retries(retry_at), vec!["target"]);
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
    fn retry_window_is_observable_without_mutating_target_state() {
        let mut supervisor = StreamingReconnectSupervisor::new(
            vec!["target".into()],
            RetryPolicy::new(2, Duration::from_secs(2), Duration::from_secs(5)),
        );
        supervisor.mark_failed("target");
        assert_eq!(
            supervisor.observe_retry_window("target", Duration::from_secs(1)),
            Some(false)
        );
        assert_eq!(
            supervisor.observe_retry_window("target", Duration::from_secs(2)),
            Some(true)
        );
        assert_eq!(supervisor.targets()[0].state, StreamTargetState::Failed);
        assert_eq!(
            supervisor.observe_retry_window("missing", Duration::from_secs(2)),
            None
        );
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
        // Poll instead of sleeping a fixed amount: under parallel test load the
        // worker thread can be starved, so a one-shot try_recv after a fixed
        // sleep is flaky. Wait up to several seconds for the command.
        let cmd = wait_until(Duration::from_secs(5), || worker.try_recv())
            .expect("worker never emitted RebuildBranch");
        assert_eq!(cmd, ReconnectCommand::RebuildBranch("target".into()));
    }
}
