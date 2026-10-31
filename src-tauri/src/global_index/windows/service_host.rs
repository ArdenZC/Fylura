//! Windows Service Control Manager host for the global index provider.
//!
//! The installed service is deliberately small: it owns volume discovery and
//! metadata enumeration, and exposes only the versioned named-pipe protocol in
//! `service.rs`. It never receives arbitrary file-operation commands.

use super::service::{
    IndexServiceCommand, IndexServiceConnection, IndexServiceEvent, IndexServiceLookupResponse,
    IndexServiceRequest, IndexServiceResponse,
};
use super::{volumes, DirectWindowsGlobalIndexProvider};
use crate::global_index::coordinator::{GlobalIndexError, GlobalIndexProvider, GlobalIndexSink};
use crate::global_index::models::{
    GlobalEntry, GlobalEntryInput, GlobalSourceDescriptor, GlobalVolume, INDEX_STATUS_PAUSED,
    INDEX_STATUS_READY, PROVIDER_WINDOWS_MFT_USN, PROVIDER_WINDOWS_RECURSIVE_FALLBACK,
    SCOPE_AUTHORIZATION_AUTHORIZED,
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

pub const INDEX_SERVICE_NAME: &str = "FyluraGlobalIndex";
pub const INDEX_SERVICE_DISPLAY_NAME: &str = "Fylura Global Index";
const SERVICE_DELETE_ACCESS: u32 = 0x0001_0000;

static SERVICE_CONTEXT: OnceLock<Arc<ServiceRuntime>> = OnceLock::new();
static SERVICE_STATUS_HANDLE_VALUE: AtomicUsize = AtomicUsize::new(0);

struct ServiceRuntime {
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    busy: AtomicBool,
    state: Mutex<ServiceLifecycleState>,
    operation_lock: Mutex<()>,
    provider: Box<dyn GlobalIndexProvider>,
}

#[derive(Debug)]
struct BusyReservation<'a> {
    busy: &'a AtomicBool,
}

impl Drop for BusyReservation<'_> {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Default)]
struct ServiceLifecycleState {
    status: String,
    last_error: Option<String>,
}

