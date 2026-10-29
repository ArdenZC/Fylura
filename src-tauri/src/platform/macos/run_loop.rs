//! Narrow stop/wake support for native macOS run-loop workers.

use std::sync::Mutex;

#[cfg(target_os = "macos")]
use objc2_core_foundation::CFRunLoop;

type StopAction = Box<dyn Fn() + Send + Sync + 'static>;

/// A dormant source gives the lifecycle worker a blocking lifetime without a
/// timer. It is signalled only for shutdown, including stop-before-run races.
#[cfg(target_os = "macos")]
pub(crate) struct LifecycleRunLoop {
    run_loop: objc2_core_foundation::CFRetained<CFRunLoop>,
    source: objc2_core_foundation::CFRetained<objc2_core_foundation::CFRunLoopSource>,
}

#[cfg(target_os = "macos")]
impl LifecycleRunLoop {
    pub(crate) fn new() -> Result<Self, &'static str> {
        use objc2_core_foundation::{
            kCFRunLoopDefaultMode, CFRunLoopSource, CFRunLoopSourceContext,
        };

        unsafe extern "C-unwind" fn stop_on_entry(_: *mut std::ffi::c_void) {
            if let Some(run_loop) = CFRunLoop::current() {
                run_loop.stop();
            }
        }

        let run_loop = CFRunLoop::current().ok_or("macos_lifecycle_run_loop_unavailable")?;
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: Some(stop_on_entry),
        };
        // CF copies the context; the callback captures no borrowed state.
        let source = unsafe { CFRunLoopSource::new(None, 0, &mut context) }
            .ok_or("macos_lifecycle_source_create_failed")?;
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
        Ok(Self { run_loop, source })
    }

    pub(crate) fn stop_action(&self) -> impl Fn() + Send + Sync + 'static {
        struct StopHandles {
            run_loop: objc2_core_foundation::CFRetained<CFRunLoop>,
            source: objc2_core_foundation::CFRetained<objc2_core_foundation::CFRunLoopSource>,
        }
        // CF run loops and source signalling are thread-safe. Only retained
        // handles cross threads; registration/removal stays on the worker.
        unsafe impl Send for StopHandles {}
        unsafe impl Sync for StopHandles {}
        impl StopHandles {
            fn stop(&self) {
                // CFRunLoopStop targets an active invocation. A pending source
                // also stops the next invocation if the worker has not entered
                // CFRunLoopRun yet. No periodic work is scheduled.
                self.source.signal();
                self.run_loop.stop();
                self.run_loop.wake_up();
            }
        }
        let handles = StopHandles {
            run_loop: self.run_loop.clone(),
            source: self.source.clone(),
        };
        move || handles.stop()
    }

    pub(crate) fn run(&self) {
        CFRunLoop::run();
    }

    #[cfg(test)]
    pub(super) fn waiting_probe(&self) -> impl Fn() -> bool + Send + 'static {
        struct Probe(objc2_core_foundation::CFRetained<CFRunLoop>);
        // A read-only CFRunLoopIsWaiting probe; CFRunLoop is thread-safe.
        unsafe impl Send for Probe {}
        impl Probe {
            fn waiting(&self) -> bool {
                self.0.is_waiting()
            }
        }
        let probe = Probe(self.run_loop.clone());
        move || probe.waiting()
    }
}

#[cfg(target_os = "macos")]
impl Drop for LifecycleRunLoop {
    fn drop(&mut self) {
        self.run_loop.remove_source(Some(&self.source), unsafe {
            objc2_core_foundation::kCFRunLoopDefaultMode
        });
        self.source.invalidate();
    }
}

#[derive(Default)]
struct StopState {
    requested: bool,
    delivered: bool,
    action: Option<StopAction>,
}

/// Owns a cross-thread stop request until the native run loop has returned.
/// The callback stays protected by the mutex while it runs so its retained
/// native handle cannot be dropped concurrently with stop/wake.
#[derive(Default)]
pub(crate) struct NativeRunLoopStopSignal {
    state: Mutex<StopState>,
}

impl NativeRunLoopStopSignal {
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn install(&self, action: impl Fn() + Send + Sync + 'static) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.requested {
            if !state.delivered {
                state.delivered = true;
                action();
            }
        } else {
            state.action = Some(Box::new(action));
        }
    }

    pub(crate) fn request_stop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.requested {
            return;
        }
        state.requested = true;
        let should_deliver = state.action.is_some();
        if should_deliver {
            state.delivered = true;
            if let Some(action) = state.action.as_ref() {
                action();
            }
        }
    }

    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.action = None;
    }
}

#[cfg(test)]
mod tests {
    use super::NativeRunLoopStopSignal;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };

    #[test]
    fn stop_before_run_loop_install_is_preserved() {
        let signal = NativeRunLoopStopSignal::default();
        signal.request_stop();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        signal.install(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stop_after_run_loop_start_wakes_a_blocked_worker() {
        let signal = Arc::new(NativeRunLoopStopSignal::default());
        let (wake_tx, wake_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let signal_for_worker = Arc::clone(&signal);
        let worker = std::thread::spawn(move || {
            signal_for_worker.install(move || wake_tx.send(()).expect("wake worker"));
            started_tx.send(()).expect("report installed run loop");
            wake_rx.recv().expect("native stop wakes blocked worker");
            signal_for_worker.clear();
        });

        started_rx.recv().expect("run loop installed");
        signal.request_stop();
        worker.join().expect("worker exits after wake");
    }

    #[test]
    fn repeated_stop_is_idempotent_and_clear_releases_the_action() {
        let signal = NativeRunLoopStopSignal::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        signal.install(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        signal.request_stop();
        signal.request_stop();
        signal.request_stop();
        signal.clear();
        signal.request_stop();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
