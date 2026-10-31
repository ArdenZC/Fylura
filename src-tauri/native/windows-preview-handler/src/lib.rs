//! Windows Preview Handler production implementation.
//!
//! `IInitializeWithStream` retains the shell stream only until the synchronous
//! owner-apartment capture in `DoPreview` finishes. The stream is then dropped
//! before a HostProvided memory source is registered or any deferred work is
//! admitted. Deferred work owns only bounded bytes, a request token and
//! representation state; it never receives an IStream, proxy, clone or path.

#![cfg(windows)]

mod capture;
mod com;
mod completion;
mod state;
mod window;

#[cfg(any(test, feature = "test-registration"))]
pub mod test_registration;

use fylura_native_host::{HostProvidedConfig, HostProvidedRegistry};
use fylura_windows_preview_registration as registration;
use std::sync::{atomic::AtomicU32, Arc, OnceLock};
use windows::core::{GUID, HRESULT};

pub const PREVIEW_HANDLER_CLSID: GUID = GUID::from_u128(registration::PRODUCTION_CLSID_U128);

pub(crate) const S_OK: HRESULT = HRESULT(0);
pub(crate) const S_FALSE: HRESULT = HRESULT(1);
pub(crate) const E_FAIL: HRESULT = HRESULT(0x80004005_u32 as _);
pub(crate) const E_NOTIMPL: HRESULT = HRESULT(0x80004001_u32 as _);
pub(crate) const E_POINTER: HRESULT = HRESULT(0x80004003_u32 as _);
pub(crate) const E_UNEXPECTED: HRESULT = HRESULT(0x8000FFFF_u32 as _);
pub(crate) const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111_u32 as _);
pub(crate) const CLASS_E_NOAGGREGATION: HRESULT = HRESULT(0x80040110_u32 as _);
pub(crate) const E_ABORT: HRESULT = HRESULT(0x80004004_u32 as _);

pub(crate) static ACTIVE_OBJECTS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SERVER_LOCKS: AtomicU32 = AtomicU32::new(0);
pub(crate) static ACTIVE_DEFERRED: AtomicU32 = AtomicU32::new(0);

static HOST_REGISTRY: OnceLock<Arc<HostProvidedRegistry>> = OnceLock::new();

pub(crate) fn host_registry() -> Arc<HostProvidedRegistry> {
    Arc::clone(HOST_REGISTRY.get_or_init(|| {
        HostProvidedRegistry::new(HostProvidedConfig::default())
            .expect("valid W4-03 HostProvided registry configuration")
    }))
}

pub(crate) fn class_id_is_supported(clsid: &GUID) -> bool {
    if *clsid == PREVIEW_HANDLER_CLSID {
        return true;
    }

    #[cfg(feature = "test-registration")]
    {
        *clsid == test_registration::TEST_CLSID
    }
    #[cfg(not(feature = "test-registration"))]
    {
        false
    }
}

#[cfg(feature = "test-observability")]
mod observations {
    use std::{
        sync::{
            atomic::{AtomicU32, AtomicU64, Ordering},
            Condvar, Mutex, OnceLock,
        },
        time::{Duration, Instant},
    };

    pub static CAPTURE_BYTES: AtomicU64 = AtomicU64::new(0);
    pub static CAPTURE_COMPLETE: AtomicU32 = AtomicU32::new(0);
    pub static CAPTURE_READ_CALLS: AtomicU32 = AtomicU32::new(0);
    pub static COMPLETION_WINDOW_CREATES: AtomicU32 = AtomicU32::new(0);
    pub static COMPLETION_WINDOW_DESTROYS: AtomicU32 = AtomicU32::new(0);
    pub static PHASE: AtomicU32 = AtomicU32::new(0);

    #[derive(Default)]
    struct DeferredGateState {
        hold: bool,
        entered: bool,
    }

    static DEFERRED_GATE: OnceLock<(Mutex<DeferredGateState>, Condvar)> = OnceLock::new();

