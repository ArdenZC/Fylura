//! Windows Service Control Manager host for the global index provider.
//!
//! The installed service is deliberately small: it owns volume discovery and
//! metadata enumeration, and exposes only the versioned named-pipe protocol in
//! `service.rs`. It never receives arbitrary file-operation commands.

use super::service::{
    IndexServiceCommand, IndexServiceEvent, IndexServiceLookupResponse, IndexServiceRequest,
    IndexServiceResponse, IndexServiceServerIo,
};
use super::{volumes, DirectWindowsGlobalIndexProvider};
use crate::global_index::coordinator::{GlobalIndexError, GlobalIndexProvider, GlobalIndexSink};
use crate::global_index::models::{
    GlobalEntry, GlobalEntryInput, GlobalSourceDescriptor, GlobalVolume, INDEX_STATUS_PAUSED,
    INDEX_STATUS_READY,
};
use std::path::Path;
use std::ptr;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, ERROR_SERVICE_ALREADY_RUNNING,
    ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_EXISTS, ERROR_SERVICE_NOT_ACTIVE,
};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
    OpenServiceW, QueryServiceStatusEx, RegisterServiceCtrlHandlerExW, SetServiceStatus,
    StartServiceCtrlDispatcherW, StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_CREATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_ACCEPT_SHUTDOWN,
    SERVICE_ACCEPT_STOP, SERVICE_AUTO_START, SERVICE_CHANGE_CONFIG, SERVICE_CONTROL_SHUTDOWN,
    SERVICE_CONTROL_STOP, SERVICE_ERROR_NORMAL, SERVICE_QUERY_STATUS, SERVICE_START,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STATUS_PROCESS,
    SERVICE_STOP, SERVICE_STOPPED, SERVICE_STOP_PENDING, SERVICE_WIN32_OWN_PROCESS,
};

pub const INDEX_SERVICE_NAME: &str = "ZenCanvasGlobalIndex";
pub const INDEX_SERVICE_DISPLAY_NAME: &str = "Zen Canvas Global Index";
const SERVICE_DELETE_ACCESS: u32 = 0x0001_0000;

static SERVICE_CONTEXT: OnceLock<Arc<ServiceRuntime>> = OnceLock::new();
static SERVICE_STATUS_HANDLE_VALUE: AtomicUsize = AtomicUsize::new(0);

struct ServiceRuntime {
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    busy: AtomicBool,
    state: Mutex<ServiceLifecycleState>,
    operation_lock: Mutex<()>,
    provider: Arc<dyn GlobalIndexProvider>,
}

#[derive(Debug, Clone, Default)]
struct ServiceLifecycleState {
    status: String,
    last_error: Option<String>,
}

impl ServiceRuntime {
    fn new() -> Self {
        Self::with_provider(Arc::new(DirectWindowsGlobalIndexProvider::new()))
    }

    fn with_provider(provider: Arc<dyn GlobalIndexProvider>) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            busy: AtomicBool::new(false),
            state: Mutex::new(ServiceLifecycleState {
                status: "starting".to_string(),
                last_error: None,
            }),
            operation_lock: Mutex::new(()),
            provider,
        }
    }

    fn set_state(&self, status: impl Into<String>, error: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.status = status.into();
            state.last_error = error;
        }
    }

    fn status(&self) -> String {
        if self.busy.load(Ordering::Acquire) {
            return "indexing".to_string();
        }
        self.state
            .lock()
            .map(|state| state.status.clone())
            .unwrap_or_else(|_| "error".to_string())
    }

    fn request_stop(&self) {
        self.cancel.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        let _ = self.provider.pause();
        self.set_state("stopping", None);
    }

    fn response(
        &self,
        request: &IndexServiceRequest,
        ok: bool,
        error_code: Option<&str>,
        message: Option<String>,
        status: Option<String>,
    ) -> IndexServiceResponse {
        IndexServiceResponse {
            protocol_version: super::service::IPC_PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            ok,
            error_code: error_code.map(ToString::to_string),
            message,
            status,
        }
    }

    fn send_error(
        &self,
        request: &IndexServiceRequest,
        connection: &mut dyn IndexServiceServerIo,
        error_code: &str,
        message: impl Into<String>,
    ) -> Result<(), String> {
        connection.send_response(self.response(
            request,
            false,
            Some(error_code),
            Some(message.into()),
            Some(self.status()),
        ))
    }

    fn handle_request(
        &self,
        request: IndexServiceRequest,
        connection: &mut dyn IndexServiceServerIo,
    ) -> Result<(), String> {
        match request.command.clone() {
            IndexServiceCommand::Status => connection.send_response(self.response(
                &request,
                true,
                None,
                None,
                Some(self.status()),
            )),
            IndexServiceCommand::DiscoverSources => match self.provider.discover_sources() {
                Ok(sources) => {
                    let volumes = sources
                        .iter()
                        .map(|source| source.volume.clone())
                        .collect::<Vec<_>>();
                    connection.send_event(IndexServiceEvent::Sources { sources: volumes })?;
                    connection.send_response(self.response(
                        &request,
                        true,
                        None,
                        None,
                        Some("ready".to_string()),
                    ))
                }
                Err(error) => {
                    self.send_error(&request, connection, "discover_failed", error.to_string())
                }
            },
            IndexServiceCommand::Pause => {
                self.cancel.store(true, Ordering::Release);
                self.provider.pause().map_err(|error| error.to_string())?;
                self.set_state(INDEX_STATUS_PAUSED, None);
                connection.send_response(self.response(
                    &request,
                    true,
                    None,
                    None,
                    Some(INDEX_STATUS_PAUSED.to_string()),
                ))
            }
            IndexServiceCommand::Shutdown => {
                self.request_stop();
                connection.send_response(self.response(
                    &request,
                    true,
                    None,
                    None,
                    Some("stopping".to_string()),
                ))
            }
            command @ (IndexServiceCommand::StartInitialIndex { .. }
            | IndexServiceCommand::ResumeIncrementalSync { .. }
            | IndexServiceCommand::Rebuild { .. }) => {
                self.handle_index_request(command, request, connection)
            }
        }
    }

    fn handle_index_request(
        &self,
        command: IndexServiceCommand,
        request: IndexServiceRequest,
        connection: &mut dyn IndexServiceServerIo,
    ) -> Result<(), String> {
        if let Err(response) = self.try_begin_index_request(&request) {
            return connection.send_response(response);
        }
        let operation_result = (|| {
            let _operation_guard = self.operation_lock.lock().map_err(|_| {
                (
                    "index_failed",
                    "index_service_operation_lock_poisoned".to_string(),
                )
            })?;
            let source_id = source_id(&command);
            let source = self
                .validated_source(source_id, request.source.as_ref())
                .map_err(|message| ("index_failed", message))?;
            self.cancel.store(false, Ordering::Release);
            connection
                .send_event(IndexServiceEvent::Sources {
                    sources: vec![source.volume.clone()],
                })
                .map_err(|error| {
                    (
                        "index_failed",
                        format!("index_service_admission_send_failed: {error}"),
                    )
                })?;
            self.set_state("indexing", None);
            let mut sink = PipeSink {
                connection,
                cancel: self.cancel.clone(),
                volume_id: source.volume.id.clone(),
            };
            let result = match command {
                IndexServiceCommand::StartInitialIndex { .. } => {
                    self.provider
                        .start_initial_index(&source, &mut sink, &self.cancel)
                }
                IndexServiceCommand::ResumeIncrementalSync { .. } => self
                    .provider
                    .resume_incremental_sync(&source, &mut sink, &self.cancel),
                IndexServiceCommand::Rebuild { .. } => {
                    self.provider.rebuild(&source, &mut sink, &self.cancel)
                }
                _ => unreachable!(),
            };
            match result {
                Ok(()) => {
                    self.set_state(INDEX_STATUS_READY, None);
                    Ok(())
                }
                Err(error) => {
                    let (code, status) = super::service_errors::classify(&error);
                    self.set_state(
                        status,
                        (!matches!(error, GlobalIndexError::Paused)).then(|| error.to_string()),
                    );
                    Err((code, error.to_string()))
                }
            }
        })();
        self.busy.store(false, Ordering::Release);
        match operation_result {
            Ok(()) => connection.send_response(self.response(
                &request,
                true,
                None,
                None,
                Some(self.status()),
            )),
            Err((code, message)) => self.send_error(&request, connection, code, message),
        }
    }

    fn try_begin_index_request(
        &self,
        request: &IndexServiceRequest,
    ) -> Result<(), IndexServiceResponse> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| {
                self.response(
                    request,
                    false,
                    Some("index_service_busy"),
                    Some("Another global index operation is already running".to_string()),
                    Some(self.status()),
                )
            })
    }

    fn validated_source(
        &self,
        source_id: &str,
        snapshot: Option<&GlobalVolume>,
    ) -> Result<GlobalSourceDescriptor, String> {
        let Some(snapshot) = snapshot else {
            return Err("index_service_source_snapshot_required".to_string());
        };
        if snapshot.id != source_id {
            return Err("index_service_source_id_mismatch".to_string());
        }
        if !snapshot.enabled {
            return Err("index_service_source_disabled".to_string());
        }
        let discovered = self
            .provider
            .discover_sources()
            .map_err(|error| error.to_string())?;
        let Some(current) = discovered
            .into_iter()
            .find(|source| source.volume.id == source_id)
        else {
            return Err("index_service_source_unavailable".to_string());
        };
        if !current.volume.enabled {
            return Err("index_service_source_disabled".to_string());
        }
        if !super::service::source_identity_matches(snapshot, &current.volume) {
            return Err("index_service_source_changed".to_string());
        }
        // Only durable indexing state comes from the desktop snapshot. The
        // service always uses the fresh native mount path and provider data.
        let volume = merge_durable_source_state(current.volume, snapshot);
        Ok(GlobalSourceDescriptor { volume })
    }
}

