//! A native modal loop remains live while SQLite opens on a worker. No webview,
//! IPC permission, indexer or watcher is started before the database is ready.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};
use zen_canvas_tauri::db::Database;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(feature = "native-qa")]
mod qa;
#[cfg(windows)]
mod windows;

#[cfg(feature = "native-qa")]
pub fn run_native_qa() -> Result<(), String> {
    qa::run()
}

const TITLE: &str = "Zen Canvas — Database upgrade";
const MESSAGE: &str = "Preparing your local search database";
const DETAILS: &str = "Checking available storage, updating search indexes and preparing exact source statistics. This may take several minutes. Your files are not changed.\n\nYou can cancel and retry when you next open Zen Canvas. Please keep the app open until cancellation finishes.";

struct Worker {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    receiver: Option<mpsc::Receiver<Result<Database, String>>>,
    result: Option<Result<Database, String>>,
}
impl Worker {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver: None,
            result: None,
        }
    }
    // Called only after the native dialog has displayed and entered its message
    // loop. In particular, do not spawn this worker before creating the window.
    fn start(&mut self) {
        if self.receiver.is_some() || self.result.is_some() {
            return;
        }
        #[cfg(feature = "native-qa")]
        qa::starting();
        let path = self.path.clone();
        let cancel = self.cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let spawn=std::thread::Builder::new().name("database-upgrade".into()).spawn(move || {
            eprintln!("database_upgrade worker_started");
            #[cfg(feature = "native-qa")]
            qa::delay_worker();
            let result=std::panic::catch_unwind(||Database::open_with_upgrade_cancellation(path,cancel))
                .map_err(|_|"The database upgrade worker stopped unexpectedly. Restart Zen Canvas to retry. SQLite will recover its transaction.".to_string())
                .and_then(|r|r.map_err(|e|e.to_string()));
            let _ = sender.send(result);
        });
        if let Err(error) = spawn {
            self.result = Some(Err(format!(
                "Cannot start the database upgrade worker: {error}"
            )));
        }
    }
    fn poll(&mut self) -> bool {
        if self.result.is_some() {
            return true;
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => self.result = Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.result = Some(Err(
                        "The upgrade worker disconnected. Restart Zen Canvas to retry.".into(),
                    ))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.result.is_some()
    }
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Even an unexpected native dialog failure must not leave an invisible
        // migration running after the recovery/exit decision.
        if self.result.is_none() {
            self.cancel();
            if let Some(receiver) = &self.receiver {
                let _ = receiver.recv();
            }
        }
    }
}

fn needs_upgrade(path: &std::path::Path) -> bool {
    match path.try_exists() {
        Ok(false) => return false,
        Err(_) => return true,
        Ok(true) => {}
    }
    // Probe only; no DDL, journal-mode change or pool/worker startup. An invalid
    // or future database is shown through the same explicit recovery dialog.
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| c.query_row::<i64, _, _>("PRAGMA user_version", [], |r| r.get(0)))
        .map_or(true, |version| {
            version != i64::from(zen_canvas_tauri::db::SUPPORTED_SCHEMA_VERSION)
        })
}

pub fn open(path: PathBuf, background: bool) -> Result<Database, String> {
    if !needs_upgrade(&path) {
        match Database::open(&path) {
            Ok(db) => return Ok(db),
            Err(error) => {
                let error = error.to_string();
                if background {
                    return Err(error);
                }
                #[cfg(windows)]
                if !windows::recover(&error, &path) {
                    return Err(error);
                }
                #[cfg(target_os = "macos")]
                if !macos::recover(&error, &path) {
                    return Err(error);
                }
            }
        }
    }
    if background {
        // An autostart/tray launch must not start an invisible multi-minute
        // upgrade or steal focus. A foreground launch resumes from user_version.
        return Err("Database upgrade pending. Open Zen Canvas normally to review and start the upgrade; background indexing has not started.".into());
    }
    #[cfg(windows)]
    {
        windows::open(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open(path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        Err("Database upgrades require a supported native desktop platform.".into())
    }
}

fn recovery_message(error: &str, path: &std::path::Path) -> String {
    format!("The database could not be opened.\n\n{error}\n\nDatabase location: {}\n\nFree space on the database and system temporary volumes if storage is full, then choose Retry. Close other Zen Canvas processes if the database is busy.\n\nIf the problem persists, exit all Zen Canvas processes, then preserve the database together with any existing -wal and -shm files before contacting support. Do not delete indexes or change the schema version. A completed commit is retained; an interrupted transaction is recovered by SQLite at the next open.", path.display())
}