    fn deferred_gate() -> &'static (Mutex<DeferredGateState>, Condvar) {
        DEFERRED_GATE.get_or_init(Default::default)
    }

    pub fn hold_deferred() {
        let (state, _) = deferred_gate();
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.hold = true;
        state.entered = false;
    }

    pub fn wait_until_deferred_held(timeout_ms: u32) -> bool {
        let (state, changed) = deferred_gate();
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if result.timed_out() && !state.entered {
                return false;
            }
        }
        true
    }

    pub fn wait_for_deferred_release() {
        let (state, changed) = deferred_gate();
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.entered = true;
        changed.notify_all();
        while state.hold {
            state = changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub fn release_deferred() {
        let (state, changed) = deferred_gate();
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.hold = false;
        changed.notify_all();
    }

    pub fn reset() {
        CAPTURE_BYTES.store(0, Ordering::Release);
        CAPTURE_COMPLETE.store(0, Ordering::Release);
        CAPTURE_READ_CALLS.store(0, Ordering::Release);
        PHASE.store(0, Ordering::Release);
        release_deferred();
    }
}

#[cfg(feature = "test-observability")]
pub(crate) fn record_capture(captured: &capture::CapturedSource) {
    use std::sync::atomic::Ordering;
    observations::CAPTURE_BYTES.store(captured.bytes.len() as u64, Ordering::Release);
    observations::CAPTURE_COMPLETE.store(captured.complete as u32, Ordering::Release);
    observations::CAPTURE_READ_CALLS.store(captured.read_calls as u32, Ordering::Release);
    observations::PHASE.store(1, Ordering::Release);
}

#[cfg(not(feature = "test-observability"))]
pub(crate) fn record_capture(_: &capture::CapturedSource) {}

#[cfg(feature = "test-observability")]
pub(crate) fn record_stream_released() {
    observations::PHASE.store(2, std::sync::atomic::Ordering::Release);
}

#[cfg(not(feature = "test-observability"))]
pub(crate) fn record_stream_released() {}

#[cfg(feature = "test-observability")]
pub(crate) fn record_deferred_admitted() {
    observations::PHASE.store(3, std::sync::atomic::Ordering::Release);
}

#[cfg(not(feature = "test-observability"))]
pub(crate) fn record_deferred_admitted() {}

#[cfg(feature = "test-observability")]
pub(crate) fn record_completion_window_created() {
    observations::COMPLETION_WINDOW_CREATES.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

#[cfg(not(feature = "test-observability"))]
pub(crate) fn record_completion_window_created() {}

#[cfg(feature = "test-observability")]
pub(crate) fn record_completion_window_destroyed() {
    observations::COMPLETION_WINDOW_DESTROYS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

#[cfg(not(feature = "test-observability"))]
pub(crate) fn record_completion_window_destroyed() {}

#[no_mangle]
/// # Safety
///
/// This export has no pointer arguments and reads only process-local lifetime
/// counters. It is the standard COM server unload probe.
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    let host_records_empty = HOST_REGISTRY
        .get()
        .is_none_or(|registry| registry.count() == 0);
    if ACTIVE_OBJECTS.load(std::sync::atomic::Ordering::Acquire) == 0
        && SERVER_LOCKS.load(std::sync::atomic::Ordering::Acquire) == 0
        && ACTIVE_DEFERRED.load(std::sync::atomic::Ordering::Acquire) == 0
        && host_records_empty
        && !completion::class_registered()
    {
        S_OK
    } else {
        S_FALSE
    }
}

#[no_mangle]
/// # Safety
///
/// The caller must provide valid readable CLSID/IID pointers and a writable
/// output pointer, following the COM class-factory ABI.
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    com::dll_get_class_object(rclsid, riid, ppv)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads process-local counters.
pub unsafe extern "system" fn W4_03_TestHostProvidedRecordCount() -> u32 {
    host_registry().count() as u32
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads process-local counters.
pub unsafe extern "system" fn W4_03_TestActiveDeferredCount() -> u32 {
    ACTIVE_DEFERRED.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads the owner-STA completion
/// window lifetime counter.
pub unsafe extern "system" fn W4_03_TestCompletionWindowCount() -> u32 {
    completion::live_window_count()
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads the completion window
/// creation counter.
pub unsafe extern "system" fn W4_03_TestCompletionWindowCreateCount() -> u32 {
    observations::COMPLETION_WINDOW_CREATES.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads the completion window
/// destruction counter.
pub unsafe extern "system" fn W4_03_TestCompletionWindowDestroyCount() -> u32 {
    observations::COMPLETION_WINDOW_DESTROYS.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads process-local counters.
pub unsafe extern "system" fn W4_03_TestLastCaptureBytes() -> u64 {
    observations::CAPTURE_BYTES.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads process-local counters.
pub unsafe extern "system" fn W4_03_TestLastCaptureComplete() -> windows::core::BOOL {
    (observations::CAPTURE_COMPLETE.load(std::sync::atomic::Ordering::Acquire) != 0).into()
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads process-local counters.
pub unsafe extern "system" fn W4_03_TestLastCaptureReadCalls() -> u32 {
    observations::CAPTURE_READ_CALLS.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and updates only test observations.
pub unsafe extern "system" fn W4_03_TestCapturePhase() -> u32 {
    observations::PHASE.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and updates only test observations.
pub unsafe extern "system" fn W4_03_TestResetObservations() {
    observations::reset();
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and updates only test observations.
pub unsafe extern "system" fn W4_03_TestHoldDeferred() {
    observations::hold_deferred();
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and reads process-local test state.
pub unsafe extern "system" fn W4_03_TestWaitDeferredHeld(timeout_ms: u32) -> windows::core::BOOL {
    observations::wait_until_deferred_held(timeout_ms).into()
}

#[cfg(feature = "test-observability")]
#[no_mangle]
/// # Safety
///
/// This test-only export takes no pointers and updates only test observations.
pub unsafe extern "system" fn W4_03_TestReleaseDeferred() {
    observations::release_deferred();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_identity_is_stable_and_production_specific() {
        assert_eq!(
            PREVIEW_HANDLER_CLSID,
            GUID::from_u128(registration::PRODUCTION_CLSID_U128)
        );
    }

    fn class_factory_status(clsid: &GUID) -> HRESULT {
        use std::ptr::null_mut;
        use windows::core::Interface;
        use windows::Win32::System::Com::IClassFactory;

        let mut output = null_mut();
        let status =
            com::dll_get_class_object(clsid, &<IClassFactory as Interface>::IID, &mut output);
        if !output.is_null() {
            let factory = unsafe { IClassFactory::from_raw(output) };
            drop(factory);
        }
        status
    }

    #[test]
    fn production_class_factory_remains_available() {
        assert_eq!(class_factory_status(&PREVIEW_HANDLER_CLSID), S_OK);
    }

    #[cfg(not(feature = "test-registration"))]
    #[test]
    fn default_build_rejects_isolated_test_clsid() {
        assert_eq!(
            class_factory_status(&test_registration::TEST_CLSID),
            CLASS_E_CLASSNOTAVAILABLE
        );
    }

    #[cfg(feature = "test-registration")]
    #[test]
    fn test_registration_build_accepts_isolated_test_clsid() {
        assert_eq!(class_factory_status(&test_registration::TEST_CLSID), S_OK);
    }
}