fn merge_durable_source_state(mut current: GlobalVolume, snapshot: &GlobalVolume) -> GlobalVolume {
    current.enabled = snapshot.enabled;
    current.index_status = snapshot.index_status.clone();
    current.journal_id = snapshot.journal_id.clone();
    current.journal_cursor = snapshot.journal_cursor.clone();
    current.last_full_index_at = snapshot.last_full_index_at;
    current.last_incremental_sync_at = snapshot.last_incremental_sync_at;
    // Provider capability stays from fresh native discovery. A persisted
    // recursive fallback is not consent and cannot override NTFS MFT/USN.
    current
}

fn source_id(command: &IndexServiceCommand) -> &str {
    match command {
        IndexServiceCommand::StartInitialIndex { source_id }
        | IndexServiceCommand::ResumeIncrementalSync { source_id }
        | IndexServiceCommand::Rebuild { source_id } => source_id,
        _ => "",
    }
}

struct PipeSink<'a> {
    connection: &'a mut dyn IndexServiceServerIo,
    cancel: Arc<AtomicBool>,
    volume_id: String,
}

impl PipeSink<'_> {
    fn ensure_running(&self) -> Result<(), GlobalIndexError> {
        if self.cancel.load(Ordering::Acquire) {
            Err(GlobalIndexError::Paused)
        } else {
            Ok(())
        }
    }

    fn lookup_error(error: String) -> GlobalIndexError {
        GlobalIndexError::Provider(format!("index_service_lookup_failed: {error}"))
    }
}

impl GlobalIndexSink for PipeSink<'_> {
    fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
        self.ensure_running()?;
        let mut written = 0;
        for chunk in entries.chunks(64) {
            self.ensure_running()?;
            self.connection
                .send_event(IndexServiceEvent::Entries {
                    entries: chunk.to_vec(),
                })
                .map_err(Self::lookup_error)?;
            written += chunk.len();
        }
        Ok(written)
    }

    fn mark_entry_stale(&mut self, entry_id: &str) -> Result<(), GlobalIndexError> {
        self.ensure_running()?;
        self.connection
            .send_event(IndexServiceEvent::EntryStale {
                volume_id: self.volume_id.clone(),
                entry_id: entry_id.to_string(),
            })
            .map_err(Self::lookup_error)
    }

    fn checkpoint(
        &mut self,
        volume_id: &str,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.ensure_running()?;
        self.connection
            .send_event(IndexServiceEvent::Checkpoint {
                volume_id: volume_id.to_string(),
                journal_id: journal_id.map(ToString::to_string),
                journal_cursor: journal_cursor.map(ToString::to_string),
            })
            .map_err(Self::lookup_error)
    }

    fn set_source_state(
        &mut self,
        volume_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), GlobalIndexError> {
        self.connection
            .send_event(IndexServiceEvent::SourceState {
                volume_id: volume_id.to_string(),
                status: status.to_string(),
                error: error.map(ToString::to_string),
            })
            .map_err(Self::lookup_error)
    }

    fn set_source_provider(
        &mut self,
        volume_id: &str,
        provider: &str,
    ) -> Result<(), GlobalIndexError> {
        self.ensure_running()?;
        self.connection
            .send_event(IndexServiceEvent::SourceProvider {
                volume_id: volume_id.to_string(),
                provider: provider.to_string(),
            })
            .map_err(Self::lookup_error)
    }

    fn resolve_parent_path(
        &mut self,
        volume_id: &str,
        parent_platform_file_id: &str,
    ) -> Result<Option<String>, GlobalIndexError> {
        self.ensure_running()?;
        let lookup_id = uuid::Uuid::new_v4().to_string();
        self.connection
            .send_event(IndexServiceEvent::ResolveParentPath {
                lookup_id: lookup_id.clone(),
                volume_id: volume_id.to_string(),
                parent_platform_file_id: parent_platform_file_id.to_string(),
            })
            .map_err(Self::lookup_error)?;
        match self
            .connection
            .read_lookup_response(&lookup_id)
            .map_err(Self::lookup_error)?
        {
            IndexServiceLookupResponse::ParentPath { path, .. } => Ok(path),
            IndexServiceLookupResponse::Entry { .. } => Err(Self::lookup_error(
                "unexpected entry lookup response".to_string(),
            )),
        }
    }

    fn find_entry_by_identity(
        &mut self,
        volume_id: &str,
        platform_file_id: &str,
        parent_platform_file_id: &str,
        name: &str,
    ) -> Result<Option<GlobalEntry>, GlobalIndexError> {
        self.ensure_running()?;
        let lookup_id = uuid::Uuid::new_v4().to_string();
        self.connection
            .send_event(IndexServiceEvent::FindEntryByIdentity {
                lookup_id: lookup_id.clone(),
                volume_id: volume_id.to_string(),
                platform_file_id: platform_file_id.to_string(),
                parent_platform_file_id: parent_platform_file_id.to_string(),
                name: name.to_string(),
            })
            .map_err(Self::lookup_error)?;
        match self
            .connection
            .read_lookup_response(&lookup_id)
            .map_err(Self::lookup_error)?
        {
            IndexServiceLookupResponse::Entry { entry, .. } => Ok(*entry),
            IndexServiceLookupResponse::ParentPath { .. } => Err(Self::lookup_error(
                "unexpected parent-path lookup response".to_string(),
            )),
        }
    }

    fn mark_volume_entries_stale(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
        self.ensure_running()?;
        self.connection
            .send_event(IndexServiceEvent::VolumeEntriesStale {
                volume_id: volume_id.to_string(),
            })
            .map_err(Self::lookup_error)
    }
}

fn service_handler(runtime: Arc<ServiceRuntime>) -> Arc<super::service::ServiceRequestHandler> {
    Arc::new(move |request, connection| runtime.handle_request(request, connection))
}

/// Entry point used by the bundled executable when invoked with
/// `--index-service`. It first tries the real Service Control Manager entry
/// point; running the same binary manually falls back to a console pipe host
/// so local development and diagnostics remain possible.
pub fn run_index_service_process() -> i32 {
    let runtime = Arc::new(ServiceRuntime::new());
    let _ = SERVICE_CONTEXT.set(runtime.clone());
    let mut service_name = volumes::to_wide(INDEX_SERVICE_NAME);
    let table = [
        windows_sys::Win32::System::Services::SERVICE_TABLE_ENTRYW {
            lpServiceName: service_name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        windows_sys::Win32::System::Services::SERVICE_TABLE_ENTRYW::default(),
    ];
    let started = unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } != 0;
    if started {
        return 0;
    }
    let error = unsafe { GetLastError() };
    if error != ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
        eprintln!("Zen Canvas index service dispatcher failed: {error}");
        return error as i32;
    }
    runtime.set_state("running", None);
    let result = super::service::serve_index_service_loop(
        runtime.stop.clone(),
        service_handler(runtime.clone()),
    );
    if let Err(error) = result {
        eprintln!("Zen Canvas index service console host failed: {error}");
        return 1;
    }
    0
}

unsafe extern "system" fn service_main(
    _argument_count: u32,
    _arguments: *mut windows_sys::core::PWSTR,
) {
    let Some(runtime) = SERVICE_CONTEXT.get().cloned() else {
        return;
    };
    let name = volumes::to_wide(INDEX_SERVICE_NAME);
    let status_handle =
        RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(service_control_handler), ptr::null());
    if status_handle.is_null() {
        return;
    }
    SERVICE_STATUS_HANDLE_VALUE.store(status_handle as usize, Ordering::Release);
    report_service_status(SERVICE_START_PENDING, 0, 1, 30_000);
    runtime.set_state("running", None);
    report_service_status(
        windows_sys::Win32::System::Services::SERVICE_RUNNING,
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN,
        0,
        0,
    );
    let result = super::service::serve_index_service_loop(
        runtime.stop.clone(),
        service_handler(runtime.clone()),
    );
    let exit_code = if result.is_ok() { 0 } else { 1 };
    if let Err(error) = result {
        runtime.set_state("error", Some(error));
    }
    report_service_status(SERVICE_STOPPED, 0, exit_code, 0);
}

