use std::sync::atomic::AtomicBool;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalIndexWakeReason {
    Startup,
    ProviderChange,
    SourceTopology,
    ExplicitCommand,
    LifecycleResume,
    RecoveryRequired,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalIndexWaitResult {
    Notified(GlobalIndexWakeReason),
    TimedOut,
    Cancelled,
    Shutdown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GlobalIndexWakeSnapshot {
    pub notifications: u64,
    pub coalesced: u64,
    pub delivered: u64,
    pub waits: u64,
    pub blocked_waits: u64,
}

#[derive(Debug, Default)]
struct WakeState {
    pending: bool,
    shutdown: bool,
    last_reason: Option<GlobalIndexWakeReason>,
    snapshot: GlobalIndexWakeSnapshot,
}

/// One bounded process-local hint that Global Index durable/provider truth may
/// need re-evaluation. It carries no file data and is not a second queue.
#[derive(Debug, Default)]
pub(crate) struct GlobalIndexWakeSlot {
    state: Mutex<WakeState>,
    changed: Condvar,
}

impl GlobalIndexWakeSlot {
    pub(crate) fn notify(&self, reason: GlobalIndexWakeReason) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.shutdown {
            return;
        }
        state.snapshot.notifications = state.snapshot.notifications.saturating_add(1);
        if state.pending {
            state.snapshot.coalesced = state.snapshot.coalesced.saturating_add(1);
        } else {
            state.pending = true;
        }
        state.last_reason = Some(reason);
        self.changed.notify_one();
    }

    pub(crate) fn shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.shutdown = true;
            self.changed.notify_all();
        }
    }

    pub(crate) fn wait(
        &self,
        cancel: &AtomicBool,
        timeout: Option<Duration>,
    ) -> GlobalIndexWaitResult {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let Ok(mut state) = self.state.lock() else {
            return GlobalIndexWaitResult::Shutdown;
        };
        state.snapshot.waits = state.snapshot.waits.saturating_add(1);
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return GlobalIndexWaitResult::Cancelled;
            }
            if state.shutdown {
                return GlobalIndexWaitResult::Shutdown;
            }
            if state.pending {
                state.pending = false;
                state.snapshot.delivered = state.snapshot.delivered.saturating_add(1);
                return GlobalIndexWaitResult::Notified(
                    state
                        .last_reason
                        .take()
                        .unwrap_or(GlobalIndexWakeReason::ProviderChange),
                );
            }

            if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return GlobalIndexWaitResult::TimedOut;
                }
                state.snapshot.blocked_waits = state.snapshot.blocked_waits.saturating_add(1);
                let Ok((next_state, result)) = self.changed.wait_timeout(state, remaining) else {
                    return GlobalIndexWaitResult::Shutdown;
                };
                state = next_state;
                if result.timed_out() && !state.pending {
                    return GlobalIndexWaitResult::TimedOut;
                }
            } else {
                state.snapshot.blocked_waits = state.snapshot.blocked_waits.saturating_add(1);
                let Ok(next_state) = self.changed.wait(state) else {
                    return GlobalIndexWaitResult::Shutdown;
                };
                state = next_state;
            }
        }
    }

    #[cfg(any(test, all(feature = "native-qa", target_os = "macos")))]
    pub(crate) fn snapshot(&self) -> GlobalIndexWakeSnapshot {
        self.state
            .lock()
            .map(|state| state.snapshot)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{GlobalIndexWaitResult, GlobalIndexWakeReason, GlobalIndexWakeSlot};
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Arc};
    use std::thread;

    #[test]
    fn provider_signals_coalesce_into_one_delivery() {
        let wake = GlobalIndexWakeSlot::default();
        let cancel = AtomicBool::new(false);
        wake.notify(GlobalIndexWakeReason::Startup);
        wake.notify(GlobalIndexWakeReason::ProviderChange);
        wake.notify(GlobalIndexWakeReason::ProviderChange);

        assert_eq!(
            wake.wait(&cancel, None),
            GlobalIndexWaitResult::Notified(GlobalIndexWakeReason::ProviderChange)
        );
        assert_eq!(wake.snapshot().notifications, 3);
        assert_eq!(wake.snapshot().coalesced, 2);
        assert_eq!(wake.snapshot().delivered, 1);
    }

    #[test]
    fn signal_between_cycle_completion_and_wait_is_not_lost() {
        let wake = GlobalIndexWakeSlot::default();
        let cancel = AtomicBool::new(false);
        wake.notify(GlobalIndexWakeReason::Startup);
        assert_eq!(
            wake.wait(&cancel, None),
            GlobalIndexWaitResult::Notified(GlobalIndexWakeReason::Startup)
        );

        // This models a provider callback after the worker finished draining
        // durable state but immediately before it enters the idle wait.
        wake.notify(GlobalIndexWakeReason::ProviderChange);
        assert_eq!(
            wake.wait(&cancel, None),
            GlobalIndexWaitResult::Notified(GlobalIndexWakeReason::ProviderChange)
        );
        assert_eq!(wake.snapshot().delivered, 2);
    }

    #[test]
    fn blocked_wait_returns_after_a_coalesced_provider_signal() {
        let wake = Arc::new(GlobalIndexWakeSlot::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = mpsc::channel();
        let waiter = {
            let wake = wake.clone();
            let cancel = cancel.clone();
            thread::spawn(move || {
                started_tx.send(()).expect("waiter started");
                wake.wait(&cancel, None)
            })
        };
        started_rx.recv().expect("waiter started");
        wake.notify(GlobalIndexWakeReason::ProviderChange);
        assert_eq!(
            waiter.join().expect("join waiter"),
            GlobalIndexWaitResult::Notified(GlobalIndexWakeReason::ProviderChange)
        );
        assert_eq!(wake.snapshot().waits, 1);
        assert_eq!(wake.snapshot().delivered, 1);
    }

    #[test]
    fn shutdown_releases_an_idle_waiter() {
        let wake = Arc::new(GlobalIndexWakeSlot::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = mpsc::channel();
        let waiter = {
            let wake = wake.clone();
            let cancel = cancel.clone();
            thread::spawn(move || {
                started_tx.send(()).expect("waiter started");
                wake.wait(&cancel, None)
            })
        };
        started_rx.recv().expect("waiter started");
        wake.shutdown();
        assert_eq!(
            waiter.join().expect("join waiter"),
            GlobalIndexWaitResult::Shutdown
        );
    }
}
