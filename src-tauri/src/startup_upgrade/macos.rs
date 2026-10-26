use super::*;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSApplication, NSModalPanelRunLoopMode};
use objc2_foundation::{NSRunLoop, NSString, NSTimer};
use std::{cell::RefCell, rc::Rc};

pub(super) fn open(path: PathBuf) -> Result<Database, String> {
    let mtm = MainThreadMarker::new().ok_or("Database upgrade UI must run on the main thread")?;
    let application = NSApplication::sharedApplication(mtm);
    loop {
        let worker = Rc::new(RefCell::new(Worker::new(path.clone())));
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(MESSAGE));
        alert.setInformativeText(&NSString::from_str(DETAILS));
        let cancel = alert.addButtonWithTitle(&NSString::from_str("Cancel upgrade"));
        let tick_worker = worker.clone();
        #[cfg(feature = "native-qa")]
        let tick_cancel = cancel.clone();
        let tick_app = application.clone();
        let tick_alert = alert.clone();
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
            if tick_worker.borrow().receiver.is_none() {
                tick_alert.window().displayIfNeeded();
                #[cfg(feature = "native-qa")]
                qa::displayed(tick_alert.window().isVisible());
                eprintln!("database_upgrade native_dialog_displayed platform=macos");
                tick_worker.borrow_mut().start();
            }
            #[cfg(feature = "native-qa")]
            if qa::cancel_on_tick() {
                unsafe {
                    tick_cancel.performClick(None);
                }
            }
            if tick_worker.borrow_mut().poll() {
                tick_app.stopModalWithCode(1002);
            }
        });
        // The timer is installed only on this main-thread modal run loop; its
        // block and Rc captures are never transferred to another thread.
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.2, true, &block) };
        unsafe {
            NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSModalPanelRunLoopMode);
        }
        loop {
            alert.runModal();
            if worker.borrow().result.is_some() {
                break;
            }
            worker.borrow().cancel();
            cancel.setEnabled(false);
            alert.setMessageText(&NSString::from_str("Cancelling upgrade"));
            alert.setInformativeText(&NSString::from_str("Waiting for SQLite to roll back. Please keep Zen Canvas open until this window closes."));
        }
        timer.invalidate();
        alert.window().orderOut(None);
        let result = worker
            .borrow_mut()
            .result
            .take()
            .expect("modal loop exits only after worker completion");
        match result {
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
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let recovery = NSAlert::new(mtm);
    recovery.setMessageText(&NSString::from_str(TITLE));
    recovery.setInformativeText(&NSString::from_str(&recovery_message(error)));
    let retry = recovery.addButtonWithTitle(&NSString::from_str("Retry"));
    #[cfg(not(feature = "native-qa"))]
    let _ = retry;
    #[cfg(feature = "native-qa")]
    let retry_timer = if qa::active() {
        let retry_block = block2::RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            qa::retried();
            unsafe {
                timer.as_ref().invalidate();
                retry.performClick(None);
            }
        });
        let timer =
            unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.2, false, &retry_block) };
        unsafe {
            NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSModalPanelRunLoopMode);
        }
        Some(timer)
    } else {
        None
    };
    recovery.addButtonWithTitle(&NSString::from_str("Exit"));
    let response = recovery.runModal();
    #[cfg(feature = "native-qa")]
    if let Some(timer) = retry_timer {
        timer.invalidate();
    }

    response == 1000
}