unsafe extern "system" fn service_control_handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut std::ffi::c_void,
    _context: *mut std::ffi::c_void,
) -> u32 {
    if matches!(control, SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN) {
        if let Some(runtime) = SERVICE_CONTEXT.get() {
            runtime.request_stop();
        }
        // ConnectNamedPipe is intentionally synchronous so the transport
        // stays small and auditable. A local wake-up makes the SCM stop
        // callback unblock the accept loop immediately.
        std::thread::spawn(super::service::wake_service_pipe);
        report_service_status(SERVICE_STOP_PENDING, 0, 0, 15_000);
    }
    0
}

fn report_service_status(
    current_state: u32,
    controls_accepted: u32,
    exit_code: u32,
    wait_hint: u32,
) {
    let handle = SERVICE_STATUS_HANDLE_VALUE.load(Ordering::Acquire);
    if handle == 0 {
        return;
    }
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: current_state,
        dwControlsAccepted: controls_accepted,
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: if current_state == SERVICE_START_PENDING
            || current_state == SERVICE_STOP_PENDING
        {
            1
        } else {
            0
        },
        dwWaitHint: wait_hint,
    };
    unsafe {
        SetServiceStatus(handle as SERVICE_STATUS_HANDLE, &status);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsServiceStatus {
    pub installed: bool,
    pub state: String,
    pub process_id: u32,
    pub win32_exit_code: u32,
}

pub fn query_service_status() -> Result<WindowsServiceStatus, String> {
    let manager = open_manager(SC_MANAGER_CONNECT)?;
    let service = unsafe {
        OpenServiceW(
            manager,
            volumes::to_wide(INDEX_SERVICE_NAME).as_ptr(),
            SERVICE_QUERY_STATUS,
        )
    };
    if service.is_null() {
        let error = unsafe { GetLastError() };
        close_service_handles(manager, ptr::null_mut());
        if error == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok(WindowsServiceStatus {
                installed: false,
                state: "not_installed".to_string(),
                process_id: 0,
                win32_exit_code: 0,
            });
        }
        return Err(format!("OpenServiceW failed: {error}"));
    }
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0u32;
    let ok = unsafe {
        QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
            std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut needed,
        )
    } != 0;
    let error = if ok {
        None
    } else {
        Some(unsafe { GetLastError() })
    };
    close_service_handles(manager, service);
    if let Some(error) = error {
        return Err(format!("QueryServiceStatusEx failed: {error}"));
    }
    Ok(WindowsServiceStatus {
        installed: true,
        state: service_state_name(status.dwCurrentState).to_string(),
        process_id: status.dwProcessId,
        win32_exit_code: status.dwWin32ExitCode,
    })
}

pub fn install_service(executable: &Path) -> Result<(), String> {
    let executable = executable
        .canonicalize()
        .map_err(|error| format!("cannot resolve index service executable: {error}"))?;
    let command_line = format!(r#""{}" --index-service"#, executable.display());
    let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
    let service_name = volumes::to_wide(INDEX_SERVICE_NAME);
    let display_name = volumes::to_wide(INDEX_SERVICE_DISPLAY_NAME);
    let command_line = volumes::to_wide(&command_line);
    let desired_access = SERVICE_QUERY_STATUS
        | SERVICE_START
        | SERVICE_STOP
        | SERVICE_DELETE_ACCESS
        | SERVICE_CHANGE_CONFIG;
    let service = unsafe {
        CreateServiceW(
            manager,
            service_name.as_ptr(),
            display_name.as_ptr(),
            desired_access,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            command_line.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
        )
    };
    if !service.is_null() {
        close_service_handles(manager, service);
        return Ok(());
    }
    let error = unsafe { GetLastError() };
    if error != ERROR_SERVICE_EXISTS {
        close_service_handles(manager, ptr::null_mut());
        return Err(format!("CreateServiceW failed: {error}"));
    }
    let service = unsafe {
        OpenServiceW(
            manager,
            service_name.as_ptr(),
            SERVICE_CHANGE_CONFIG | SERVICE_QUERY_STATUS | SERVICE_START | SERVICE_STOP,
        )
    };
    if service.is_null() {
        let error = unsafe { GetLastError() };
        close_service_handles(manager, ptr::null_mut());
        return Err(format!("OpenServiceW failed while upgrading: {error}"));
    }
    let changed = unsafe {
        windows_sys::Win32::System::Services::ChangeServiceConfigW(
            service,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            command_line.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            display_name.as_ptr(),
        )
    } != 0;
    let error = if changed {
        None
    } else {
        Some(unsafe { GetLastError() })
    };
    close_service_handles(manager, service);
    if let Some(error) = error {
        Err(format!("ChangeServiceConfigW failed: {error}"))
    } else {
        Ok(())
    }
}

pub fn start_service() -> Result<(), String> {
    let manager = open_manager(SC_MANAGER_CONNECT)?;
    let service = open_service(manager, SERVICE_START | SERVICE_QUERY_STATUS)?;
    let started = unsafe { StartServiceW(service, 0, ptr::null()) } != 0;
    let error = if started {
        None
    } else {
        Some(unsafe { GetLastError() })
    };
    close_service_handles(manager, service);
    match error {
        None | Some(ERROR_SERVICE_ALREADY_RUNNING) => Ok(()),
        Some(error) => Err(format!("StartServiceW failed: {error}")),
    }
}

pub fn stop_service() -> Result<(), String> {
    let manager = open_manager(SC_MANAGER_CONNECT)?;
    let service = unsafe {
        OpenServiceW(
            manager,
            volumes::to_wide(INDEX_SERVICE_NAME).as_ptr(),
            SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    };
    if service.is_null() {
        let error = unsafe { GetLastError() };
        close_service_handles(manager, ptr::null_mut());
        if error == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok(());
        }
        return Err(format!("OpenServiceW failed: {error}"));
    }
    let mut status = SERVICE_STATUS::default();
    let stopped = unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) } != 0;
    let error = if stopped {
        None
    } else {
        Some(unsafe { GetLastError() })
    };
    close_service_handles(manager, service);
    match error {
        None | Some(ERROR_SERVICE_NOT_ACTIVE) => Ok(()),
        Some(error) => Err(format!("ControlService failed: {error}")),
    }
}

pub fn uninstall_service() -> Result<(), String> {
    stop_service()?;
    let manager = open_manager(SC_MANAGER_CONNECT)?;
    let service = unsafe {
        OpenServiceW(
            manager,
            volumes::to_wide(INDEX_SERVICE_NAME).as_ptr(),
            SERVICE_DELETE_ACCESS,
        )
    };
    if service.is_null() {
        let error = unsafe { GetLastError() };
        close_service_handles(manager, ptr::null_mut());
        if error == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok(());
        }
        return Err(format!("OpenServiceW failed: {error}"));
    }
    let deleted = unsafe { DeleteService(service) } != 0;
    let error = if deleted {
        None
    } else {
        Some(unsafe { GetLastError() })
    };
    close_service_handles(manager, service);
    if let Some(error) = error {
        Err(format!("DeleteService failed: {error}"))
    } else {
        Ok(())
    }
}

fn open_manager(access: u32) -> Result<SC_HANDLE, String> {
    let manager = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), access) };
    if manager.is_null() {
        Err(format!("OpenSCManagerW failed: {}", unsafe {
            GetLastError()
        }))
    } else {
        Ok(manager)
    }
}

fn open_service(manager: SC_HANDLE, access: u32) -> Result<SC_HANDLE, String> {
    let service = unsafe {
        OpenServiceW(
            manager,
            volumes::to_wide(INDEX_SERVICE_NAME).as_ptr(),
            access,
        )
    };
    if service.is_null() {
        Err(format!("OpenServiceW failed: {}", unsafe {
            GetLastError()
        }))
    } else {
        Ok(service)
    }
}

fn close_service_handles(manager: SC_HANDLE, service: SC_HANDLE) {
    unsafe {
        if !service.is_null() {
            CloseServiceHandle(service);
        }
        if !manager.is_null() {
            CloseServiceHandle(manager);
        }
    }
}

