use std::{
    ffi::c_void,
    mem::size_of,
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex, OnceLock,
    },
};

use windows::{
    core::{w, Error, Result, PCWSTR},
    Win32::{
        Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::{
            GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, PostMessageW,
            RegisterClassExW, SetWindowLongPtrW, UnregisterClassW, GWLP_USERDATA, HWND_MESSAGE,
            WM_NCDESTROY, WNDCLASSEXW, WS_EX_NOACTIVATE,
        },
    },
};

use crate::com::PreviewHandler;

const COMPLETION_MESSAGE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x403;
const COMPLETION_CLASS_NAME: PCWSTR = w!("Fylura.W4_03.CompletionWindow");
static NEXT_NOTIFICATION_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Default)]
struct CompletionClassState {
    instance: usize,
    registered: bool,
    live_windows: u32,
}

static COMPLETION_CLASS: OnceLock<Mutex<CompletionClassState>> = OnceLock::new();

fn completion_class() -> &'static Mutex<CompletionClassState> {
    COMPLETION_CLASS.get_or_init(|| Mutex::new(CompletionClassState::default()))
}

fn handler_instance() -> Result<HINSTANCE> {
    let mut module = HMODULE::default();
    let window_proc_address = window_proc as *const () as *const u16;
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(window_proc_address),
            &mut module,
        )?;
    }
    Ok(module.into())
}

fn acquire_window_slot() -> Result<HINSTANCE> {
    let mut class = completion_class()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !class.registered {
        let instance = handler_instance()?;
        let class_definition = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            hInstance: instance,
            lpfnWndProc: Some(window_proc),
            lpszClassName: COMPLETION_CLASS_NAME,
            ..Default::default()
        };
        if unsafe { RegisterClassExW(&class_definition) } == 0 {
            return Err(Error::from_win32());
        }
        class.instance = instance.0 as usize;
        class.registered = true;
    }
    class.live_windows += 1;
    Ok(HINSTANCE(class.instance as *mut c_void))
}

fn release_window_slot() {
    let mut class = completion_class()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    class.live_windows = class.live_windows.saturating_sub(1);
    if class.live_windows == 0 && class.registered {
        let instance = HINSTANCE(class.instance as *mut c_void);
        if unsafe { UnregisterClassW(COMPLETION_CLASS_NAME, Some(instance)) }.is_ok() {
            class.instance = 0;
            class.registered = false;
        }
    }
}

pub(crate) fn class_registered() -> bool {
    completion_class()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .registered
}

#[cfg(feature = "test-observability")]
pub(crate) fn live_window_count() -> u32 {
    completion_class()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .live_windows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredPreview {
    pub(crate) text: String,
    pub(crate) bytes_len: usize,
    pub(crate) complete: bool,
    pub(crate) language: Option<String>,
}

pub(crate) struct DeferredCompletion {
    notification_id: u32,
    result: Mutex<Option<std::result::Result<DeferredPreview, String>>>,
}

impl DeferredCompletion {
    pub(crate) fn new() -> Self {
        let notification_id = NEXT_NOTIFICATION_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            notification_id,
            result: Mutex::new(None),
        }
    }

    pub(crate) fn notification_id(&self) -> u32 {
        self.notification_id
    }

    pub(crate) fn store(&self, result: std::result::Result<DeferredPreview, String>) {
        *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
    }

    pub(crate) fn take(&self) -> Option<std::result::Result<DeferredPreview, String>> {
        self.result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// A message-only window created on the handler's owner STA. It carries only
/// an opaque notification id; the worker never receives a handler pointer or
/// any COM/native source object. The private class is registered by this
/// subsystem, so its WndProc is direct ownership rather than a STATIC-window
/// subclass that would require previous-procedure restoration.
pub(crate) struct CompletionWindow {
    hwnd: HWND,
}

impl CompletionWindow {
    pub(crate) fn create(owner: *const PreviewHandler) -> Result<Self> {
        let instance = acquire_window_slot()?;
        let hwnd = match unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                COMPLETION_CLASS_NAME,
                w!(""),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )
        } {
            Ok(hwnd) => hwnd,
            // A failed CreateWindowExW must release the class slot acquired
            // above. If this was the last slot, the private class is removed
            // before the error is returned.
            Err(error) => {
                release_window_slot();
                return Err(error);
            }
        };
        unsafe {
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner as isize);
        }
        crate::record_completion_window_created();
        Ok(Self { hwnd })
    }

    pub(crate) fn raw_handle(&self) -> isize {
        self.hwnd.0 as isize
    }
}

impl Drop for CompletionWindow {
    fn drop(&mut self) {
        let destroyed = unsafe {
            let _ = SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd).is_ok()
        };
        if destroyed {
            release_window_slot();
            crate::record_completion_window_destroyed();
        }
    }
}

pub(crate) fn post_completion(raw_hwnd: isize, notification_id: u32) {
    if raw_hwnd == 0 {
        return;
    }
    let hwnd = HWND(raw_hwnd as *mut c_void);
    unsafe {
        let _ = PostMessageW(
            Some(hwnd),
            COMPLETION_MESSAGE,
            WPARAM(notification_id as usize),
            LPARAM(0),
        );
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == COMPLETION_MESSAGE {
        let owner = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const PreviewHandler;
        if !owner.is_null() {
            (&*owner).publish_deferred(wparam.0 as u32);
        }
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}