impl ServiceRuntime {
    fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            busy: AtomicBool::new(false),
            state: Mutex::new(ServiceLifecycleState {
                status: "starting".to_string(),
                last_error: None,
            }),
            operation_lock: Mutex::new(()),
            provider: Box::new(DirectWindowsGlobalIndexProvider::new()),
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
        connection: &mut dyn IndexServiceConnection,
        error_code: &str,
        message: impl Into<String>,
        status: &str,
    ) -> Result<(), String> {
        connection.send_response(self.response(
            request,
            false,
            Some(error_code),
            Some(message.into()),
            Some(status.to_string()),
        ))
    }

    fn handle_request(
        &self,
        request: IndexServiceRequest,
        connection: &mut dyn IndexServiceConnection,
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
                    let (code, status) = super::service_errors::classify(&error);
                    self.send_error(&request, connection, &code, error.to_string(), status)
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
        connection: &mut dyn IndexServiceConnection,
    ) -> Result<(), String> {
        let _reservation = match self.try_begin_index_request(&request) {
            Ok(reservation) => reservation,
            Err(response) => return connection.send_response(response),
        };
        let operation_result: Result<(), (String, String, &'static str)> = (|| {
            let _operation_guard = self.operation_lock.lock().map_err(|_| {
                (
                    "index_failed".to_string(),
                    "index_service_operation_lock_poisoned".to_string(),
                    crate::global_index::models::INDEX_STATUS_ERROR,
                )
            })?;
            let source_id = source_id(&command);
            let source = self
                .validated_source(source_id, request.source.as_ref())
                .map_err(|message| {
                    (
                        "index_provider_error".to_string(),
                        message,
                        crate::global_index::models::INDEX_STATUS_ERROR,
                    )
                })?;
            connection
                .send_event(IndexServiceEvent::Sources {
                    sources: vec![source.volume.clone()],
                })
                .map_err(|message| {
                    (
                        "index_provider_error".to_string(),
                        message,
                        crate::global_index::models::INDEX_STATUS_ERROR,
                    )
                })?;
            self.cancel.store(false, Ordering::Release);
            self.set_state("indexing", None);
            let mut sink = PipeSink {
                connection,
                cancel: self.cancel.clone(),
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
                    Err((code, error.to_string(), status))
                }
            }
        })();
        match operation_result {
            Ok(()) => connection.send_response(self.response(
                &request,
                true,
                None,
                None,
                Some(self.status()),
            )),
            Err((code, message, status)) => {
                self.send_error(&request, connection, &code, message, status)
            }
        }
    }

    fn try_begin_index_request(
        &self,
        request: &IndexServiceRequest,
    ) -> Result<BusyReservation<'_>, IndexServiceResponse> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| BusyReservation { busy: &self.busy })
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
        if snapshot.platform == "windows"
            && (snapshot.scope_authorization_state != SCOPE_AUTHORIZATION_AUTHORIZED
                || !snapshot.enabled)
        {
            return Err("index_service_source_authorization_required".to_string());
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
        if current.volume.stable_volume_id != snapshot.stable_volume_id
            || !provider_snapshot_is_compatible(&current.volume.provider, &snapshot.provider)
            || !current
                .volume
                .mount_path
                .eq_ignore_ascii_case(&snapshot.mount_path)
            || !current
                .volume
                .filesystem_type
                .eq_ignore_ascii_case(&snapshot.filesystem_type)
        {
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
    current.scope_authorization_state = snapshot.scope_authorization_state.clone();
    current.index_status = snapshot.index_status.clone();
    current.journal_id = snapshot.journal_id.clone();
    current.journal_cursor = snapshot.journal_cursor.clone();
    current.last_full_index_at = snapshot.last_full_index_at;
    current.last_incremental_sync_at = snapshot.last_incremental_sync_at;
    // Provider capability stays from fresh native discovery. A persisted
    // recursive fallback is not consent and cannot override NTFS MFT/USN.
    current
}

fn provider_snapshot_is_compatible(current: &str, snapshot: &str) -> bool {
    current == snapshot
        || (current == PROVIDER_WINDOWS_MFT_USN && snapshot == PROVIDER_WINDOWS_RECURSIVE_FALLBACK)
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
    connection: &'a mut dyn IndexServiceConnection,
    cancel: Arc<AtomicBool>,
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
        eprintln!("Fylura index service dispatcher failed: {error}");
        return error as i32;
    }
    runtime.set_state("running", None);
    let result = super::service::serve_index_service_loop(
        runtime.stop.clone(),
        service_handler(runtime.clone()),
    );
    if let Err(error) = result {
        eprintln!("Fylura index service console host failed: {error}");
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
#[path = "service_host_p1_tests.rs"]
mod p1_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::models::INDEX_STATUS_DISCOVERED;

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
        assert_eq!(INDEX_SERVICE_NAME, "FyluraGlobalIndex");
        assert!(format!(
            r#""{}" --index-service"#,
            r"C:\Program Files\Fylura\Fylura.exe"
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
    fn service_host_rejects_unconfirmed_source_before_discovery() {
        let runtime = ServiceRuntime::new();
        let mut legacy = GlobalVolume {
            id: "v".into(),
            platform: "windows".into(),
            stable_volume_id: "native-v".into(),
            display_name: "test".into(),
            mount_path: "Z:\\".into(),
            filesystem_type: "NTFS".into(),
            drive_kind: "fixed".into(),
            enabled: true,
            scope_authorization_state: SCOPE_AUTHORIZATION_AUTHORIZED.into(),
            provider: PROVIDER_WINDOWS_MFT_USN.into(),
            index_status: INDEX_STATUS_READY.into(),
            last_error: None,
            journal_id: None,
            journal_cursor: None,
            last_full_index_at: None,
            last_incremental_sync_at: None,
            entry_count: 0,
            created_at: 0,
            updated_at: 0,
        };
        legacy.scope_authorization_state =
            crate::global_index::models::SCOPE_AUTHORIZATION_LEGACY_CONFIRMATION_REQUIRED
                .to_string();
        let error = runtime
            .validated_source(&legacy.id, Some(&legacy))
            .expect_err("service entry must fail closed on unknown historical consent");
        assert_eq!(error, "index_service_source_authorization_required");
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
        let _active_reservation = runtime
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
        assert!(provider_snapshot_is_compatible(
            PROVIDER_WINDOWS_MFT_USN,
            PROVIDER_WINDOWS_RECURSIVE_FALLBACK
        ));
        assert!(!provider_snapshot_is_compatible(
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
            scope_authorization_state: "authorized".to_string(),
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
}