fn service_state_name(state: u32) -> &'static str {
    match state {
        SERVICE_START_PENDING => "start_pending",
        windows_sys::Win32::System::Services::SERVICE_RUNNING => "running",
        SERVICE_STOP_PENDING => "stop_pending",
        SERVICE_STOPPED => "stopped",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::coordinator::{GlobalIndexProvider, GlobalIndexSink};
    use crate::global_index::models::{
        INDEX_STATUS_DISCOVERED, PROVIDER_WINDOWS_MFT_USN, PROVIDER_WINDOWS_RECURSIVE_FALLBACK,
    };
    use crate::global_index::windows::service::{
        IndexServiceCallError, IndexServiceClientIo, IndexServiceFrame, IndexServiceLookupResponse,
    };
    use crate::global_index::windows::WindowsGlobalIndexProvider;
    use std::collections::VecDeque;

    fn index_request(request_id: &str, command: IndexServiceCommand) -> IndexServiceRequest {
        IndexServiceRequest {
            protocol_version: super::super::service::IPC_PROTOCOL_VERSION,
            request_id: request_id.to_string(),
            command,
            source: None,
        }
    }

    #[test]
    fn service_contract_uses_stable_name_and_console_switch() {
        assert_eq!(INDEX_SERVICE_NAME, "ZenCanvasGlobalIndex");
        assert!(format!(
            r#""{}" --index-service"#,
            r"C:\Program Files\Zen Canvas\Zen Canvas.exe"
        )
        .contains("--index-service"));
    }

    #[test]
    fn service_runtime_uses_the_shared_direct_mft_provider() {
        let runtime = ServiceRuntime::new();

        assert_eq!(
            runtime.provider.status().expect("direct provider status"),
            "windows_mft_usn"
        );
    }

    #[test]
    fn same_source_start_resume_and_rebuild_contenders_get_busy_without_service_fault_state() {
        let runtime = ServiceRuntime::new();
        let active = index_request(
            "request-active",
            IndexServiceCommand::StartInitialIndex {
                source_id: "source-a".to_string(),
            },
        );
        runtime
            .try_begin_index_request(&active)
            .expect("first operation owns the service slot");
        runtime.set_state("indexing", None);

        for (request_id, command) in [
            (
                "request-duplicate-start",
                IndexServiceCommand::StartInitialIndex {
                    source_id: "source-a".to_string(),
                },
            ),
            (
                "request-duplicate-resume",
                IndexServiceCommand::ResumeIncrementalSync {
                    source_id: "source-a".to_string(),
                },
            ),
            (
                "request-duplicate-rebuild",
                IndexServiceCommand::Rebuild {
                    source_id: "source-a".to_string(),
                },
            ),
        ] {
            let request = index_request(request_id, command);
            assert_eq!(source_id(&request.command), "source-a");
            let response = runtime
                .try_begin_index_request(&request)
                .expect_err("overlapping request must be rejected as busy");
            assert!(!response.ok);
            assert_eq!(response.request_id, request_id);
            assert_eq!(response.error_code.as_deref(), Some("index_service_busy"));
            assert_eq!(
                response.message.as_deref(),
                Some("Another global index operation is already running")
            );
            assert_eq!(response.status.as_deref(), Some("indexing"));
        }

        assert_eq!(runtime.status(), "indexing");
        let state = runtime.state.lock().expect("service lifecycle state");
        assert_eq!(state.status, "indexing");
        assert!(state.last_error.is_none());
    }

    #[test]
    fn source_id_is_extracted_only_from_index_commands() {
        assert_eq!(source_id(&IndexServiceCommand::Status), "");
        assert_eq!(
            source_id(&IndexServiceCommand::Rebuild {
                source_id: "volume".to_string()
            }),
            "volume"
        );
    }

    #[test]
    fn persisted_recursive_fallback_snapshot_is_accepted_for_native_normalization() {
        assert!(super::super::service::provider_snapshot_is_compatible(
            PROVIDER_WINDOWS_MFT_USN,
            PROVIDER_WINDOWS_RECURSIVE_FALLBACK
        ));
        assert!(!super::super::service::provider_snapshot_is_compatible(
            PROVIDER_WINDOWS_RECURSIVE_FALLBACK,
            PROVIDER_WINDOWS_MFT_USN
        ));

        let mut current = GlobalVolume {
            id: "volume".to_string(),
            platform: "windows".to_string(),
            stable_volume_id: "stable".to_string(),
            display_name: "C".to_string(),
            mount_path: "C:\\".to_string(),
            filesystem_type: "ntfs".to_string(),
            drive_kind: "fixed".to_string(),
            enabled: true,
            provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
            index_status: INDEX_STATUS_DISCOVERED.to_string(),
            last_error: None,
            journal_id: None,
            journal_cursor: None,
            last_full_index_at: None,
            last_incremental_sync_at: None,
            entry_count: 0,
            created_at: 1,
            updated_at: 1,
        };
        let mut snapshot = current.clone();
        snapshot.provider = PROVIDER_WINDOWS_RECURSIVE_FALLBACK.to_string();
        snapshot.index_status = "permission_required".to_string();
        snapshot.last_full_index_at = Some(10);
        current = merge_durable_source_state(current, &snapshot);
        assert_eq!(current.provider, PROVIDER_WINDOWS_MFT_USN);
        assert_eq!(current.index_status, "permission_required");
        assert_eq!(current.last_full_index_at, Some(10));
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ContractProviderOutcome {
        Complete,
        EmptyIncremental,
        PermissionFailure,
        RebuildFailure,
    }

    #[derive(Clone)]
    struct ContractProvider {
        source: GlobalSourceDescriptor,
        starts: Arc<AtomicUsize>,
        outcome: ContractProviderOutcome,
    }

    impl ContractProvider {
        fn new(source: GlobalSourceDescriptor) -> Self {
            Self {
                source,
                starts: Arc::new(AtomicUsize::new(0)),
                outcome: ContractProviderOutcome::Complete,
            }
        }

        fn with_outcome(source: GlobalSourceDescriptor, outcome: ContractProviderOutcome) -> Self {
            Self {
                source,
                starts: Arc::new(AtomicUsize::new(0)),
                outcome,
            }
        }
    }

    impl GlobalIndexProvider for ContractProvider {
        fn discover_sources(&self) -> Result<Vec<GlobalSourceDescriptor>, GlobalIndexError> {
            Ok(vec![self.source.clone()])
        }

        fn start_initial_index(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.starts.fetch_add(1, Ordering::AcqRel);
            sink.set_source_provider(&source.volume.id, PROVIDER_WINDOWS_MFT_USN)?;
            sink.mark_volume_entries_stale(&source.volume.id)?;
            match self.outcome {
                ContractProviderOutcome::PermissionFailure => {
                    sink.set_source_state(
                        &source.volume.id,
                        crate::global_index::models::INDEX_STATUS_PERMISSION_REQUIRED,
                        Some("fixture permission failure"),
                    )?;
                    return Err(GlobalIndexError::PermissionRequired(
                        "fixture permission failure".to_string(),
                    ));
                }
                ContractProviderOutcome::RebuildFailure => {
                    sink.set_source_state(
                        &source.volume.id,
                        crate::global_index::models::INDEX_STATUS_REBUILD_REQUIRED,
                        Some("fixture rebuild required"),
                    )?;
                    return Err(GlobalIndexError::RebuildRequired(
                        "fixture rebuild required".to_string(),
                    ));
                }
                ContractProviderOutcome::Complete | ContractProviderOutcome::EmptyIncremental => {}
            }
            sink.write_batch(&[GlobalEntryInput {
                volume_id: source.volume.id.clone(),
                platform_file_id: "fixture-file".to_string(),
                parent_platform_file_id: "fixture-root".to_string(),
                name: "fixture.txt".to_string(),
                path: "C:\\fixture.txt".to_string(),
                extension: "txt".to_string(),
                is_directory: false,
                size: 1,
                created_at_fs: None,
                modified_at_fs: None,
                file_attributes: 0,
                is_hidden: false,
                is_system: false,
                source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                last_seen_at: 1,
            }])?;
            sink.checkpoint(
                &source.volume.id,
                source.volume.journal_id.as_deref(),
                source.volume.journal_cursor.as_deref(),
            )?;
            Ok(())
        }

        fn resume_incremental_sync(
            &self,
            source: &GlobalSourceDescriptor,
            sink: &mut dyn GlobalIndexSink,
            _cancel: &AtomicBool,
        ) -> Result<(), GlobalIndexError> {
            self.starts.fetch_add(1, Ordering::AcqRel);
            sink.set_source_provider(&source.volume.id, PROVIDER_WINDOWS_MFT_USN)?;
            match self.outcome {
                ContractProviderOutcome::PermissionFailure => {
                    sink.set_source_state(
                        &source.volume.id,
                        crate::global_index::models::INDEX_STATUS_PERMISSION_REQUIRED,
                        Some("fixture permission failure"),
                    )?;
                    return Err(GlobalIndexError::PermissionRequired(
                        "fixture permission failure".to_string(),
                    ));
                }
                ContractProviderOutcome::RebuildFailure => {
                    sink.set_source_state(
                        &source.volume.id,
                        crate::global_index::models::INDEX_STATUS_REBUILD_REQUIRED,
                        Some("fixture rebuild required"),
                    )?;
                    return Err(GlobalIndexError::RebuildRequired(
                        "fixture rebuild required".to_string(),
                    ));
                }
                ContractProviderOutcome::EmptyIncremental => {}
                ContractProviderOutcome::Complete => {
                    sink.mark_entry_stale(&format!("entry-{}", source.volume.id))?;
                }
            }
            sink.checkpoint(
                &source.volume.id,
                source.volume.journal_id.as_deref(),
                source.volume.journal_cursor.as_deref(),
            )?;
            Ok(())
        }

        fn pause(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }

        fn status(&self) -> Result<String, GlobalIndexError> {
            Ok("contract_provider".to_string())
        }

        fn shutdown(&self) -> Result<(), GlobalIndexError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct ContractSink {
        actions: Vec<String>,
        fail_active_status_with_sqlite_busy: bool,
    }

    impl GlobalIndexSink for ContractSink {
        fn write_batch(&mut self, entries: &[GlobalEntryInput]) -> Result<usize, GlobalIndexError> {
            self.actions.push("entries".to_string());
            Ok(entries.len())
        }

        fn mark_entry_stale(&mut self, entry_id: &str) -> Result<(), GlobalIndexError> {
            self.actions.push(format!("entry_stale:{entry_id}"));
            Ok(())
        }

        fn checkpoint(
            &mut self,
            _volume_id: &str,
            _journal_id: Option<&str>,
            _journal_cursor: Option<&str>,
        ) -> Result<(), GlobalIndexError> {
            self.actions.push("checkpoint".to_string());
            Ok(())
        }

        fn set_source_state(
            &mut self,
            _volume_id: &str,
            status: &str,
            _error: Option<&str>,
        ) -> Result<(), GlobalIndexError> {
            if self.fail_active_status_with_sqlite_busy
                && (status == crate::global_index::models::INDEX_STATUS_INDEXING
                    || status == crate::global_index::models::INDEX_STATUS_SYNCING)
            {
                return Err(GlobalIndexError::Database(crate::db::DbError::Sqlite(
                    rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                        None,
                    ),
                )));
            }
            self.actions.push(format!("source_state:{status}"));
            Ok(())
        }

        fn set_source_provider(
            &mut self,
            _volume_id: &str,
            provider: &str,
        ) -> Result<(), GlobalIndexError> {
            self.actions.push(format!("source_provider:{provider}"));
            Ok(())
        }

        fn resolve_parent_path(
            &mut self,
            _volume_id: &str,
            _parent_platform_file_id: &str,
        ) -> Result<Option<String>, GlobalIndexError> {
            Ok(None)
        }

        fn find_entry_by_identity(
            &mut self,
            _volume_id: &str,
            _platform_file_id: &str,
            _parent_platform_file_id: &str,
            _name: &str,
        ) -> Result<Option<GlobalEntry>, GlobalIndexError> {
            Ok(None)
        }

        fn mark_volume_entries_stale(&mut self, volume_id: &str) -> Result<(), GlobalIndexError> {
            self.actions.push(format!("volume_stale:{volume_id}"));
            Ok(())
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum ContractMutation {
        None,
        AdmissionOnlySuccess,
        DropAdmission,
        MissingSourceProvider,
        FullScanMissingVolumeStale,
        MissingCheckpoint,
        PayloadAfterCheckpoint,
        LateAdmission,
        WrongEventRequestId,
        WrongAdmissionIdentity,
        WrongAdmissionCardinality,
        WrongEventVolume,
        WrongEntryStaleVolume,
        EntriesBeforeVolumeStale,
        WrongProvider,
        DuplicateAdmission,
        SourceStateBeforeData,
        ReadyStateBeforeProvider,
        ServiceBusyAfterAdmission,
        WrongResponseProtocol,
        WrongResponseRequestId,
    }

    struct ServiceClientTransport {
        runtime: Arc<ServiceRuntime>,
        request_id: String,
        frames: VecDeque<IndexServiceFrame>,
        service_frames: Vec<IndexServiceFrame>,
        lookup_responses: Vec<IndexServiceLookupResponse>,
        mutation: ContractMutation,
        mutated: bool,
        disconnect_after_admission: bool,
        cancel_after_admission: Option<Arc<AtomicBool>>,
        frames_read: usize,
    }

    impl ServiceClientTransport {
        fn new(
            runtime: Arc<ServiceRuntime>,
            mutation: ContractMutation,
            disconnect_after_admission: bool,
            cancel_after_admission: Option<Arc<AtomicBool>>,
        ) -> Self {
            Self {
                runtime,
                request_id: String::new(),
                frames: VecDeque::new(),
                service_frames: Vec::new(),
                lookup_responses: Vec::new(),
                mutation,
                mutated: false,
                disconnect_after_admission,
                cancel_after_admission,
                frames_read: 0,
            }
        }

        fn apply_mutation(&mut self) {
            if self.mutated {
                return;
            }
            self.mutated = true;
            let admission_index = self.frames.iter().position(|frame| {
                matches!(
                    frame,
                    IndexServiceFrame::Event {
                        event: IndexServiceEvent::Sources { .. },
                        ..
                    }
                )
            });
            match self.mutation {
                ContractMutation::None => {}
                ContractMutation::AdmissionOnlySuccess => {
                    let admission = self
                        .frames
                        .iter()
                        .find(|frame| {
                            matches!(
                                frame,
                                IndexServiceFrame::Event {
                                    event: IndexServiceEvent::Sources { .. },
                                    ..
                                }
                            )
                        })
                        .cloned();
                    let response = self
                        .frames
                        .iter()
                        .find(|frame| matches!(frame, IndexServiceFrame::Response { .. }))
                        .cloned();
                    self.frames = admission.into_iter().chain(response).collect();
                }
                ContractMutation::DropAdmission => {
                    self.frames.retain(|frame| {
                        !matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Sources { .. },
                                ..
                            }
                        )
                    });
                }
                ContractMutation::MissingSourceProvider => {
                    self.frames.retain(|frame| {
                        !matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::SourceProvider { .. },
                                ..
                            }
                        )
                    });
                }
                ContractMutation::FullScanMissingVolumeStale => {
                    self.frames.retain(|frame| {
                        !matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::VolumeEntriesStale { .. }
                                    | IndexServiceEvent::Entries { .. },
                                ..
                            }
                        )
                    });
                }
                ContractMutation::MissingCheckpoint => {
                    self.frames.retain(|frame| {
                        !matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Checkpoint { .. },
                                ..
                            }
                        )
                    });
                }
                ContractMutation::PayloadAfterCheckpoint => {
                    let checkpoint_index = self.frames.iter().position(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Checkpoint { .. },
                                ..
                            }
                        )
                    });
                    let entries_index = self.frames.iter().position(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Entries { .. },
                                ..
                            }
                        )
                    });
                    if let (Some(checkpoint_index), Some(entries_index)) =
                        (checkpoint_index, entries_index)
                    {
                        self.frames.swap(checkpoint_index, entries_index);
                    }
                }
                ContractMutation::LateAdmission => {
                    if let Some(index) = admission_index {
                        let admission = self.frames.remove(index).expect("admission frame");
                        self.frames
                            .insert((index + 1).min(self.frames.len()), admission);
                    }
                }
                ContractMutation::WrongEventRequestId => {
                    if let Some(IndexServiceFrame::Event { request_id, .. }) = self
                        .frames
                        .iter_mut()
                        .find(|frame| matches!(frame, IndexServiceFrame::Event { .. }))
                    {
                        *request_id = "replayed-request-id".to_string();
                    }
                }
                ContractMutation::WrongAdmissionIdentity => {
                    if let Some(IndexServiceFrame::Event {
                        event: IndexServiceEvent::Sources { sources },
                        ..
                    }) = self.frames.iter_mut().find(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Sources { .. },
                                ..
                            }
                        )
                    }) {
                        sources[0].stable_volume_id = "different-volume-identity".to_string();
                    }
                }
                ContractMutation::WrongAdmissionCardinality => {
                    if let Some(IndexServiceFrame::Event {
                        event: IndexServiceEvent::Sources { sources },
                        ..
                    }) = self.frames.iter_mut().find(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Sources { .. },
                                ..
                            }
                        )
                    }) {
                        sources.push(sources[0].clone());
                    }
                }
                ContractMutation::WrongEventVolume => {
                    if let Some(IndexServiceFrame::Event {
                        event: IndexServiceEvent::VolumeEntriesStale { volume_id },
                        ..
                    }) = self.frames.iter_mut().find(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::VolumeEntriesStale { .. },
                                ..
                            }
                        )
                    }) {
                        *volume_id = "different-volume".to_string();
                    }
                }
                ContractMutation::EntriesBeforeVolumeStale => {
                    let stale_index = self.frames.iter().position(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::VolumeEntriesStale { .. },
                                ..
                            }
                        )
                    });
                    let entries_index = self.frames.iter().position(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::Entries { .. },
                                ..
                            }
                        )
                    });
                    if let (Some(stale_index), Some(entries_index)) = (stale_index, entries_index) {
                        self.frames.swap(stale_index, entries_index);
                    }
                }
                ContractMutation::WrongEntryStaleVolume => {
                    if let Some(IndexServiceFrame::Event {
                        event: IndexServiceEvent::EntryStale { volume_id, .. },
                        ..
                    }) = self.frames.iter_mut().find(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::EntryStale { .. },
                                ..
                            }
                        )
                    }) {
                        *volume_id = "different-volume".to_string();
                    }
                }
                ContractMutation::WrongProvider => {
                    if let Some(IndexServiceFrame::Event {
                        event: IndexServiceEvent::SourceProvider { provider, .. },
                        ..
                    }) = self.frames.iter_mut().find(|frame| {
                        matches!(
                            frame,
                            IndexServiceFrame::Event {
                                event: IndexServiceEvent::SourceProvider { .. },
                                ..
                            }
                        )
                    }) {
                        *provider = "unexpected-provider".to_string();
                    }
                }
                ContractMutation::DuplicateAdmission => {
                    if let Some(index) = admission_index {
                        let duplicate = self.frames[index].clone();
                        self.frames.insert(index + 1, duplicate);
                    }
                }
                ContractMutation::SourceStateBeforeData => {
                    if let Some(IndexServiceFrame::Event { event, .. }) =
                        self.frames.iter_mut().find(|frame| {
                            matches!(
                                frame,
                                IndexServiceFrame::Event {
                                    event: IndexServiceEvent::SourceProvider { .. },
                                    ..
                                }
                            )
                        })
                    {
                        let volume_id = match event {
                            IndexServiceEvent::SourceProvider { volume_id, .. } => {
                                volume_id.clone()
                            }
                            _ => unreachable!(),
                        };
                        *event = IndexServiceEvent::SourceState {
                            volume_id,
                            status: "permission_required".to_string(),
                            error: Some("terminal_state_fixture".to_string()),
                        };
                    }
                }
                ContractMutation::ReadyStateBeforeProvider => {
                    if let Some(IndexServiceFrame::Event { event, .. }) =
                        self.frames.iter_mut().find(|frame| {
                            matches!(
                                frame,
                                IndexServiceFrame::Event {
                                    event: IndexServiceEvent::SourceProvider { .. },
                                    ..
                                }
                            )
                        })
                    {
                        let volume_id = match event {
                            IndexServiceEvent::SourceProvider { volume_id, .. } => {
                                volume_id.clone()
                            }
                            _ => unreachable!(),
                        };
                        *event = IndexServiceEvent::SourceState {
                            volume_id,
                            status: crate::global_index::models::INDEX_STATUS_READY.to_string(),
                            error: None,
                        };
                    }
                }
                ContractMutation::ServiceBusyAfterAdmission => {
                    if let Some(IndexServiceFrame::Response { response }) = self
                        .frames
                        .iter_mut()
                        .find(|frame| matches!(frame, IndexServiceFrame::Response { .. }))
                    {
                        response.ok = false;
                        response.error_code = Some("index_service_busy".to_string());
                    }
                }
                ContractMutation::WrongResponseProtocol => {
                    if let Some(IndexServiceFrame::Response { response }) = self
                        .frames
                        .iter_mut()
                        .find(|frame| matches!(frame, IndexServiceFrame::Response { .. }))
                    {
                        response.protocol_version += 1;
                    }
                }
                ContractMutation::WrongResponseRequestId => {
                    if let Some(IndexServiceFrame::Response { response }) = self
                        .frames
                        .iter_mut()
                        .find(|frame| matches!(frame, IndexServiceFrame::Response { .. }))
                    {
                        response.request_id = "replayed-request-id".to_string();
                    }
                }
            }
        }
    }

    impl IndexServiceServerIo for ServiceClientTransport {
        fn send_event(&mut self, event: IndexServiceEvent) -> Result<(), String> {
            let frame = IndexServiceFrame::Event {
                request_id: self.request_id.clone(),
                event,
            };
            self.service_frames.push(frame.clone());
            self.frames.push_back(frame);
            Ok(())
        }

        fn send_response(&mut self, response: IndexServiceResponse) -> Result<(), String> {
            let frame = IndexServiceFrame::Response { response };
            self.service_frames.push(frame.clone());
            self.frames.push_back(frame);
            Ok(())
        }

        fn read_lookup_response(
            &mut self,
            expected_lookup_id: &str,
        ) -> Result<IndexServiceLookupResponse, String> {
            Err(format!("unexpected test lookup {expected_lookup_id}"))
        }
    }

    impl IndexServiceClientIo for ServiceClientTransport {
        fn send_request(&mut self, request: &IndexServiceRequest) -> Result<(), String> {
            self.request_id = request.request_id.clone();
            let runtime = self.runtime.clone();
            runtime.handle_request(request.clone(), self)?;
            self.apply_mutation();
            Ok(())
        }

        fn read_frame(&mut self) -> Result<IndexServiceFrame, String> {
            if self.disconnect_after_admission && self.frames_read == 1 {
                return Err("simulated_service_disconnect_after_admission".to_string());
            }
            let frame = self
                .frames
                .pop_front()
                .ok_or_else(|| "simulated_service_stream_ended".to_string())?;
            self.frames_read += 1;
            if matches!(
                &frame,
                IndexServiceFrame::Event {
                    event: IndexServiceEvent::Sources { .. },
                    ..
                }
            ) {
                if let Some(cancel) = &self.cancel_after_admission {
                    cancel.store(true, Ordering::Release);
                }
            }
            Ok(frame)
        }

        fn send_lookup_response(
            &mut self,
            response: IndexServiceLookupResponse,
        ) -> Result<(), String> {
            self.lookup_responses.push(response);
            Ok(())
        }
    }

    fn contract_source(enabled: bool) -> GlobalSourceDescriptor {
        GlobalSourceDescriptor {
            volume: GlobalVolume {
                id: "volume-a".to_string(),
                platform: "windows".to_string(),
                stable_volume_id: "stable-volume-a".to_string(),
                display_name: "C".to_string(),
                mount_path: "C:\\".to_string(),
                filesystem_type: "ntfs".to_string(),
                drive_kind: "fixed".to_string(),
                enabled,
                provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
                index_status: "ready".to_string(),
                last_error: None,
                journal_id: Some("journal-a".to_string()),
                journal_cursor: Some("44".to_string()),
                last_full_index_at: Some(10),
                last_incremental_sync_at: Some(9),
                entry_count: 1,
                created_at: 1,
                updated_at: 11,
            },
        }
    }

    fn rebuild_request(source: GlobalSourceDescriptor) -> IndexServiceRequest {
        IndexServiceRequest::new(
            IndexServiceCommand::Rebuild {
                source_id: source.volume.id.clone(),
            },
            Some(source.volume),
        )
    }

    fn initial_request(source: GlobalSourceDescriptor) -> IndexServiceRequest {
        IndexServiceRequest::new(
            IndexServiceCommand::StartInitialIndex {
                source_id: source.volume.id.clone(),
            },
            Some(source.volume),
        )
    }

    fn resume_request(source: GlobalSourceDescriptor) -> IndexServiceRequest {
        IndexServiceRequest::new(
            IndexServiceCommand::ResumeIncrementalSync {
                source_id: source.volume.id.clone(),
            },
            Some(source.volume),
        )
    }

    fn contract_runtime(
        source: GlobalSourceDescriptor,
    ) -> (Arc<ServiceRuntime>, Arc<ContractProvider>) {
        let provider = Arc::new(ContractProvider::new(source));
        let runtime = Arc::new(ServiceRuntime::with_provider(provider.clone()));
        (runtime, provider)
    }

    fn contract_runtime_with_outcome(
        source: GlobalSourceDescriptor,
        outcome: ContractProviderOutcome,
    ) -> (Arc<ServiceRuntime>, Arc<ContractProvider>) {
        let provider = Arc::new(ContractProvider::with_outcome(source, outcome));
        let runtime = Arc::new(ServiceRuntime::with_provider(provider.clone()));
        (runtime, provider)
    }

    struct ContractExecution<'a> {
        runtime: Arc<ServiceRuntime>,
        request: &'a IndexServiceRequest,
        sink: &'a mut ContractSink,
        cancel: Arc<AtomicBool>,
        mutation: ContractMutation,
        disconnect_after_admission: bool,
        cancel_after_admission: Option<Arc<AtomicBool>>,
        direct_calls: &'a mut usize,
    }

    fn execute_contract_request(
        execution: ContractExecution<'_>,
    ) -> (Result<(), GlobalIndexError>, ServiceClientTransport) {
        let ContractExecution {
            runtime,
            request,
            sink,
            cancel,
            mutation,
            disconnect_after_admission,
            cancel_after_admission,
            direct_calls,
        } = execution;
        let desktop_provider = WindowsGlobalIndexProvider::new();
        let mut transport = ServiceClientTransport::new(
            runtime,
            mutation,
            disconnect_after_admission,
            cancel_after_admission,
        );
        let result = desktop_provider.run_with_fallback_using(
            request,
            sink,
            &cancel,
            |_direct, _sink| {
                *direct_calls += 1;
                Ok(())
            },
            |provider, request, sink, cancel| {
                provider.run_service_stream_with_client(request, sink, cancel, &mut transport)
            },
        );
        (result, transport)
    }

    #[test]
    fn real_service_client_path_admits_before_persisting_active_state_or_index_events() {
        let source = contract_source(true);
        let (runtime, provider) = contract_runtime(source.clone());
        let request = rebuild_request(source);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });

        assert!(result.is_ok());
        assert_eq!(direct_calls, 0);
        assert_eq!(provider.starts.load(Ordering::Acquire), 1);
        assert!(matches!(
            transport.service_frames.first(),
            Some(IndexServiceFrame::Event {
                request_id,
                event: IndexServiceEvent::Sources { sources },
            }) if request_id == &request.request_id && sources.len() == 1
        ));
        assert_eq!(
            sink.actions,
            [
                "source_state:syncing",
                "source_provider:windows_mft_usn",
                "volume_stale:volume-a",
                "entries",
                "checkpoint",
            ]
        );
    }

    #[test]
    fn successful_response_requires_command_complete_event_proof() {
        for command in ["initial", "rebuild"] {
            for mutation in [
                ContractMutation::AdmissionOnlySuccess,
                ContractMutation::MissingSourceProvider,
                ContractMutation::FullScanMissingVolumeStale,
                ContractMutation::MissingCheckpoint,
                ContractMutation::PayloadAfterCheckpoint,
            ] {
                let mut source = contract_source(true);
                if command == "initial" {
                    source.volume.last_full_index_at = None;
                    source.volume.last_incremental_sync_at = None;
                }
                let expected_active = if command == "initial" {
                    "source_state:indexing"
                } else {
                    "source_state:syncing"
                };
                let (runtime, _provider) = contract_runtime(source.clone());
                let request = if command == "initial" {
                    initial_request(source)
                } else {
                    rebuild_request(source)
                };
                let mut sink = ContractSink::default();
                let mut direct_calls = 0;
                let (result, _transport) = execute_contract_request(ContractExecution {
                    runtime,
                    request: &request,
                    sink: &mut sink,
                    cancel: Arc::new(AtomicBool::new(false)),
                    mutation,
                    disconnect_after_admission: false,
                    cancel_after_admission: None,
                    direct_calls: &mut direct_calls,
                });

                assert!(
                    matches!(result, Err(GlobalIndexError::ServiceOutcomeUnknown(_))),
                    "{command} {mutation:?}"
                );
                assert_eq!(direct_calls, 0, "{command} {mutation:?}");
                assert!(
                    !sink
                        .actions
                        .iter()
                        .any(|action| action == "source_state:ready"),
                    "{command} {mutation:?}"
                );
                match mutation {
                    ContractMutation::AdmissionOnlySuccess
                    | ContractMutation::MissingSourceProvider => {
                        assert_eq!(sink.actions, [expected_active], "{command}");
                    }
                    ContractMutation::FullScanMissingVolumeStale => {
                        assert_eq!(
                            sink.actions,
                            [
                                expected_active,
                                "source_provider:windows_mft_usn",
                                "checkpoint",
                            ],
                            "{command}"
                        );
                    }
                    ContractMutation::MissingCheckpoint => {
                        assert_eq!(
                            sink.actions,
                            [
                                expected_active,
                                "source_provider:windows_mft_usn",
                                "volume_stale:volume-a",
                                "entries",
                            ],
                            "{command}"
                        );
                    }
                    ContractMutation::PayloadAfterCheckpoint => {
                        assert_eq!(
                            sink.actions,
                            [
                                expected_active,
                                "source_provider:windows_mft_usn",
                                "volume_stale:volume-a",
                                "checkpoint",
                                "entries",
                            ],
                            "{command}"
                        );
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn empty_incremental_sync_completes_with_provider_and_checkpoint_only() {
        let source = contract_source(true);
        let (runtime, _provider) = contract_runtime_with_outcome(
            source.clone(),
            ContractProviderOutcome::EmptyIncremental,
        );
        let request = resume_request(source);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });

        assert!(result.is_ok());
        assert_eq!(direct_calls, 0);
        assert_eq!(
            sink.actions,
            [
                "source_state:syncing",
                "source_provider:windows_mft_usn",
                "checkpoint"
            ]
        );
        assert!(!transport.service_frames.iter().any(|frame| matches!(
            frame,
            IndexServiceFrame::Event {
                event: IndexServiceEvent::Entries { .. }
                    | IndexServiceEvent::EntryStale { .. }
                    | IndexServiceEvent::VolumeEntriesStale { .. },
                ..
            }
        )));
    }

    #[test]
    fn real_provider_failure_source_states_are_forwarded_after_provider_activity() {
        for (outcome, request_kind, expected_status) in [
            (
                ContractProviderOutcome::PermissionFailure,
                "rebuild",
                "source_state:permission_required",
            ),
            (
                ContractProviderOutcome::RebuildFailure,
                "resume",
                "source_state:rebuild_required",
            ),
        ] {
            let source = contract_source(true);
            let (runtime, _provider) = contract_runtime_with_outcome(source.clone(), outcome);
            let request = if request_kind == "rebuild" {
                rebuild_request(source)
            } else {
                resume_request(source)
            };
            let mut sink = ContractSink::default();
            let mut direct_calls = 0;
            let (result, _transport) = execute_contract_request(ContractExecution {
                runtime,
                request: &request,
                sink: &mut sink,
                cancel: Arc::new(AtomicBool::new(false)),
                mutation: ContractMutation::None,
                disconnect_after_admission: false,
                cancel_after_admission: None,
                direct_calls: &mut direct_calls,
            });

            match outcome {
                ContractProviderOutcome::PermissionFailure => {
                    assert!(matches!(
                        result,
                        Err(GlobalIndexError::PermissionRequired(_))
                    ));
                }
                ContractProviderOutcome::RebuildFailure => {
                    assert!(matches!(result, Err(GlobalIndexError::RebuildRequired(_))));
                }
                _ => unreachable!(),
            }
            assert_eq!(direct_calls, 0);
            assert!(sink.actions.contains(&expected_status.to_string()));
            assert!(sink
                .actions
                .contains(&"source_provider:windows_mft_usn".to_string()));
            if request_kind == "rebuild" {
                assert!(sink.actions.contains(&"volume_stale:volume-a".to_string()));
            }
        }
    }

    #[test]
    fn real_service_busy_is_typed_and_never_runs_direct_or_mutates_source_state() {
        let source = contract_source(true);
        let (runtime, provider) = contract_runtime(source.clone());
        runtime.busy.store(true, Ordering::Release);
        let request = rebuild_request(source);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });

        assert!(matches!(result, Err(GlobalIndexError::ServiceBusy)));
        assert_eq!(direct_calls, 0);
        assert_eq!(provider.starts.load(Ordering::Acquire), 0);
        assert!(sink.actions.is_empty());
        assert!(matches!(
            transport.service_frames.as_slice(),
            [IndexServiceFrame::Response { response }]
                if response.error_code.as_deref() == Some("index_service_busy")
        ));
    }

    #[test]
    fn local_sqlite_busy_while_applying_admission_remains_a_database_error() {
        let source = contract_source(true);
        let (runtime, provider) = contract_runtime(source.clone());
        let request = rebuild_request(source);
        let mut sink = ContractSink {
            fail_active_status_with_sqlite_busy: true,
            ..ContractSink::default()
        };
        let mut direct_calls = 0;
        let (result, _transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });

        assert!(matches!(
            result,
            Err(GlobalIndexError::Database(crate::db::DbError::Sqlite(
                rusqlite::Error::SqliteFailure(error, _)
            ))) if error.code == rusqlite::ErrorCode::DatabaseBusy
        ));
        assert_eq!(direct_calls, 0);
        assert_eq!(provider.starts.load(Ordering::Acquire), 1);
        assert!(sink.actions.is_empty());
    }

    #[test]
    fn direct_fallback_is_permitted_only_before_request_dispatch() {
        let source = contract_source(true);
        let request = rebuild_request(source);
        let provider = WindowsGlobalIndexProvider::new();
        let cancel = AtomicBool::new(false);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;

        let result = provider.run_with_fallback_using(
            &request,
            &mut sink,
            &cancel,
            |_direct, _sink| {
                direct_calls += 1;
                Ok(())
            },
            |_provider, _request, _sink, _cancel| {
                Err(IndexServiceCallError::NotSent(
                    "named pipe was unavailable before sending request".to_string(),
                ))
            },
        );

        assert!(result.is_ok());
        assert_eq!(direct_calls, 1);
        assert_eq!(sink.actions, ["source_state:unavailable"]);
    }

    #[test]
    fn old_v3_service_without_admission_fails_closed_before_the_sink() {
        let source = contract_source(true);
        let (runtime, provider) = contract_runtime(source.clone());
        let request = rebuild_request(source);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, _transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::DropAdmission,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });

        assert!(matches!(
            result,
            Err(GlobalIndexError::ServiceAdmissionMissing(_))
        ));
        assert_eq!(direct_calls, 0);
        assert_eq!(provider.starts.load(Ordering::Acquire), 1);
        assert!(sink.actions.is_empty());
    }

    #[test]
    fn request_event_source_and_order_mismatches_are_rejected_before_each_sink_write() {
        for (mutation, expected_action_count) in [
            (ContractMutation::WrongEventRequestId, 0),
            (ContractMutation::WrongAdmissionIdentity, 0),
            (ContractMutation::WrongAdmissionCardinality, 0),
            (ContractMutation::WrongProvider, 1),
            (ContractMutation::WrongEventVolume, 2),
            (ContractMutation::WrongEntryStaleVolume, 2),
            (ContractMutation::EntriesBeforeVolumeStale, 2),
            (ContractMutation::DuplicateAdmission, 1),
            (ContractMutation::SourceStateBeforeData, 1),
            (ContractMutation::ReadyStateBeforeProvider, 1),
            (ContractMutation::LateAdmission, 0),
        ] {
            let source = contract_source(true);
            let (runtime, _provider) = contract_runtime(source.clone());
            let request = if matches!(mutation, ContractMutation::WrongEntryStaleVolume) {
                resume_request(source)
            } else {
                rebuild_request(source)
            };
            let mut sink = ContractSink::default();
            let mut direct_calls = 0;
            let (result, _transport) = execute_contract_request(ContractExecution {
                runtime,
                request: &request,
                sink: &mut sink,
                cancel: Arc::new(AtomicBool::new(false)),
                mutation,
                disconnect_after_admission: false,
                cancel_after_admission: None,
                direct_calls: &mut direct_calls,
            });

            if matches!(mutation, ContractMutation::LateAdmission) {
                assert!(matches!(
                    result,
                    Err(GlobalIndexError::ServiceAdmissionMissing(_))
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(GlobalIndexError::ServiceOutcomeUnknown(_))
                ));
            }
            assert_eq!(sink.actions.len(), expected_action_count, "{mutation:?}");
            assert_eq!(direct_calls, 0, "{mutation:?}");
        }
    }

    #[test]
    fn response_identity_mismatches_disconnect_and_cancel_never_use_direct_fallback() {
        for mutation in [
            ContractMutation::ServiceBusyAfterAdmission,
            ContractMutation::WrongResponseProtocol,
            ContractMutation::WrongResponseRequestId,
        ] {
            let source = contract_source(true);
            let (runtime, _provider) = contract_runtime(source.clone());
            let request = rebuild_request(source);
            let mut sink = ContractSink::default();
            let mut direct_calls = 0;
            let (result, _transport) = execute_contract_request(ContractExecution {
                runtime,
                request: &request,
                sink: &mut sink,
                cancel: Arc::new(AtomicBool::new(false)),
                mutation,
                disconnect_after_admission: false,
                cancel_after_admission: None,
                direct_calls: &mut direct_calls,
            });
            assert!(matches!(
                result,
                Err(GlobalIndexError::ServiceOutcomeUnknown(_))
            ));
            assert_eq!(direct_calls, 0);
        }

        let source = contract_source(true);
        let (runtime, _provider) = contract_runtime(source.clone());
        let request = rebuild_request(source);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, _transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: true,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });
        assert!(matches!(
            result,
            Err(GlobalIndexError::ServiceOutcomeUnknown(_))
        ));
        assert_eq!(direct_calls, 0);
        assert_eq!(sink.actions, ["source_state:syncing"]);

        let source = contract_source(true);
        let (runtime, _provider) = contract_runtime(source.clone());
        let request = rebuild_request(source);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, _transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: cancel.clone(),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: Some(cancel),
            direct_calls: &mut direct_calls,
        });
        assert!(matches!(
            result,
            Err(GlobalIndexError::ServiceOutcomeUnknown(_))
        ));
        assert_eq!(direct_calls, 0);
        assert!(sink.actions.is_empty());
    }

    #[test]
    fn disabled_service_source_and_mismatched_request_snapshot_fail_closed() {
        let service_source = contract_source(false);
        let (runtime, provider) = contract_runtime(service_source);
        let request = rebuild_request(contract_source(true));
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });
        assert!(matches!(result, Err(GlobalIndexError::Provider(_))));
        assert_eq!(provider.starts.load(Ordering::Acquire), 0);
        assert_eq!(direct_calls, 0);
        assert!(sink.actions.is_empty());
        assert!(!transport.service_frames.iter().any(|frame| matches!(
            frame,
            IndexServiceFrame::Event {
                event: IndexServiceEvent::Sources { .. },
                ..
            }
        )));

        let requested_source = contract_source(true);
        let mut changed_source = contract_source(true);
        changed_source.volume.stable_volume_id = "changed-stable-volume".to_string();
        let (runtime, provider) = contract_runtime(changed_source);
        let request = rebuild_request(requested_source);
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &request,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });
        assert!(matches!(result, Err(GlobalIndexError::Provider(_))));
        assert_eq!(provider.starts.load(Ordering::Acquire), 0);
        assert_eq!(direct_calls, 0);
        assert!(sink.actions.is_empty());
        assert!(!transport.service_frames.iter().any(|frame| matches!(
            frame,
            IndexServiceFrame::Event {
                event: IndexServiceEvent::Sources { .. },
                ..
            }
        )));

        let source = contract_source(true);
        let (runtime, provider) = contract_runtime(source.clone());
        let mut mismatched = rebuild_request(source);
        mismatched.command = IndexServiceCommand::Rebuild {
            source_id: "different-volume".to_string(),
        };
        let mut sink = ContractSink::default();
        let mut direct_calls = 0;
        let (result, transport) = execute_contract_request(ContractExecution {
            runtime,
            request: &mismatched,
            sink: &mut sink,
            cancel: Arc::new(AtomicBool::new(false)),
            mutation: ContractMutation::None,
            disconnect_after_admission: false,
            cancel_after_admission: None,
            direct_calls: &mut direct_calls,
        });
        assert!(matches!(result, Err(GlobalIndexError::Provider(_))));
        assert_eq!(provider.starts.load(Ordering::Acquire), 0);
        assert_eq!(direct_calls, 0);
        assert!(transport.service_frames.is_empty());
        assert!(sink.actions.is_empty());
    }

    #[test]
    fn invalid_request_protocol_or_request_id_never_dispatches_or_falls_back() {
        for invalid_field in ["protocol_version", "request_id"] {
            let source = contract_source(true);
            let (runtime, provider) = contract_runtime(source.clone());
            let mut request = rebuild_request(source);
            match invalid_field {
                "protocol_version" => request.protocol_version += 1,
                "request_id" => request.request_id.clear(),
                _ => unreachable!(),
            }
            let mut sink = ContractSink::default();
            let mut direct_calls = 0;
            let (result, transport) = execute_contract_request(ContractExecution {
                runtime,
                request: &request,
                sink: &mut sink,
                cancel: Arc::new(AtomicBool::new(false)),
                mutation: ContractMutation::None,
                disconnect_after_admission: false,
                cancel_after_admission: None,
                direct_calls: &mut direct_calls,
            });

            assert!(matches!(result, Err(GlobalIndexError::Provider(_))));
            assert_eq!(provider.starts.load(Ordering::Acquire), 0);
            assert_eq!(direct_calls, 0);
            assert!(sink.actions.is_empty());
            assert!(transport.service_frames.is_empty());
        }
    }
}
