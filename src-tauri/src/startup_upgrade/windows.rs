use super::*;
use std::cell::RefCell;
use windows_sys::Win32::Graphics::Gdi::UpdateWindow;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{Controls::*, WindowsAndMessaging::*},
};
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn callback(
    hwnd: HWND,
    notification: u32,
    wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> i32 {
    // Task Dialog messages can synchronously re-enter this callback. Never
    // retain a mutable Worker reference/borrow across SendMessage/painting.
    let state = unsafe { &*(data as *const RefCell<Worker>) };
    match notification as i32 {
        TDN_DIALOG_CONSTRUCTED => unsafe {
            SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_MARQUEE as u32, 1, 0);
        },
        TDN_TIMER => {
            if state.borrow().receiver.is_none() {
                unsafe {
                    UpdateWindow(hwnd);
                }
                #[cfg(feature = "native-qa")]
                unsafe {
                    qa::displayed(IsWindowVisible(hwnd) != 0);
                }
                eprintln!("database_upgrade native_dialog_displayed platform=windows");
                state.borrow_mut().start();
            }
            #[cfg(feature = "native-qa")]
            if qa::cancel_on_tick() {
                unsafe {
                    SendMessageW(hwnd, TDM_CLICK_BUTTON as u32, IDCANCEL as usize, 0);
                }
            }
            let finished = state.borrow_mut().poll();
            if finished {
                unsafe {
                    SendMessageW(hwnd, TDM_CLICK_BUTTON as u32, IDCANCEL as usize, 0);
                }
            }
        }
        TDN_BUTTON_CLICKED if wparam == IDCANCEL as usize && state.borrow().result.is_none() => {
            state.borrow().cancel();
            let stopping = wide("Cancelling upgrade — waiting for SQLite to roll back…");
            unsafe {
                SendMessageW(
                    hwnd,
                    TDM_SET_ELEMENT_TEXT as u32,
                    TDE_CONTENT as usize,
                    stopping.as_ptr() as isize,
                );
            }
            return 1; // Keep window/message loop alive until the transaction ends.
        }
        _ => {}
    }
    0
}
pub(super) fn open(path: PathBuf) -> Result<Database, String> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES | ICC_PROGRESS_CLASS,
    };
    if unsafe { InitCommonControlsEx(&controls) } == 0 {
        return Err(
            "Cannot initialize native upgrade controls. The database was not opened.".into(),
        );
    }
    loop {
        let mut worker = RefCell::new(Worker::new(path.clone()));
        let title = wide(TITLE);
        let heading = wide(MESSAGE);
        let details = wide(DETAILS);
        let config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            dwFlags: TDF_SHOW_MARQUEE_PROGRESS_BAR
                | TDF_CALLBACK_TIMER
                | TDF_ALLOW_DIALOG_CANCELLATION
                | TDF_SIZE_TO_CONTENT,
            dwCommonButtons: TDCBF_CANCEL_BUTTON,
            pszWindowTitle: title.as_ptr(),
            pszMainInstruction: heading.as_ptr(),
            pszContent: details.as_ptr(),
            pfCallback: Some(callback),
            lpCallbackData: (&worker as *const RefCell<Worker>) as isize,
            ..Default::default()
        };
        let hr = unsafe {
            TaskDialogIndirect(
                &config,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if hr < 0 {
            let error=format!("Cannot display the native upgrade window (HRESULT {hr:#x}). Reopen Zen Canvas to retry. A completed commit is retained; SQLite recovers an interrupted upgrade.");
            drop(worker); // cancellation/worker completion precedes the final error notice
            let message = wide(&error);
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
                );
            }
            return Err(error);
        }
        match worker
            .get_mut()
            .result
            .take()
            .unwrap_or_else(|| Err("Upgrade window closed before the database was opened.".into()))
        {
            Ok(db) => return Ok(db),
            Err(error) => {
                if !recover(&error) {
                    return Err(error);
                }
            }
        }
    }
}

pub(super) fn recover(error: &str) -> bool {
    let title = wide(TITLE);
    let heading = wide("The database could not be opened");
    let message = wide(&recovery_message(error));
    let retry = wide("Retry");
    let exit = wide("Exit");
    let buttons = [
        TASKDIALOG_BUTTON {
            nButtonID: IDRETRY,
            pszButtonText: retry.as_ptr(),
        },
        TASKDIALOG_BUTTON {
            nButtonID: IDCANCEL,
            pszButtonText: exit.as_ptr(),
        },
    ];
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: heading.as_ptr(),
        pszContent: message.as_ptr(),
        cButtons: 2,
        pButtons: buttons.as_ptr(),
        nDefaultButton: IDCANCEL,
        ..Default::default()
    };
    #[cfg(feature = "native-qa")]
    if qa::active() {
        schedule_retry();
    }
    let mut button = 0;
    let hr = unsafe {
        TaskDialogIndirect(
            &config,
            &mut button,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if hr < 0 {
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
        return false;
    }
    button == IDRETRY
}

#[cfg(feature = "native-qa")]
fn schedule_retry() {
    std::thread::spawn(|| {
        let title = wide(TITLE);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
            if !hwnd.is_null()
                && unsafe { !GetDlgItem(hwnd, IDRETRY).is_null() && IsWindowVisible(hwnd) != 0 }
            {
                qa::retried();
                unsafe {
                    PostMessageW(hwnd, TDM_CLICK_BUTTON as u32, IDRETRY as usize, 0);
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Do not leave an unattended native modal loop running forever.
        std::process::exit(3);
    });
}
