use super::*;
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
    let worker = unsafe { &mut *(data as *mut Worker) };
    match notification as i32 {
        TDN_DIALOG_CONSTRUCTED => unsafe {
            SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_MARQUEE as u32, 1, 0);
        },
        TDN_TIMER => {
            if worker.receiver.is_none() {
                unsafe {
                    UpdateWindow(hwnd);
                }
                eprintln!("database_upgrade native_dialog_displayed platform=windows");
                worker.start();
            }
            if worker.poll() {
                unsafe {
                    SendMessageW(hwnd, TDM_CLICK_BUTTON as u32, IDCANCEL as usize, 0);
                }
            }
        }
        TDN_BUTTON_CLICKED if wparam == IDCANCEL as usize && worker.result.is_none() => {
            worker.cancel();
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
    loop {
        let mut worker = Worker::new(path.clone());
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
            lpCallbackData: (&mut worker as *mut Worker) as isize,
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
            return Err(format!("Cannot display the native upgrade window (HRESULT {hr:#x}). The upgrade was not started."));
        }
        match worker
            .result
            .take()
            .unwrap_or_else(|| Err("Upgrade window closed before the database was opened.".into()))
        {
            Ok(db) => return Ok(db),
            Err(error) => {
                let message = wide(&recovery_message(&error));
                let action = unsafe {
                    MessageBoxW(
                        std::ptr::null_mut(),
                        message.as_ptr(),
                        title.as_ptr(),
                        MB_RETRYCANCEL | MB_ICONERROR | MB_SETFOREGROUND,
                    )
                };
                if action != IDRETRY {
                    return Err(error);
                }
            }
        }
    }
}
