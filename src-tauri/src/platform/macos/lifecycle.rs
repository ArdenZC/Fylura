//! macOS workspace lifecycle coordination.
//!
//! AppKit owns sleep, wake, mount, unmount, and volume-change notifications.
//! This adapter turns those notifications into one bounded state machine so
//! existing durable workers can pause and resume without creating a second
//! reconciliation authority or polling the filesystem.

use super::run_loop::NativeRunLoopStopSignal;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
#[cfg(target_os = "macos")]
use std::thread;
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacLifecycleEvent {
    WillSleep,
    DidWake,
    DidMount,
    WillUnmount,
    DidUnmount,
    VolumeChanged,
    ResourcePolicyChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacLifecycleState {
    Active,
    Suspended,
    ReconcileRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacLifecycleSnapshot {
    pub state: MacLifecycleState,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct MacLifecycleController {
    state: Arc<Mutex<MacLifecycleSnapshot>>,
    stopped: Arc<AtomicBool>,
    stop_signal: Arc<NativeRunLoopStopSignal>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl MacLifecycleController {
    pub fn start<F>(on_event: F) -> Result<Self, String>
    where
        F: Fn(MacLifecycleEvent) -> Result<(), String> + Send + Sync + 'static,
    {
        let controller = Self {
            state: Arc::new(Mutex::new(MacLifecycleSnapshot {
                state: MacLifecycleState::Active,
                last_error: None,
            })),
            stopped: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(NativeRunLoopStopSignal::default()),
            worker: Arc::new(Mutex::new(None)),
        };

        #[cfg(target_os = "macos")]
        {
            let state = Arc::clone(&controller.state);
            let stopped = Arc::clone(&controller.stopped);
            let stop_signal = Arc::clone(&controller.stop_signal);
            let callback: Arc<dyn Fn(MacLifecycleEvent) -> Result<(), String> + Send + Sync> =
                Arc::new(on_event);
            let worker = thread::Builder::new()
                .name("fylura-macos-lifecycle".to_string())
                .spawn(move || {
                    objc2::rc::autoreleasepool(|_| {
                        run_workspace_observer(
                            &state,
                            &stopped,
                            &stop_signal,
                            &callback,
                            #[cfg(test)]
                            &|_, _| {},
                        );
                    });
                })
                .map_err(|error| format!("macos_lifecycle_thread_start_failed: {error}"))?;
            if let Ok(mut slot) = controller.worker.lock() {
                *slot = Some(worker);
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = on_event;
        }

        Ok(controller)
    }

    pub fn snapshot(&self) -> MacLifecycleSnapshot {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or(MacLifecycleSnapshot {
                state: MacLifecycleState::ReconcileRequired,
                last_error: Some("macos_lifecycle_state_unavailable".to_string()),
            })
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.stop_signal.request_stop();
        if let Ok(mut slot) = self.worker.lock() {
            if let Some(worker) = slot.take() {
                let _ = worker.join();
            }
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn apply_event(
        state: &Arc<Mutex<MacLifecycleSnapshot>>,
        callback: &dyn Fn(MacLifecycleEvent) -> Result<(), String>,
        event: MacLifecycleEvent,
    ) {
        if event == MacLifecycleEvent::ResourcePolicyChanged {
            let _ = callback(event);
            return;
        }
        #[cfg(target_os = "macos")]
        if matches!(
            event,
            MacLifecycleEvent::DidMount
                | MacLifecycleEvent::WillUnmount
                | MacLifecycleEvent::DidUnmount
                | MacLifecycleEvent::VolumeChanged
        ) {
            crate::platform::macos::strategy::invalidate_source_retirement_capability_cache();
            crate::platform::macos::file_provider::invalidate_materialized_provider_items();
        }
        if let Ok(mut snapshot) = state.lock() {
            snapshot.last_error = None;
            snapshot.state = match event {
                MacLifecycleEvent::WillSleep => MacLifecycleState::Suspended,
                MacLifecycleEvent::DidWake
                | MacLifecycleEvent::DidMount
                | MacLifecycleEvent::WillUnmount
                | MacLifecycleEvent::DidUnmount
                | MacLifecycleEvent::VolumeChanged => MacLifecycleState::ReconcileRequired,
                MacLifecycleEvent::ResourcePolicyChanged => unreachable!(),
            };
        }

        let result = callback(event);
        if let Ok(mut snapshot) = state.lock() {
            match result {
                Ok(())
                    if matches!(
                        event,
                        MacLifecycleEvent::DidWake
                            | MacLifecycleEvent::DidMount
                            | MacLifecycleEvent::DidUnmount
                            | MacLifecycleEvent::VolumeChanged
                    ) =>
                {
                    snapshot.state = MacLifecycleState::Active;
                }
                Ok(()) => {}
                Err(error) => {
                    snapshot.state = MacLifecycleState::ReconcileRequired;
                    snapshot.last_error = Some(error);
                }
            }
        }
    }
}

impl Drop for MacLifecycleController {
    fn drop(&mut self) {
        if Arc::strong_count(&self.worker) == 1 {
            self.stop();
        }
    }
}

#[cfg(any(target_os = "macos", test))]
struct ObserverCleanup<F: FnOnce()> {
    cleanup: Option<F>,
}

#[cfg(any(target_os = "macos", test))]
impl<F: FnOnce()> ObserverCleanup<F> {
    fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }
}

#[cfg(any(target_os = "macos", test))]
impl<F: FnOnce()> Drop for ObserverCleanup<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

#[cfg(target_os = "macos")]
fn run_workspace_observer(
    state: &Arc<Mutex<MacLifecycleSnapshot>>,
    stopped: &AtomicBool,
    stop_signal: &NativeRunLoopStopSignal,
    callback: &Arc<dyn Fn(MacLifecycleEvent) -> Result<(), String> + Send + Sync>,
    #[cfg(test)] hook: &dyn Fn(&str, Option<&super::run_loop::LifecycleRunLoop>),
) {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidMountNotification, NSWorkspaceDidRenameVolumeNotification,
        NSWorkspaceDidUnmountNotification, NSWorkspaceDidWakeNotification,
        NSWorkspaceWillSleepNotification, NSWorkspaceWillUnmountNotification,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSProcessInfo,
        NSProcessInfoPowerStateDidChangeNotification,
        NSProcessInfoThermalStateDidChangeNotification,
    };
    use std::ptr::NonNull;

    let workspace = NSWorkspace::sharedWorkspace();
    let center: Retained<NSNotificationCenter> = workspace.notificationCenter();
    let state_for_block = Arc::clone(state);
    let callback_for_block = Arc::clone(callback);
    let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
        let notification = unsafe { notification.as_ref() };
        let name = notification.name().to_string();
        let event = if name == unsafe { NSWorkspaceWillSleepNotification }.to_string() {
            Some(MacLifecycleEvent::WillSleep)
        } else if name == unsafe { NSWorkspaceDidWakeNotification }.to_string() {
            Some(MacLifecycleEvent::DidWake)
        } else if name == unsafe { NSWorkspaceDidMountNotification }.to_string() {
            Some(MacLifecycleEvent::DidMount)
        } else if name == unsafe { NSWorkspaceWillUnmountNotification }.to_string() {
            Some(MacLifecycleEvent::WillUnmount)
        } else if name == unsafe { NSWorkspaceDidUnmountNotification }.to_string() {
            Some(MacLifecycleEvent::DidUnmount)
        } else if name == unsafe { NSWorkspaceDidRenameVolumeNotification }.to_string() {
            Some(MacLifecycleEvent::VolumeChanged)
        } else {
            None
        };
        if let Some(event) = event {
            MacLifecycleController::apply_event(&state_for_block, &*callback_for_block, event);
        }
    });

    let observer =
        unsafe { center.addObserverForName_object_queue_usingBlock(None, None, None, &block) };
    // Read once before subscription as required by NSProcessInfo's thermal
    // state contract, then subscribe to both native policy-change signals.
    let process_info = NSProcessInfo::processInfo();
    let _initial_activity = (
        process_info.thermalState(),
        process_info.isLowPowerModeEnabled(),
    );
    let process_center: Retained<NSNotificationCenter> = NSNotificationCenter::defaultCenter();
    let policy_callback = Arc::clone(callback);
    let policy_state = Arc::clone(state);
    let policy_block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        MacLifecycleController::apply_event(
            &policy_state,
            &*policy_callback,
            MacLifecycleEvent::ResourcePolicyChanged,
        );
    });
    let power_observer = unsafe {
        process_center.addObserverForName_object_queue_usingBlock(
            Some(NSProcessInfoPowerStateDidChangeNotification),
            None,
            None,
            &policy_block,
        )
    };
    let thermal_observer = unsafe {
        process_center.addObserverForName_object_queue_usingBlock(
            Some(NSProcessInfoThermalStateDidChangeNotification),
            None,
            None,
            &policy_block,
        )
    };
    let _observer_cleanup = ObserverCleanup::new(|| {
        let protocol_object: &ProtocolObject<dyn objc2_foundation::NSObjectProtocol> =
            observer.as_ref();
        let observer_object: &AnyObject = protocol_object.as_ref();
        unsafe { center.removeObserver(observer_object) };
        for observer in [&power_observer, &thermal_observer] {
            let protocol_object: &ProtocolObject<dyn objc2_foundation::NSObjectProtocol> =
                observer.as_ref();
            let observer_object: &AnyObject = protocol_object.as_ref();
            unsafe { process_center.removeObserver(observer_object) };
        }
        #[cfg(test)]
        hook("cleaned", None);
    });

    // A change between the initial snapshot and observer registration is
    // covered by this fresh scheduler re-evaluation.
    MacLifecycleController::apply_event(
        state,
        &**callback,
        MacLifecycleEvent::ResourcePolicyChanged,
    );
    #[cfg(test)]
    hook("registered", None);
    let run_loop = match super::run_loop::LifecycleRunLoop::new() {
        Ok(run_loop) => run_loop,
        Err(error) => {
            record_observer_failure(state, error);
            return;
        }
    };
    stop_signal.install(run_loop.stop_action());
    // Clear cross-thread retained handles before removing the owned source;
    // observer RAII cleanup follows both on every return path.
    let _stop_cleanup = ObserverCleanup::new(|| stop_signal.clear());
    #[cfg(test)]
    hook("installed", Some(&run_loop));
    if !stopped.load(Ordering::Acquire) {
        #[cfg(test)]
        hook("before_run", Some(&run_loop));
        run_loop.run();
        if !stopped.load(Ordering::Acquire) {
            record_observer_failure(state, "macos_lifecycle_run_loop_exited_unexpectedly");
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn record_observer_failure(state: &Mutex<MacLifecycleSnapshot>, error: &str) {
    eprintln!("{error}");
    if let Ok(mut snapshot) = state.lock() {
        snapshot.state = MacLifecycleState::ReconcileRequired;
        snapshot.last_error = Some(error.to_string());
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "lifecycle_native_tests.rs"]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::{
        MacLifecycleController, MacLifecycleEvent, MacLifecycleSnapshot, MacLifecycleState,
        ObserverCleanup,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    };

    fn test_controller() -> MacLifecycleController {
        MacLifecycleController {
            state: Arc::new(Mutex::new(MacLifecycleSnapshot {
                state: MacLifecycleState::Active,
                last_error: None,
            })),
            stopped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stop_signal: Arc::new(super::NativeRunLoopStopSignal::default()),
            worker: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn stop_before_run_loop_install_is_preserved() {
        let controller = test_controller();
        let calls = Arc::new(AtomicUsize::new(0));
        controller.stop();
        let observed = Arc::clone(&calls);
        controller.stop_signal.install(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_after_run_loop_install_wakes_worker_without_polling() {
        let controller = test_controller();
        let (wake_tx, wake_rx) = mpsc::channel();
        let (waiting_tx, waiting_rx) = mpsc::channel();
        let stop_signal = Arc::clone(&controller.stop_signal);
        let worker = std::thread::spawn(move || {
            stop_signal.install(move || wake_tx.send(()).expect("wake lifecycle worker"));
            waiting_tx.send(()).expect("report blocking wait entry");
            wake_rx.recv().expect("stop signal wakes lifecycle worker");
            stop_signal.clear();
        });

        waiting_rx
            .recv()
            .expect("worker installed its native stop action");
        controller.stop();
        worker.join().expect("worker exits after stop wake");
    }

    #[test]
    fn repeated_stop_is_idempotent() {
        let controller = test_controller();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        controller.stop_signal.install(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });

        controller.stop();
        controller.stop();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn observer_cleanup_runs_when_worker_scope_ends() {
        let calls = Arc::new(AtomicUsize::new(0));
        {
            let observed = Arc::clone(&calls);
            let _cleanup = ObserverCleanup::new(move || {
                observed.fetch_add(1, Ordering::SeqCst);
            });
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unexpected_observer_exit_is_diagnostic() {
        let controller = test_controller();
        super::record_observer_failure(
            &controller.state,
            "macos_lifecycle_run_loop_exited_unexpectedly",
        );
        assert_eq!(
            controller.snapshot().state,
            MacLifecycleState::ReconcileRequired
        );
        assert_eq!(
            controller.snapshot().last_error.as_deref(),
            Some("macos_lifecycle_run_loop_exited_unexpectedly")
        );
    }

    #[test]
    fn lifecycle_transitions_are_fail_closed_until_reconciliation_succeeds() {
        let controller = MacLifecycleController::start(|_| Ok(())).expect("controller starts");

        MacLifecycleController::apply_event(
            &controller.state,
            &|_| Ok(()),
            MacLifecycleEvent::WillSleep,
        );
        assert_eq!(controller.snapshot().state, MacLifecycleState::Suspended);
        MacLifecycleController::apply_event(
            &controller.state,
            &|_| Ok(()),
            MacLifecycleEvent::DidWake,
        );
        assert_eq!(controller.snapshot().state, MacLifecycleState::Active);
        controller.stop();
    }

    #[test]
    fn failed_reconciliation_remains_visible() {
        let controller = MacLifecycleController::start(|_| Err("reconcile_failed".to_string()))
            .expect("controller starts");
        MacLifecycleController::apply_event(
            &controller.state,
            &|_| Err("reconcile_failed".to_string()),
            MacLifecycleEvent::DidMount,
        );
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.state, MacLifecycleState::ReconcileRequired);
        assert_eq!(snapshot.last_error.as_deref(), Some("reconcile_failed"));
        controller.stop();
    }

    #[test]
    fn resource_policy_change_wakes_admission_without_changing_lifecycle_state() {
        let controller = MacLifecycleController::start(|_| Ok(())).expect("controller starts");
        let before = controller.snapshot();
        let callbacks = std::sync::atomic::AtomicUsize::new(0);
        MacLifecycleController::apply_event(
            &controller.state,
            &|event| {
                assert_eq!(event, MacLifecycleEvent::ResourcePolicyChanged);
                callbacks.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                Ok(())
            },
            MacLifecycleEvent::ResourcePolicyChanged,
        );
        assert_eq!(callbacks.load(std::sync::atomic::Ordering::Acquire), 1);
        assert_eq!(controller.snapshot(), before);
        controller.stop();
    }
}
