#![allow(unsafe_op_in_unsafe_fn)]

//! Test-only T10-B1 SQLite VFS wrapper and B1-F0 feasibility gate.

use crate::db::{PRODUCT_BUSY_TIMEOUT_MS, PRODUCT_POOL_SIZE, configure_product_main};
use rusqlite::ffi as sqlite;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::mem::{self, align_of, size_of};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const F0_SEED: &str = "issue381-b1-f0-seed-20261014";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FileKind {
    MainDb,
    Wal,
    MainJournal,
    Temp,
    Other,
}

impl FileKind {
    fn from_flags(flags: c_int) -> Self {
        if flags & sqlite::SQLITE_OPEN_MAIN_DB != 0 {
            Self::MainDb
        } else if flags & sqlite::SQLITE_OPEN_WAL != 0 {
            Self::Wal
        } else if flags & sqlite::SQLITE_OPEN_MAIN_JOURNAL != 0 {
            Self::MainJournal
        } else if flags
            & (sqlite::SQLITE_OPEN_TEMP_DB
                | sqlite::SQLITE_OPEN_TEMP_JOURNAL
                | sqlite::SQLITE_OPEN_SUBJOURNAL
                | sqlite::SQLITE_OPEN_TRANSIENT_DB)
            != 0
        {
            Self::Temp
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IoOp {
    Read,
    Write,
    Sync,
    Truncate,
}

impl IoOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "xRead",
            Self::Write => "xWrite",
            Self::Sync => "xSync",
            Self::Truncate => "xTruncate",
        }
    }
}

#[derive(Clone, Debug)]
struct FaultPlan {
    id: String,
    operation: IoOp,
    target_kind: FileKind,
    target_basename: String,
    occurrence: usize,
    return_code: c_int,
    prefix_bytes: Option<c_int>,
    seen: usize,
    fired: bool,
}

#[derive(Clone, Debug, Serialize)]
struct IoEvent {
    sequence: u64,
    operation: String,
    path: String,
    file_kind: FileKind,
    open_flags: c_int,
    offset: Option<i64>,
    requested_bytes: Option<c_int>,
    prefix_bytes: Option<c_int>,
    zero_fill_bytes: Option<c_int>,
    zero_fill_verified: Option<bool>,
    injected: bool,
    injection_id: Option<String>,
    result_code: c_int,
}

#[derive(Default)]
struct Recorder {
    next_sequence: u64,
    events: Vec<IoEvent>,
    fault: Option<FaultPlan>,
}

struct FaultContext {
    inner_offset: usize,
    open_handles: AtomicUsize,
    recorder: Mutex<Recorder>,
}

#[repr(C)]
struct WrappedFile {
    base: sqlite::sqlite3_file,
    context: *mut FaultContext,
    info: *mut FileInfo,
}

struct FileInfo {
    path: String,
    flags: c_int,
    kind: FileKind,
}

#[repr(C)]
struct VfsContainer {
    // Must remain first: SQLite passes &raw as sqlite3_vfs* and callbacks cast
    // it back to this container to access the original VFS and fault context.
    raw: sqlite::sqlite3_vfs,
    parent: *mut sqlite::sqlite3_vfs,
    context: *mut FaultContext,
    name: CString,
}

struct TestVfs {
    container: Box<VfsContainer>,
    context: Box<FaultContext>,
    registered: bool,
}

impl TestVfs {
    fn register() -> Result<Self, String> {
        let init_rc = unsafe { sqlite::sqlite3_initialize() };
        if init_rc != sqlite::SQLITE_OK {
            return Err(format!("sqlite3_initialize returned {init_rc}"));
        }
        let parent = unsafe { sqlite::sqlite3_vfs_find(ptr::null()) };
        if parent.is_null() {
            return Err("SQLite default VFS lookup returned null".to_string());
        }
        unsafe { validate_parent_vfs(parent)? };

        let alignment = align_of::<u64>();
        let inner_offset = size_of::<WrappedFile>().div_ceil(alignment) * alignment;
        let file_bytes = inner_offset
            .checked_add(unsafe { (*parent).szOsFile as usize })
            .ok_or_else(|| "VFS sqlite3_file size overflow".to_string())?;
        let file_bytes = c_int::try_from(file_bytes)
            .map_err(|_| "VFS sqlite3_file size exceeds c_int".to_string())?;
        if unsafe { (*parent).szOsFile <= 0 } {
            return Err("default VFS reports invalid szOsFile".to_string());
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("system clock error: {e}"))?
            .as_nanos();
        let name = CString::new(format!("issue381_b1_vfs_{}_{}", std::process::id(), nonce))
            .map_err(|e| format!("VFS name encoding failed: {e}"))?;
        if !unsafe { sqlite::sqlite3_vfs_find(name.as_ptr()) }.is_null() {
            return Err("unique VFS name unexpectedly already registered".to_string());
        }

        let mut context = Box::new(FaultContext {
            inner_offset,
            open_handles: AtomicUsize::new(0),
            recorder: Mutex::new(Recorder::default()),
        });
        let context_ptr = (&mut *context) as *mut FaultContext;
        let mut raw = unsafe { ptr::read(parent) };
        raw.szOsFile = file_bytes;
        raw.pNext = ptr::null_mut();
        raw.zName = name.as_ptr();
        raw.xOpen = Some(io_vfs_x_open);
        let mut container = Box::new(VfsContainer {
            raw,
            parent,
            context: context_ptr,
            name,
        });
        install_vfs_forwarders(&mut container);
        let register_rc = unsafe { sqlite::sqlite3_vfs_register(&mut container.raw, 0) };
        if register_rc != sqlite::SQLITE_OK {
            return Err(format!("sqlite3_vfs_register returned {register_rc}"));
        }
        Ok(Self {
            container,
            context,
            registered: true,
        })
    }

    fn name(&self) -> &str {
        self.container
            .name
            .to_str()
            .expect("VFS name is valid UTF-8")
    }

    fn arm(&self, plan: FaultPlan) -> Result<(), String> {
        let mut recorder = self
            .context
            .recorder
            .lock()
            .map_err(|_| "VFS recorder mutex poisoned".to_string())?;
        if recorder.fault.as_ref().is_some_and(|f| !f.fired) {
            return Err("refusing to replace an unfired VFS fault plan".to_string());
        }
        recorder.fault = Some(plan);
        Ok(())
    }

    fn events(&self) -> Result<Vec<IoEvent>, String> {
        self.context
            .recorder
            .lock()
            .map(|r| r.events.clone())
            .map_err(|_| "VFS recorder mutex poisoned".to_string())
    }

    fn fault_plan(&self) -> Result<Option<FaultPlan>, String> {
        self.context
            .recorder
            .lock()
            .map(|r| r.fault.clone())
            .map_err(|_| "VFS recorder mutex poisoned".to_string())
    }

    fn open_handle_count(&self) -> usize {
        self.context.open_handles.load(Ordering::SeqCst)
    }

    fn unregister(&mut self) -> Result<(), String> {
        let open_count = self.open_handle_count();
        if open_count != 0 {
            return Err(format!(
                "refusing to unregister named VFS with {open_count} live SQLite file handles"
            ));
        }
        if self.registered {
            let rc = unsafe { sqlite::sqlite3_vfs_unregister(&mut self.container.raw) };
            if rc != sqlite::SQLITE_OK {
                return Err(format!("sqlite3_vfs_unregister returned {rc}"));
            }
            self.registered = false;
        }
        Ok(())
    }
}

impl Drop for TestVfs {
    fn drop(&mut self) {
        if self.registered {
            unsafe {
                let _ = sqlite::sqlite3_vfs_unregister(&mut self.container.raw);
            }
            self.registered = false;
        }
        if self.open_handle_count() != 0 {
            // Leaking callback state is safer than freeing memory reachable
            // through a surviving SQLite file handle.
            let placeholder_name = CString::new("issue381-b1-leaked-vfs-state").unwrap();
            let placeholder_context = Box::new(FaultContext {
                inner_offset: 0,
                open_handles: AtomicUsize::new(0),
                recorder: Mutex::new(Recorder::default()),
            });
            let placeholder_container = Box::new(VfsContainer {
                raw: unsafe { mem::zeroed() },
                parent: ptr::null_mut(),
                context: ptr::null_mut(),
                name: placeholder_name,
            });
            let container = mem::replace(&mut self.container, placeholder_container);
            let context = mem::replace(&mut self.context, placeholder_context);
            mem::forget(container);
            mem::forget(context);
        }
    }
}

#[derive(Clone, Debug)]
struct StartedEvent {
    sequence: u64,
    inject: bool,
    injection_id: Option<String>,
    return_code: c_int,
    prefix_bytes: Option<c_int>,
}

impl FaultContext {
    fn start_io(&self, operation: IoOp, info: &FileInfo) -> StartedEvent {
        let mut recorder = self.recorder.lock().unwrap_or_else(|p| p.into_inner());
        recorder.next_sequence += 1;
        let sequence = recorder.next_sequence;
        let mut started = StartedEvent {
            sequence,
            inject: false,
            injection_id: None,
            return_code: sqlite::SQLITE_OK,
            prefix_bytes: None,
        };
        if let Some(plan) = recorder.fault.as_mut() {
            let basename = Path::new(&info.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if !plan.fired
                && plan.operation == operation
                && plan.target_kind == info.kind
                && plan.target_basename == basename
            {
                plan.seen += 1;
                if plan.seen == plan.occurrence {
                    plan.fired = true;
                    started.inject = true;
                    started.injection_id = Some(plan.id.clone());
                    started.return_code = plan.return_code;
                    started.prefix_bytes = plan.prefix_bytes;
                }
            }
        }
        started
    }

    fn start_non_io(&self) -> u64 {
        let mut recorder = self.recorder.lock().unwrap_or_else(|p| p.into_inner());
        recorder.next_sequence += 1;
        recorder.next_sequence
    }

    fn record(&self, event: IoEvent) {
        self.recorder
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .events
            .push(event);
    }
}

unsafe fn container(vfs: *mut sqlite::sqlite3_vfs) -> &'static VfsContainer {
    &*(vfs.cast::<VfsContainer>())
}

unsafe fn inner_file(file: *mut sqlite::sqlite3_file) -> *mut sqlite::sqlite3_file {
    let wrapper = &*(file.cast::<WrappedFile>());
    file.cast::<u8>()
        .add((*wrapper.context).inner_offset)
        .cast()
}

unsafe fn file_info(file: *mut sqlite::sqlite3_file) -> &'static FileInfo {
    let wrapper = &*(file.cast::<WrappedFile>());
    &*wrapper.info
}

unsafe fn context_for_file(file: *mut sqlite::sqlite3_file) -> &'static FaultContext {
    &*(*file.cast::<WrappedFile>()).context
}

unsafe fn parent_methods(
    file: *mut sqlite::sqlite3_file,
) -> Option<&'static sqlite::sqlite3_io_methods> {
    let methods = (*inner_file(file)).pMethods;
    if methods.is_null() {
        None
    } else {
        Some(&*methods)
    }
}

unsafe fn validate_parent_vfs(parent: *mut sqlite::sqlite3_vfs) -> Result<(), String> {
    let vfs = &*parent;
    if vfs.iVersion < 3 {
        return Err(format!(
            "default VFS version {} is below required version 3",
            vfs.iVersion
        ));
    }
    if vfs.szOsFile <= 0 || vfs.mxPathname <= 0 {
        return Err(format!(
            "default VFS reports invalid sizes: szOsFile={}, mxPathname={}",
            vfs.szOsFile, vfs.mxPathname
        ));
    }
    for (name, present) in [
        ("xOpen", vfs.xOpen.is_some()),
        ("xDelete", vfs.xDelete.is_some()),
        ("xAccess", vfs.xAccess.is_some()),
        ("xFullPathname", vfs.xFullPathname.is_some()),
        ("xRandomness", vfs.xRandomness.is_some()),
        ("xSleep", vfs.xSleep.is_some()),
        ("xCurrentTime", vfs.xCurrentTime.is_some()),
        ("xCurrentTimeInt64", vfs.xCurrentTimeInt64.is_some()),
    ] {
        if !present {
            return Err(format!("default VFS is missing required {name}"));
        }
    }
    Ok(())
}

unsafe fn parent_io_methods_complete(
    methods: *const sqlite::sqlite3_io_methods,
    kind: FileKind,
) -> bool {
    if methods.is_null() {
        return false;
    }
    let methods = &*methods;
    let core_methods = methods.iVersion >= 1
        && methods.xClose.is_some()
        && methods.xRead.is_some()
        && methods.xWrite.is_some()
        && methods.xTruncate.is_some()
        && methods.xSync.is_some()
        && methods.xFileSize.is_some()
        && methods.xLock.is_some()
        && methods.xUnlock.is_some()
        && methods.xCheckReservedLock.is_some()
        && methods.xFileControl.is_some()
        && methods.xSectorSize.is_some()
        && methods.xDeviceCharacteristics.is_some();
    let main_database_shm = kind != FileKind::MainDb
        || (methods.iVersion >= 2
            && methods.xShmMap.is_some()
            && methods.xShmLock.is_some()
            && methods.xShmBarrier.is_some()
            && methods.xShmUnmap.is_some());
    let mmap_pair_valid =
        methods.iVersion < 3 || methods.xFetch.is_some() == methods.xUnfetch.is_some();
    core_methods && main_database_shm && mmap_pair_valid
}

unsafe fn log_open(context: &FaultContext, path: String, flags: c_int, result_code: c_int) {
    let sequence = context.start_non_io();
    context.record(IoEvent {
        sequence,
        operation: "xOpen".to_string(),
        path,
        file_kind: FileKind::from_flags(flags),
        open_flags: flags,
        offset: None,
        requested_bytes: None,
        prefix_bytes: None,
        zero_fill_bytes: None,
        zero_fill_verified: None,
        injected: false,
        injection_id: None,
        result_code,
    });
}

unsafe extern "C" fn io_vfs_x_open(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    file: *mut sqlite::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    vfs_x_open(vfs, filename, file, flags, out_flags)
}

unsafe extern "C" fn io_x_close(file: *mut sqlite::sqlite3_file) -> c_int {
    if file.is_null() {
        return sqlite::SQLITE_IOERR_CLOSE;
    }
    let context = context_for_file(file);
    let info = file_info(file);
    let sequence = context.start_non_io();
    let result = parent_methods(file)
        .and_then(|m| m.xClose)
        .map(|f| f(inner_file(file)))
        .unwrap_or(sqlite::SQLITE_IOERR_CLOSE);
    context.record(IoEvent {
        sequence,
        operation: "xClose".to_string(),
        path: info.path.clone(),
        file_kind: info.kind,
        open_flags: info.flags,
        offset: None,
        requested_bytes: None,
        prefix_bytes: None,
        zero_fill_bytes: None,
        zero_fill_verified: None,
        injected: false,
        injection_id: None,
        result_code: result,
    });
    drop(Box::from_raw((*file.cast::<WrappedFile>()).info));
    (*file.cast::<WrappedFile>()).info = ptr::null_mut();
    (*file).pMethods = ptr::null();
    context.open_handles.fetch_sub(1, Ordering::SeqCst);
    result
}

unsafe extern "C" fn io_x_read(
    file: *mut sqlite::sqlite3_file,
    buffer: *mut c_void,
    amount: c_int,
    offset: sqlite::sqlite3_int64,
) -> c_int {
    let info = file_info(file);
    let context = context_for_file(file);
    let started = context.start_io(IoOp::Read, info);
    let mut prefix = amount;
    let mut zero_fill = 0;
    let mut zero_fill_verified = None;
    let result = if started.inject {
        let requested_prefix = started.prefix_bytes.unwrap_or_else(|| (amount / 2).max(1));
        prefix = requested_prefix.clamp(0, amount.max(0));
        if prefix > 0 {
            let rc = parent_methods(file)
                .and_then(|m| m.xRead)
                .map(|f| f(inner_file(file), buffer, prefix, offset))
                .unwrap_or(sqlite::SQLITE_IOERR_READ);
            if rc != sqlite::SQLITE_OK && rc != sqlite::SQLITE_IOERR_SHORT_READ {
                rc
            } else {
                zero_fill = amount.saturating_sub(prefix);
                if zero_fill > 0 {
                    ptr::write_bytes(
                        buffer.cast::<u8>().add(prefix as usize),
                        0,
                        zero_fill as usize,
                    );
                }
                let tail = std::slice::from_raw_parts(
                    buffer.cast::<u8>().add(prefix as usize),
                    zero_fill as usize,
                );
                let valid = tail.iter().all(|byte| *byte == 0);
                zero_fill_verified = Some(valid);
                if valid {
                    started.return_code
                } else {
                    sqlite::SQLITE_IOERR_READ
                }
            }
        } else {
            zero_fill = amount.max(0);
            if zero_fill > 0 {
                ptr::write_bytes(buffer.cast::<u8>(), 0, zero_fill as usize);
            }
            let valid = std::slice::from_raw_parts(buffer.cast::<u8>(), zero_fill as usize)
                .iter()
                .all(|byte| *byte == 0);
            zero_fill_verified = Some(valid);
            if valid {
                started.return_code
            } else {
                sqlite::SQLITE_IOERR_READ
            }
        }
    } else {
        parent_methods(file)
            .and_then(|m| m.xRead)
            .map(|f| f(inner_file(file), buffer, amount, offset))
            .unwrap_or(sqlite::SQLITE_IOERR_READ)
    };
    context.record(IoEvent {
        sequence: started.sequence,
        operation: IoOp::Read.as_str().to_string(),
        path: info.path.clone(),
        file_kind: info.kind,
        open_flags: info.flags,
        offset: Some(offset),
        requested_bytes: Some(amount),
        prefix_bytes: started.inject.then_some(prefix),
        zero_fill_bytes: started.inject.then_some(zero_fill),
        zero_fill_verified,
        injected: started.inject,
        injection_id: started.injection_id,
        result_code: result,
    });
    result
}

unsafe extern "C" fn io_x_write(
    file: *mut sqlite::sqlite3_file,
    buffer: *const c_void,
    amount: c_int,
    offset: sqlite::sqlite3_int64,
) -> c_int {
    let info = file_info(file);
    let context = context_for_file(file);
    let started = context.start_io(IoOp::Write, info);
    let mut prefix_written = 0;
    let result = if started.inject {
        if let Some(requested_prefix) = started.prefix_bytes {
            let prefix = requested_prefix.clamp(0, amount.max(0));
            if prefix > 0 {
                let rc = parent_methods(file)
                    .and_then(|m| m.xWrite)
                    .map(|f| f(inner_file(file), buffer, prefix, offset))
                    .unwrap_or(sqlite::SQLITE_IOERR_WRITE);
                if rc != sqlite::SQLITE_OK {
                    rc
                } else {
                    prefix_written = prefix;
                    started.return_code
                }
            } else {
                started.return_code
            }
        } else {
            started.return_code
        }
    } else {
        let rc = parent_methods(file)
            .and_then(|m| m.xWrite)
            .map(|f| f(inner_file(file), buffer, amount, offset))
            .unwrap_or(sqlite::SQLITE_IOERR_WRITE);
        if rc == sqlite::SQLITE_OK {
            prefix_written = amount;
        }
        rc
    };
    context.record(IoEvent {
        sequence: started.sequence,
        operation: IoOp::Write.as_str().to_string(),
        path: info.path.clone(),
        file_kind: info.kind,
        open_flags: info.flags,
        offset: Some(offset),
        requested_bytes: Some(amount),
        prefix_bytes: Some(prefix_written),
        zero_fill_bytes: None,
        zero_fill_verified: None,
        injected: started.inject,
        injection_id: started.injection_id,
        result_code: result,
    });
    result
}

unsafe extern "C" fn io_x_truncate(
    file: *mut sqlite::sqlite3_file,
    size: sqlite::sqlite3_int64,
) -> c_int {
    let info = file_info(file);
    let context = context_for_file(file);
    let started = context.start_io(IoOp::Truncate, info);
    let result = if started.inject {
        started.return_code
    } else {
        parent_methods(file)
            .and_then(|m| m.xTruncate)
            .map(|f| f(inner_file(file), size))
            .unwrap_or(sqlite::SQLITE_IOERR_TRUNCATE)
    };
    context.record(IoEvent {
        sequence: started.sequence,
        operation: IoOp::Truncate.as_str().to_string(),
        path: info.path.clone(),
        file_kind: info.kind,
        open_flags: info.flags,
        offset: None,
        requested_bytes: i32::try_from(size).ok(),
        prefix_bytes: None,
        zero_fill_bytes: None,
        zero_fill_verified: None,
        injected: started.inject,
        injection_id: started.injection_id,
        result_code: result,
    });
    result
}

unsafe extern "C" fn io_x_sync(file: *mut sqlite::sqlite3_file, flags: c_int) -> c_int {
    let info = file_info(file);
    let context = context_for_file(file);
    let started = context.start_io(IoOp::Sync, info);
    let result = if started.inject {
        started.return_code
    } else {
        parent_methods(file)
            .and_then(|m| m.xSync)
            .map(|f| f(inner_file(file), flags))
            .unwrap_or(sqlite::SQLITE_IOERR_FSYNC)
    };
    context.record(IoEvent {
        sequence: started.sequence,
        operation: IoOp::Sync.as_str().to_string(),
        path: info.path.clone(),
        file_kind: info.kind,
        open_flags: info.flags,
        offset: None,
        requested_bytes: Some(flags),
        prefix_bytes: None,
        zero_fill_bytes: None,
        zero_fill_verified: None,
        injected: started.inject,
        injection_id: started.injection_id,
        result_code: result,
    });
    result
}

unsafe extern "C" fn io_x_file_size(
    file: *mut sqlite::sqlite3_file,
    size: *mut sqlite::sqlite3_int64,
) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xFileSize)
        .map(|f| f(inner_file(file), size))
        .unwrap_or(sqlite::SQLITE_IOERR_FSTAT)
}

unsafe extern "C" fn io_x_lock(file: *mut sqlite::sqlite3_file, level: c_int) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xLock)
        .map(|f| f(inner_file(file), level))
        .unwrap_or(sqlite::SQLITE_IOERR_LOCK)
}

unsafe extern "C" fn io_x_unlock(file: *mut sqlite::sqlite3_file, level: c_int) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xUnlock)
        .map(|f| f(inner_file(file), level))
        .unwrap_or(sqlite::SQLITE_IOERR_UNLOCK)
}

unsafe extern "C" fn io_x_check_reserved_lock(
    file: *mut sqlite::sqlite3_file,
    result: *mut c_int,
) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xCheckReservedLock)
        .map(|f| f(inner_file(file), result))
        .unwrap_or(sqlite::SQLITE_IOERR_CHECKRESERVEDLOCK)
}

unsafe extern "C" fn io_x_file_control(
    file: *mut sqlite::sqlite3_file,
    operation: c_int,
    argument: *mut c_void,
) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xFileControl)
        .map(|f| f(inner_file(file), operation, argument))
        .unwrap_or(sqlite::SQLITE_NOTFOUND)
}

unsafe extern "C" fn io_x_sector_size(file: *mut sqlite::sqlite3_file) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xSectorSize)
        .map(|f| f(inner_file(file)))
        .unwrap_or(512)
}

unsafe extern "C" fn io_x_device_characteristics(file: *mut sqlite::sqlite3_file) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xDeviceCharacteristics)
        .map(|f| f(inner_file(file)))
        .unwrap_or(0)
}

unsafe extern "C" fn io_x_shm_map(
    file: *mut sqlite::sqlite3_file,
    page: c_int,
    page_size: c_int,
    extend: c_int,
    result: *mut *mut c_void,
) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xShmMap)
        .map(|f| f(inner_file(file), page, page_size, extend, result))
        .unwrap_or(sqlite::SQLITE_IOERR_SHMMAP)
}

unsafe extern "C" fn io_x_shm_lock(
    file: *mut sqlite::sqlite3_file,
    offset: c_int,
    count: c_int,
    flags: c_int,
) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xShmLock)
        .map(|f| f(inner_file(file), offset, count, flags))
        .unwrap_or(sqlite::SQLITE_IOERR_SHMLOCK)
}

unsafe extern "C" fn io_x_shm_barrier(file: *mut sqlite::sqlite3_file) {
    if let Some(f) = parent_methods(file).and_then(|m| m.xShmBarrier) {
        f(inner_file(file));
    }
}

unsafe extern "C" fn io_x_shm_unmap(file: *mut sqlite::sqlite3_file, delete_flag: c_int) -> c_int {
    parent_methods(file)
        .and_then(|m| m.xShmUnmap)
        .map(|f| f(inner_file(file), delete_flag))
        .unwrap_or(sqlite::SQLITE_IOERR_SHMOPEN)
}

unsafe extern "C" fn io_x_fetch(
    file: *mut sqlite::sqlite3_file,
    offset: sqlite::sqlite3_int64,
    amount: c_int,
    result: *mut *mut c_void,
) -> c_int {
    if let Some(f) = parent_methods(file).and_then(|m| m.xFetch) {
        f(inner_file(file), offset, amount, result)
    } else {
        *result = ptr::null_mut();
        sqlite::SQLITE_OK
    }
}

unsafe extern "C" fn io_x_unfetch(
    file: *mut sqlite::sqlite3_file,
    offset: sqlite::sqlite3_int64,
    page: *mut c_void,
) -> c_int {
    if let Some(f) = parent_methods(file).and_then(|m| m.xUnfetch) {
        f(inner_file(file), offset, page)
    } else {
        sqlite::SQLITE_OK
    }
}

static WRAPPER_IO_METHODS: sqlite::sqlite3_io_methods = sqlite::sqlite3_io_methods {
    iVersion: 3,
    xClose: Some(io_x_close),
    xRead: Some(io_x_read),
    xWrite: Some(io_x_write),
    xTruncate: Some(io_x_truncate),
    xSync: Some(io_x_sync),
    xFileSize: Some(io_x_file_size),
    xLock: Some(io_x_lock),
    xUnlock: Some(io_x_unlock),
    xCheckReservedLock: Some(io_x_check_reserved_lock),
    xFileControl: Some(io_x_file_control),
    xSectorSize: Some(io_x_sector_size),
    xDeviceCharacteristics: Some(io_x_device_characteristics),
    xShmMap: Some(io_x_shm_map),
    xShmLock: Some(io_x_shm_lock),
    xShmBarrier: Some(io_x_shm_barrier),
    xShmUnmap: Some(io_x_shm_unmap),
    xFetch: Some(io_x_fetch),
    xUnfetch: Some(io_x_unfetch),
};

unsafe fn parent_io(vfs: *mut sqlite::sqlite3_vfs) -> *mut sqlite::sqlite3_vfs {
    container(vfs).parent
}

fn vfs_x_open(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    file: *mut sqlite::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    unsafe {
        if vfs.is_null() || file.is_null() {
            return sqlite::SQLITE_CANTOPEN;
        }
        let container = container(vfs);
        let context = &*container.context;
        let path = if filename.is_null() {
            "<sqlite-temp-anonymous>".to_string()
        } else {
            CStr::from_ptr(filename).to_string_lossy().into_owned()
        };
        let info = Box::new(FileInfo {
            path: path.clone(),
            flags,
            kind: FileKind::from_flags(flags),
        });
        ptr::write(
            file.cast::<WrappedFile>(),
            WrappedFile {
                base: sqlite::sqlite3_file {
                    pMethods: ptr::null(),
                },
                context: container.context,
                info: Box::into_raw(info),
            },
        );
        let real_file = inner_file(file);
        ptr::write_bytes(
            real_file.cast::<u8>(),
            0,
            (*container.parent).szOsFile as usize,
        );
        let Some(parent_open) = (*container.parent).xOpen else {
            drop(Box::from_raw((*file.cast::<WrappedFile>()).info));
            (*file.cast::<WrappedFile>()).info = ptr::null_mut();
            log_open(context, path, flags, sqlite::SQLITE_CANTOPEN);
            return sqlite::SQLITE_CANTOPEN;
        };
        let rc = parent_open(container.parent, filename, real_file, flags, out_flags);
        if rc != sqlite::SQLITE_OK {
            if !(*real_file).pMethods.is_null() {
                if let Some(close) = (*(*real_file).pMethods).xClose {
                    let _ = close(real_file);
                }
            }
            drop(Box::from_raw((*file.cast::<WrappedFile>()).info));
            (*file.cast::<WrappedFile>()).info = ptr::null_mut();
            log_open(context, path, flags, rc);
            return rc;
        }
        if !parent_io_methods_complete((*real_file).pMethods, FileKind::from_flags(flags)) {
            if !(*real_file).pMethods.is_null() {
                if let Some(close) = (*(*real_file).pMethods).xClose {
                    let _ = close(real_file);
                }
            }
            drop(Box::from_raw((*file.cast::<WrappedFile>()).info));
            (*file.cast::<WrappedFile>()).info = ptr::null_mut();
            log_open(context, path, flags, sqlite::SQLITE_CANTOPEN);
            return sqlite::SQLITE_CANTOPEN;
        }
        (*file).pMethods = &WRAPPER_IO_METHODS;
        context.open_handles.fetch_add(1, Ordering::SeqCst);
        log_open(context, path, flags, sqlite::SQLITE_OK);
        sqlite::SQLITE_OK
    }
}

unsafe fn vfs_x_delete(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    sync_dir: c_int,
) -> c_int {
    let parent = parent_io(vfs);
    (*parent)
        .xDelete
        .map(|f| f(parent, filename, sync_dir))
        .unwrap_or(sqlite::SQLITE_IOERR_DELETE)
}

unsafe fn vfs_x_access(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    flags: c_int,
    result: *mut c_int,
) -> c_int {
    let parent = parent_io(vfs);
    (*parent)
        .xAccess
        .map(|f| f(parent, filename, flags, result))
        .unwrap_or(sqlite::SQLITE_IOERR_ACCESS)
}

unsafe fn vfs_x_full_pathname(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    output_size: c_int,
    output: *mut c_char,
) -> c_int {
    let parent = parent_io(vfs);
    (*parent)
        .xFullPathname
        .map(|f| f(parent, filename, output_size, output))
        .unwrap_or(sqlite::SQLITE_CANTOPEN)
}

unsafe fn vfs_x_randomness(
    vfs: *mut sqlite::sqlite3_vfs,
    bytes: c_int,
    output: *mut c_char,
) -> c_int {
    let parent = parent_io(vfs);
    (*parent)
        .xRandomness
        .map(|f| f(parent, bytes, output))
        .unwrap_or(0)
}

unsafe fn vfs_x_sleep(vfs: *mut sqlite::sqlite3_vfs, microseconds: c_int) -> c_int {
    let parent = parent_io(vfs);
    (*parent)
        .xSleep
        .map(|f| f(parent, microseconds))
        .unwrap_or(0)
}

unsafe fn vfs_x_current_time(vfs: *mut sqlite::sqlite3_vfs, result: *mut f64) -> c_int {
    let parent = parent_io(vfs);
    (*parent)
        .xCurrentTime
        .map(|f| f(parent, result))
        .unwrap_or(sqlite::SQLITE_ERROR)
}

unsafe extern "C" fn io_vfs_x_delete(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    sync_dir: c_int,
) -> c_int {
    vfs_x_delete(vfs, filename, sync_dir)
}

unsafe extern "C" fn io_vfs_x_access(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    flags: c_int,
    result: *mut c_int,
) -> c_int {
    vfs_x_access(vfs, filename, flags, result)
}

unsafe extern "C" fn io_vfs_x_full_pathname(
    vfs: *mut sqlite::sqlite3_vfs,
    filename: *const c_char,
    output_size: c_int,
    output: *mut c_char,
) -> c_int {
    vfs_x_full_pathname(vfs, filename, output_size, output)
}

unsafe extern "C" fn io_vfs_x_randomness(
    vfs: *mut sqlite::sqlite3_vfs,
    bytes: c_int,
    output: *mut c_char,
) -> c_int {
    vfs_x_randomness(vfs, bytes, output)
}

unsafe extern "C" fn io_vfs_x_sleep(vfs: *mut sqlite::sqlite3_vfs, microseconds: c_int) -> c_int {
    vfs_x_sleep(vfs, microseconds)
}

unsafe extern "C" fn io_vfs_x_current_time(
    vfs: *mut sqlite::sqlite3_vfs,
    result: *mut f64,
) -> c_int {
    vfs_x_current_time(vfs, result)
}

fn install_vfs_forwarders(container: &mut VfsContainer) {
    container.raw.xDelete = Some(io_vfs_x_delete);
    container.raw.xAccess = Some(io_vfs_x_access);
    container.raw.xFullPathname = Some(io_vfs_x_full_pathname);
    container.raw.xRandomness = Some(io_vfs_x_randomness);
    container.raw.xSleep = Some(io_vfs_x_sleep);
    container.raw.xCurrentTime = Some(io_vfs_x_current_time);
    // The less frequently used dynamic-loader and v3 system-call entries
    // remain the original default-VFS function pointers copied above.
}

struct VfsManager {
    path: PathBuf,
    vfs_name: String,
}

impl r2d2::ManageConnection for VfsManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let mut connection = Connection::open_with_flags_and_vfs(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
            self.vfs_name.as_str(),
        )?;
        configure_product_main(&mut connection)?;
        Ok(connection)
    }

    fn is_valid(&self, connection: &mut Self::Connection) -> Result<(), Self::Error> {
        connection.query_row("SELECT 1", [], |_| Ok(()))
    }

    fn has_broken(&self, connection: &mut Self::Connection) -> bool {
        !connection.is_autocommit()
    }
}

fn open_vfs_rw(path: &Path, vfs: &TestVfs) -> Result<Connection, rusqlite::Error> {
    Connection::open_with_flags_and_vfs(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        vfs.name(),
    )
}

fn configure_fixture_connection(connection: &mut Connection) -> Result<(), rusqlite::Error> {
    connection.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "temp_store", "MEMORY")?;
    connection.pragma_update(None, "mmap_size", 0_i64)?;
    Ok(())
}

fn create_seed(path: &Path, vfs: &TestVfs) -> Result<(), Box<dyn std::error::Error>> {
    let mut connection = open_vfs_rw(path, vfs)?;
    configure_fixture_connection(&mut connection)?;
    connection.execute_batch(
        r#"
        CREATE TABLE docs(
          id INTEGER PRIMARY KEY,
          uid TEXT NOT NULL UNIQUE,
          name TEXT NOT NULL,
          path TEXT NOT NULL,
          payload TEXT NOT NULL
        );
        CREATE VIRTUAL TABLE docs_fts USING fts5(
          name, path, content='docs', content_rowid='id', tokenize='trigram'
        );
        CREATE TRIGGER docs_ai AFTER INSERT ON docs BEGIN
          INSERT INTO docs_fts(rowid,name,path) VALUES(new.id,new.name,new.path);
        END;
        "#,
    )?;
    let transaction = connection.transaction()?;
    {
        let mut insert =
            transaction.prepare("INSERT INTO docs(uid,name,path,payload) VALUES(?1,?2,?3,?4)")?;
        for index in 0..16 {
            let uid = format!("{F0_SEED}-{index:02}");
            let name = format!("baseline-reference document-{index:02} signal");
            let path = format!("/synthetic/f0/{index:02}.txt");
            let payload = format!("record-{index:02}-{}", "p".repeat(6_000));
            insert.execute(params![uid, name, path, payload])?;
        }
    }
    transaction.commit()?;
    connection.execute(
        "INSERT INTO docs_fts(docs_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    let checkpoint: (i64, i64, i64) =
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if checkpoint.0 != 0 || checkpoint.1 != checkpoint.2 {
        return Err(format!("seed WAL checkpoint incomplete: {checkpoint:?}").into());
    }
    drop(connection);
    Ok(())
}

fn sqlite_codes(error: &rusqlite::Error, connection: &Connection) -> (Option<i32>, Option<i32>) {
    if error.sqlite_error().is_none() {
        return (None, None);
    }
    unsafe {
        let db = connection.handle();
        (
            Some(sqlite::sqlite3_errcode(db)),
            Some(sqlite::sqlite3_extended_errcode(db)),
        )
    }
}

fn sqlite_code_name(code: Option<i32>) -> Option<&'static str> {
    match code? {
        sqlite::SQLITE_OK => Some("SQLITE_OK"),
        sqlite::SQLITE_ERROR => Some("SQLITE_ERROR"),
        sqlite::SQLITE_IOERR => Some("SQLITE_IOERR"),
        sqlite::SQLITE_CORRUPT => Some("SQLITE_CORRUPT"),
        sqlite::SQLITE_FULL => Some("SQLITE_FULL"),
        sqlite::SQLITE_IOERR_SHORT_READ => Some("SQLITE_IOERR_SHORT_READ"),
        sqlite::SQLITE_IOERR_WRITE => Some("SQLITE_IOERR_WRITE"),
        sqlite::SQLITE_IOERR_FSYNC => Some("SQLITE_IOERR_FSYNC"),
        sqlite::SQLITE_IOERR_TRUNCATE => Some("SQLITE_IOERR_TRUNCATE"),
        _ => None,
    }
}

fn verify_db(path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    connection.execute(
        "INSERT INTO docs_fts(docs_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    let rows: Vec<(String, String, String)> = {
        let mut statement = connection.prepare("SELECT uid,name,path FROM docs ORDER BY uid")?;
        let mapped = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        mapped.collect::<Result<_, _>>()?
    };
    let payload_bytes: i64 = connection.query_row(
        "SELECT COALESCE(SUM(length(payload)),0) FROM docs",
        [],
        |r| r.get(0),
    )?;
    let fault_rows: i64 = connection.query_row(
        "SELECT count(*) FROM docs WHERE uid='issue381-b1-f0-injected-row'",
        [],
        |r| r.get(0),
    )?;
    let wal_mode: String = connection.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    Ok(json!({
        "integrity_check":integrity,
        "fts5_integrity_check":"PASS",
        "row_count":rows.len(),
        "rows":rows,
        "payload_bytes":payload_bytes,
        "injected_row_count":fault_rows,
        "journal_mode":wal_mode
    }))
}

fn sha256_file(path: &Path) -> Value {
    match std::fs::read(path) {
        Ok(bytes) => {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&bytes);
            json!({
                "path":path.file_name().and_then(|n| n.to_str()).unwrap_or("<unknown>"),
                "bytes":bytes.len(),
                "sha256":format!("{digest:x}")
            })
        }
        Err(error) => json!({
            "path":path.file_name().and_then(|n| n.to_str()).unwrap_or("<unknown>"),
            "unavailable":error.to_string()
        }),
    }
}

fn file_hash_snapshot(path: &Path) -> Value {
    json!({
        "main":sha256_file(path),
        "wal":sha256_file(&PathBuf::from(format!("{}-wal", path.display()))),
        "shm":sha256_file(&PathBuf::from(format!("{}-shm", path.display()))),
        "journal":sha256_file(&PathBuf::from(format!("{}-journal", path.display())))
    })
}

fn basename(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("path has no UTF-8 filename: {}", path.display()))
}

fn fault_plan(
    id: &str,
    operation: IoOp,
    target_kind: FileKind,
    path: &Path,
    return_code: c_int,
    prefix_bytes: Option<c_int>,
) -> Result<FaultPlan, String> {
    Ok(FaultPlan {
        id: id.to_string(),
        operation,
        target_kind,
        target_basename: basename(path)?,
        occurrence: 1,
        return_code,
        prefix_bytes,
        seen: 0,
        fired: false,
    })
}

fn emit(record_type: &str, case_id: &str, status: &str, details: Value) {
    println!(
        "{}",
        json!({
            "record_type":record_type,
            "case_id":case_id,
            "status":status,
            "details":details
        })
    );
}

fn emit_events(case_id: &str, events: Vec<IoEvent>) {
    for event in events {
        emit("sqlite_vfs_event", case_id, "OBSERVED", json!(event));
    }
}

fn error_details(result: Result<(), rusqlite::Error>, connection: &Connection) -> Value {
    match result {
        Ok(()) => json!({"ok":true,"primary_code":null,"extended_code":null,"error":null}),
        Err(error) => {
            let (primary, extended) = sqlite_codes(&error, connection);
            json!({
                "ok":false,
                "primary_code":primary,
                "primary_code_name":sqlite_code_name(primary),
                "extended_code":extended,
                "extended_code_name":sqlite_code_name(extended),
                "error":error.to_string(),
                "debug":format!("{error:?}")
            })
        }
    }
}

fn run_pool_smoke(vfs: &TestVfs, path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let manager = VfsManager {
        path: path.to_path_buf(),
        vfs_name: vfs.name().to_string(),
    };
    let pool = r2d2::Pool::builder()
        .max_size(PRODUCT_POOL_SIZE)
        .connection_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))
        .build(manager)?;
    let mut connections = Vec::with_capacity(PRODUCT_POOL_SIZE as usize);
    for _ in 0..PRODUCT_POOL_SIZE {
        connections.push(pool.get()?);
    }
    let settings = connections
        .iter()
        .map(|connection| {
            let busy_timeout: i64 =
                connection.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
            let journal_mode: String =
                connection.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
            Ok::<_, rusqlite::Error>(json!({
                "busy_timeout_ms":busy_timeout,
                "journal_mode":journal_mode,
                "synchronous":synchronous
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let state = pool.state();
    let settings_ok = settings.iter().all(|s| {
        s["busy_timeout_ms"] == PRODUCT_BUSY_TIMEOUT_MS
            && s["journal_mode"] == "wal"
            && s["synchronous"] == 1
    });
    let ok = pool.max_size() == PRODUCT_POOL_SIZE
        && state.connections == PRODUCT_POOL_SIZE
        && settings.len() == PRODUCT_POOL_SIZE as usize
        && settings_ok;
    let details = json!({
        "named_vfs_used_by_every_connection":true,
        "pool_max_size":pool.max_size(),
        "expected_pool_max_size":PRODUCT_POOL_SIZE,
        "live_connections":state.connections,
        "expected_live_connections":PRODUCT_POOL_SIZE,
        "product_busy_timeout_ms":PRODUCT_BUSY_TIMEOUT_MS,
        "connection_settings":settings,
        "all_eight_connections_match":settings_ok,
        "ok":ok
    });
    emit(
        "case_result",
        "B1-F0-POOL-CONTRACT",
        if ok { "PASS" } else { "FAIL" },
        details.clone(),
    );
    drop(connections);
    drop(pool);
    if !ok {
        return Err(format!("8-connection/5000ms contract failed: {details}").into());
    }
    Ok(details)
}

fn vfs_summary(vfs: &TestVfs) -> Value {
    let (version, os_file_bytes, max_pathname) = unsafe {
        (
            (*vfs.container.parent).iVersion,
            (*vfs.container.parent).szOsFile,
            (*vfs.container.parent).mxPathname,
        )
    };
    json!({
        "vfs_name":vfs.name(),
        "parent_name":unsafe { CStr::from_ptr((*vfs.container.parent).zName).to_string_lossy() },
        "parent_version":version,
        "parent_szOsFile":os_file_bytes,
        "parent_mxPathname":max_pathname,
        "wrapper_inner_file_offset":vfs.context.inner_offset,
        "wrapper_os_file_size":vfs.container.raw.szOsFile,
        "sqlite_version":unsafe { CStr::from_ptr(sqlite::sqlite3_libversion()).to_string_lossy() },
        "sqlite_source_id":unsafe { CStr::from_ptr(sqlite::sqlite3_sourceid()).to_string_lossy() },
        "sqlite_error_codes":{
            "SQLITE_FULL":sqlite::SQLITE_FULL,
            "SQLITE_IOERR_WRITE":sqlite::SQLITE_IOERR_WRITE,
            "SQLITE_IOERR_FSYNC":sqlite::SQLITE_IOERR_FSYNC,
            "SQLITE_IOERR_SHORT_READ":sqlite::SQLITE_IOERR_SHORT_READ
        },
        "mmap_policy":"xFetch/xUnfetch are delegated; PRAGMA mmap_size=0 on xRead fault connection"
    })
}

fn run_write_or_sync_fault(
    vfs: &TestVfs,
    directory: &Path,
    id: &str,
    operation: IoOp,
    return_code: c_int,
    prefix_bytes: Option<c_int>,
    occurrence: usize,
) -> Result<Value, Box<dyn std::error::Error>> {
    let path = directory.join(format!("{id}.sqlite"));
    create_seed(&path, vfs)?;
    let before = verify_db(&path)?;
    if before["integrity_check"] != "ok" || before["row_count"] != 16 {
        return Err(format!("{id} seed baseline mismatch: {before}").into());
    }
    let before_hashes = file_hash_snapshot(&path);
    let mut connection = open_vfs_rw(&path, vfs)?;
    configure_fixture_connection(&mut connection)?;
    connection.execute_batch("BEGIN IMMEDIATE")?;
    connection.execute(
        "INSERT INTO docs(uid,name,path,payload) VALUES(?1,?2,?3,?4)",
        params![
            "issue381-b1-f0-injected-row",
            "injected transaction marker",
            "/synthetic/f0/injected.txt",
            "transaction payload"
        ],
    )?;
    let wal_path = PathBuf::from(format!("{}-wal", path.display()));
    let mut plan = fault_plan(
        id,
        operation,
        FileKind::Wal,
        &wal_path,
        return_code,
        prefix_bytes,
    )?;
    plan.occurrence = occurrence;
    vfs.arm(plan.clone())?;
    let commit = connection.execute_batch("COMMIT");
    let commit_details = error_details(commit, &connection);
    let rollback = if connection.is_autocommit() {
        json!({"ok":true,"skipped":"SQLite returned to autocommit after COMMIT"})
    } else {
        error_details(connection.execute_batch("ROLLBACK"), &connection)
    };
    drop(connection);
    let plan_after = vfs.fault_plan()?.ok_or("fault plan disappeared")?;
    let after = verify_db(&path)?;
    let after_hashes = file_hash_snapshot(&path);
    let injected_events = vfs
        .events()?
        .into_iter()
        .filter(|event| event.injection_id.as_deref() == Some(id))
        .collect::<Vec<_>>();
    let expected = i64::from(return_code);
    let actual = commit_details["extended_code"].as_i64();
    let code_ok = !commit_details["ok"].as_bool().unwrap_or(false) && actual == Some(expected);
    let integrity_ok = after["integrity_check"] == "ok" && after["fts5_integrity_check"] == "PASS";
    let no_phantom_commit = after["injected_row_count"] == 0;
    let require_no_phantom_commit =
        matches!(id, "B1-F0-SQLITE_FULL" | "B08-SQLITE-FULL") || prefix_bytes.is_some();
    let fired_ok = plan_after.fired && injected_events.len() == 1;
    let partial_prefix_ok = prefix_bytes.is_none()
        || injected_events.first().is_some_and(|event| {
            event
                .prefix_bytes
                .zip(event.requested_bytes)
                .is_some_and(|(prefix, requested)| prefix > 0 && prefix < requested)
        });
    let ok = code_ok
        && integrity_ok
        && fired_ok
        && partial_prefix_ok
        && (!require_no_phantom_commit || no_phantom_commit)
        && after["row_count"]
            .as_u64()
            .is_some_and(|n| n == 16 || n == 17);
    let details = json!({
        "seed":F0_SEED,
        "source":"actual named sqlite3_vfs xWrite/xSync callback",
        "database_path":path.display().to_string(),
        "target_file_kind":"WAL",
        "target_callback":operation.as_str(),
        "injection_id":id,
        "injection_occurrence":plan.occurrence,
        "requested_partial_prefix_bytes":prefix_bytes,
        "actual_partial_prefix_verified":partial_prefix_ok,
        "injection_reached":plan_after.fired,
        "actual_injection_events":injected_events,
        "expected_sqlite_extended_code":expected,
        "sqlite_commit_result":commit_details,
        "sqlite_rollback_result":rollback,
        "fresh_default_vfs_reopen":after,
        "before_file_hashes":before_hashes,
        "after_file_hashes":after_hashes,
        "no_phantom_commit_required":require_no_phantom_commit,
        "no_phantom_commit":no_phantom_commit,
        "fault_model_claim_only":true,
        "ok":ok
    });
    emit(
        "case_result",
        id,
        if ok { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !ok {
        return Err(format!("{id} feasibility assertions failed: {details}").into());
    }
    Ok(details)
}

fn run_short_read(
    vfs: &TestVfs,
    directory: &Path,
    id: &str,
    filename: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let path = directory.join(filename);
    create_seed(&path, vfs)?;
    let before = verify_db(&path)?;
    let before_hashes = file_hash_snapshot(&path);
    let mut connection = open_vfs_rw(&path, vfs)?;
    configure_fixture_connection(&mut connection)?;
    // mmap is disabled here so SQLite page reads pass through xRead.
    connection.pragma_update(None, "mmap_size", 0_i64)?;
    let plan = fault_plan(
        id,
        IoOp::Read,
        FileKind::MainDb,
        &path,
        sqlite::SQLITE_IOERR_SHORT_READ,
        None,
    )?;
    vfs.arm(plan.clone())?;
    let query = connection.query_row("SELECT sum(length(payload)) FROM docs", [], |row| {
        row.get::<_, i64>(0)
    });
    let query_details = match query {
        Ok(value) => json!({"ok":true,"value":value,"primary_code":null,"extended_code":null}),
        Err(error) => {
            let (primary, extended) = sqlite_codes(&error, &connection);
            json!({
                "ok":false,
                "value":null,
                "primary_code":primary,
                "primary_code_name":sqlite_code_name(primary),
                "extended_code":extended,
                "extended_code_name":sqlite_code_name(extended),
                "error":error.to_string(),
                "debug":format!("{error:?}")
            })
        }
    };
    drop(connection);
    let plan_after = vfs
        .fault_plan()?
        .ok_or("short-read fault plan disappeared")?;
    let after = verify_db(&path)?;
    let after_hashes = file_hash_snapshot(&path);
    let injected_events = vfs
        .events()?
        .into_iter()
        .filter(|event| event.injection_id.as_deref() == Some(id))
        .collect::<Vec<_>>();
    let injected = injected_events.first();
    let sqlite_statement_code = query_details["extended_code"].as_i64();
    let sqlite_failed_with_database_error = !query_details["ok"].as_bool().unwrap_or(false)
        && (sqlite_statement_code == Some(i64::from(sqlite::SQLITE_IOERR_SHORT_READ))
            || sqlite_statement_code == Some(i64::from(sqlite::SQLITE_CORRUPT)));
    let callback_code_ok =
        injected.is_some_and(|event| event.result_code == sqlite::SQLITE_IOERR_SHORT_READ);
    let buffer_ok = injected.is_some_and(|event| {
        event.result_code == sqlite::SQLITE_IOERR_SHORT_READ
            && event.zero_fill_verified == Some(true)
            && event.zero_fill_bytes.unwrap_or_default() > 0
            && event.prefix_bytes.unwrap_or_default() > 0
    });
    let recovery_ok = after["integrity_check"] == "ok"
        && after["fts5_integrity_check"] == "PASS"
        && after["row_count"] == 16
        && after["injected_row_count"] == 0;
    let ok = plan_after.fired
        && injected_events.len() == 1
        && callback_code_ok
        && sqlite_failed_with_database_error
        && buffer_ok
        && recovery_ok;
    let details = json!({
        "seed":F0_SEED,
        "source":"actual named sqlite3_vfs xRead callback",
        "database_path":path.display().to_string(),
        "injection_id":id,
        "injection_occurrence":plan.occurrence,
        "injection_reached":plan_after.fired,
        "actual_injection_events":injected_events,
        "vfs_returned_expected_extended_code":sqlite::SQLITE_IOERR_SHORT_READ,
        "sqlite_query_result":query_details,
        "sqlite_translated_short_read_to_database_error":sqlite_statement_code
            == Some(i64::from(sqlite::SQLITE_CORRUPT)),
        "mmap_size_for_injection_connection":0,
        "short_read_buffer_zero_fill_verified_inside_callback":buffer_ok,
        "sqlite_statement_failed_as_expected":sqlite_failed_with_database_error,
        "before_default_vfs_reopen":before,
        "fresh_default_vfs_reopen":after,
        "before_file_hashes":before_hashes,
        "after_file_hashes":after_hashes,
        "fault_model_claim_only":true,
        "ok":ok
    });
    emit(
        "case_result",
        id,
        if ok { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !ok {
        return Err(format!("{id} feasibility assertions failed: {details}").into());
    }
    Ok(details)
}

fn temp_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path =
        std::env::temp_dir().join(format!("issue381-b1-f0-{}-{timestamp}", std::process::id()));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

fn open_pointer_db(path: &Path, vfs: &TestVfs) -> Result<Connection, rusqlite::Error> {
    let mut connection = open_vfs_rw(path, vfs)?;
    configure_fixture_connection(&mut connection)?;
    Ok(connection)
}

fn create_pointer_db(
    path: &Path,
    vfs: &TestVfs,
    slot: &str,
    generation: i64,
    epoch: i64,
    digest: &str,
    manifest_sha256: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = open_pointer_db(path, vfs)?;
    connection.execute_batch(
        "CREATE TABLE pointer(singleton INTEGER PRIMARY KEY CHECK(singleton=1),\
         slot TEXT NOT NULL,generation INTEGER NOT NULL,epoch INTEGER NOT NULL,\
         digest TEXT NOT NULL,manifest_sha256 TEXT NOT NULL);\
         CREATE TABLE pointer_ops(\
           op_id TEXT PRIMARY KEY,expected_epoch INTEGER NOT NULL,\
           target_slot TEXT NOT NULL,target_generation INTEGER NOT NULL,\
           target_epoch INTEGER NOT NULL,target_digest TEXT NOT NULL,\
           target_manifest_sha256 TEXT NOT NULL);\
         CREATE TABLE user_facts(legacy_file_id TEXT NOT NULL,fact_key TEXT NOT NULL,\
           fact_value TEXT NOT NULL,PRIMARY KEY(legacy_file_id,fact_key));\
         INSERT INTO user_facts VALUES('legacy-file-001','tag','keep-me');\
         INSERT INTO user_facts VALUES('legacy-file-001','note','retain historical fact');",
    )?;
    connection.execute(
        "INSERT INTO pointer VALUES(1,?1,?2,?3,?4,?5)",
        params![slot, generation, epoch, digest, manifest_sha256],
    )?;
    drop(connection);
    Ok(())
}

fn add_generation_marker(
    path: &Path,
    vfs: &TestVfs,
    generation: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = open_vfs_rw(path, vfs)?;
    connection.execute(
        "INSERT INTO docs(uid,name,path,payload) VALUES(?1,?2,?3,?4)",
        params![
            format!("{generation}-generation-marker"),
            format!("{generation} generation marker"),
            format!("/synthetic/{generation}/marker.txt"),
            format!("generation-specific payload {generation}")
        ],
    )?;
    drop(connection);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PointerSnapshot {
    slot: String,
    generation: i64,
    epoch: i64,
    digest: String,
    manifest_sha256: String,
}

fn open_vfs_readonly(path: &Path, vfs: &TestVfs) -> Result<Connection, rusqlite::Error> {
    let connection = Connection::open_with_flags_and_vfs(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        vfs.name(),
    )?;
    connection.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    Ok(connection)
}

fn pointer_state(path: &Path, vfs: &TestVfs) -> Result<PointerSnapshot, rusqlite::Error> {
    let connection = open_vfs_readonly(path, vfs)?;
    connection.query_row(
        "SELECT slot,generation,epoch,digest,manifest_sha256 FROM pointer WHERE singleton=1",
        [],
        |row| {
            Ok(PointerSnapshot {
                slot: row.get(0)?,
                generation: row.get(1)?,
                epoch: row.get(2)?,
                digest: row.get(3)?,
                manifest_sha256: row.get(4)?,
            })
        },
    )
}

// Test-only Manifest for the isolated B1 fixture DBs; it is not the product schema.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct FixtureManifest {
    format_version: u32,
    slot: String,
    generation: i64,
    sqlite_file_sha256: String,
    sqlite_file_bytes: u64,
    sqlite_source_id: String,
    source_revision: i64,
}

fn fixture_file_digest(path: &Path) -> Result<(String, u64), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    Ok((
        format!("{:x}", sha2::Sha256::digest(&bytes)),
        bytes.len() as u64,
    ))
}

fn fixture_source_id(path: &Path, vfs: &TestVfs) -> Result<String, rusqlite::Error> {
    let connection = open_vfs_readonly(path, vfs)?;
    connection.query_row("SELECT sqlite_source_id()", [], |row| row.get(0))
}

fn create_fixture_manifest(
    database: &Path,
    manifest_path: &Path,
    slot: &str,
    generation: i64,
    source_revision: i64,
    vfs: &TestVfs,
) -> Result<(FixtureManifest, String), Box<dyn std::error::Error>> {
    let (sqlite_file_sha256, sqlite_file_bytes) = fixture_file_digest(database)?;
    let manifest = FixtureManifest {
        format_version: 1,
        slot: slot.to_string(),
        generation,
        sqlite_file_sha256,
        sqlite_file_bytes,
        sqlite_source_id: fixture_source_id(database, vfs)?,
        source_revision,
    };
    let bytes = serde_json::to_vec(&manifest)?;
    let manifest_sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
    std::fs::write(manifest_path, bytes)?;
    Ok((manifest, manifest_sha256))
}

fn validate_fixture_manifest(
    database: &Path,
    manifest_path: &Path,
    authority_manifest_sha256: &str,
    expected_slot: &str,
    expected_generation: i64,
    expected_file_sha256: &str,
    expected_source_id: &str,
    expected_source_revision: i64,
    vfs: &TestVfs,
) -> Value {
    let mut reasons = Vec::<String>::new();
    let mut checks = json!({
        "database_path":database.display().to_string(),
        "manifest_path":manifest_path.display().to_string(),
        "authority_manifest_sha256":authority_manifest_sha256,
        "expected_slot":expected_slot,
        "expected_generation":expected_generation,
        "expected_file_sha256":expected_file_sha256,
        "expected_source_id":expected_source_id,
        "expected_source_revision":expected_source_revision
    });

    let manifest_bytes = match std::fs::read(manifest_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            reasons.push("MANIFEST_MISSING".to_string());
            Vec::new()
        }
        Err(error) => {
            reasons.push(format!("MANIFEST_READ_ERROR:{error}"));
            Vec::new()
        }
    };
    if !manifest_bytes.is_empty() {
        let actual_manifest_sha256 = format!("{:x}", sha2::Sha256::digest(&manifest_bytes));
        checks["actual_manifest_sha256"] = json!(actual_manifest_sha256);
        checks["manifest_digest_matches_authority"] =
            json!(actual_manifest_sha256 == authority_manifest_sha256);
        if actual_manifest_sha256 != authority_manifest_sha256 {
            reasons.push("MANIFEST_DIGEST_MISMATCH".to_string());
        }
        match serde_json::from_slice::<FixtureManifest>(&manifest_bytes) {
            Ok(manifest) => {
                let identity_ok = manifest.format_version == 1
                    && manifest.slot == expected_slot
                    && manifest.generation == expected_generation
                    && manifest.source_revision == expected_source_revision
                    && manifest.sqlite_source_id == expected_source_id;
                checks["manifest_identity"] = json!({
                    "format_version":manifest.format_version,
                    "slot":manifest.slot,
                    "generation":manifest.generation,
                    "source_id":manifest.sqlite_source_id,
                    "source_revision":manifest.source_revision,
                    "matches_expected":identity_ok
                });
                if !identity_ok {
                    reasons.push("MANIFEST_IDENTITY_MISMATCH".to_string());
                }
                match fixture_file_digest(database) {
                    Ok((actual_sha256, actual_bytes)) => {
                        let digest_ok = actual_sha256 == expected_file_sha256
                            && actual_sha256 == manifest.sqlite_file_sha256;
                        let length_ok = actual_bytes == manifest.sqlite_file_bytes;
                        checks["disk_file"] = json!({
                            "exists":true,
                            "sha256":actual_sha256,
                            "bytes":actual_bytes,
                            "digest_matches_authority_and_manifest":digest_ok,
                            "length_matches_manifest":length_ok
                        });
                        if !digest_ok {
                            reasons.push("DISK_DIGEST_MISMATCH".to_string());
                        }
                        if !length_ok {
                            reasons.push("DISK_LENGTH_MISMATCH".to_string());
                        }
                    }
                    Err(error) => {
                        checks["disk_file"] =
                            json!({"exists":database.exists(),"error":error.to_string()});
                        reasons.push(if database.exists() {
                            format!("DISK_READ_ERROR:{error}")
                        } else {
                            "SLOT_FILE_MISSING".to_string()
                        });
                    }
                }
                match open_vfs_readonly(database, vfs) {
                    Ok(connection) => {
                        let integrity = connection
                            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0));
                        let actual_source_id =
                            connection.query_row("SELECT sqlite_source_id()", [], |row| {
                                row.get::<_, String>(0)
                            });
                        match integrity {
                            Ok(result) => {
                                checks["sqlite_integrity_check"] = json!(result);
                                if result != "ok" {
                                    reasons.push("SQLITE_INTEGRITY_REJECTED".to_string());
                                }
                            }
                            Err(error) => {
                                checks["sqlite_integrity_check"] =
                                    json!({"error":error.to_string()});
                                reasons.push(format!("SQLITE_INTEGRITY_REJECTED:{error}"));
                            }
                        }
                        match actual_source_id {
                            Ok(source_id) => {
                                checks["disk_source_id"] = json!(source_id);
                                if source_id != expected_source_id {
                                    reasons.push("SQLITE_SOURCE_ID_MISMATCH".to_string());
                                }
                            }
                            Err(error) => {
                                reasons.push(format!("SQLITE_SOURCE_ID_READ_ERROR:{error}"))
                            }
                        }
                    }
                    Err(error) => {
                        checks["sqlite_open_readonly"] = json!({"error":error.to_string()});
                        reasons.push(format!("SQLITE_INTEGRITY_REJECTED:{error}"));
                    }
                }
            }
            Err(error) => {
                checks["manifest_parse_error"] = json!(error.to_string());
                reasons.push("MANIFEST_INVALID_JSON".to_string());
            }
        }
    }
    reasons.sort();
    reasons.dedup();
    json!({"valid":reasons.is_empty(),"reasons":reasons,"checks":checks})
}

// Resolve only the generation named by the queried authority row; never repair or fall back.
fn resolve_generation(
    pointer_path: &Path,
    e0_database: &Path,
    e0_manifest: &Path,
    e1_database: &Path,
    e1_manifest: &Path,
    expected_e0_digest: &str,
    expected_e1_digest: &str,
    expected_source_id: &str,
    expected_source_revision: i64,
    vfs: &TestVfs,
) -> Result<Value, Box<dyn std::error::Error>> {
    let pointer = pointer_state(pointer_path, vfs)?;
    let (slot, generation, database, manifest_path, expected_digest) =
        match (pointer.slot.as_str(), pointer.generation) {
            ("E0", 0) => ("E0", 0, e0_database, e0_manifest, expected_e0_digest),
            ("E1", 1) => ("E1", 1, e1_database, e1_manifest, expected_e1_digest),
            _ => {
                return Ok(json!({
                    "decision":"UNAVAILABLE",
                    "reason":"AUTHORITY_GENERATION_UNKNOWN",
                    "pointer":pointer,
                    "automatic_generation_substitution":false,
                    "pointer_rewrite":false
                }));
            }
        };
    if pointer.epoch != generation || pointer.digest != expected_digest {
        return Ok(json!({
            "decision":"UNAVAILABLE",
            "reason":"AUTHORITY_EPOCH_OR_DIGEST_MISMATCH",
            "pointer":pointer,
            "selected_slot":slot,
            "automatic_generation_substitution":false,
            "pointer_rewrite":false
        }));
    }
    let validation = validate_fixture_manifest(
        database,
        manifest_path,
        &pointer.manifest_sha256,
        slot,
        generation,
        expected_digest,
        expected_source_id,
        expected_source_revision,
        vfs,
    );
    let valid = validation["valid"] == true;
    Ok(json!({
        "decision":if valid {"VALIDATED_GENERATION"} else {"UNAVAILABLE"},
        "reason":if valid {"MANIFEST_SLOT_AND_AUTHORITY_VALID"} else {"SELECTED_GENERATION_VALIDATION_FAILED"},
        "selected_slot":slot,
        "selected_generation":generation,
        "pointer":pointer,
        "validation":validation,
        "automatic_generation_substitution":false,
        "pointer_rewrite":false
    }))
}

// Test-only publish guard shared by B01 and B03, including a valid-manifest control.
fn publication_admission(
    pointer_path: &Path,
    candidate_database: &Path,
    candidate_manifest: &Path,
    expected_parent: &PointerSnapshot,
    candidate_slot: &str,
    candidate_generation: i64,
    candidate_epoch: i64,
    expected_candidate_digest: &str,
    expected_candidate_manifest_sha256: &str,
    expected_source_id: &str,
    expected_source_revision: i64,
    vfs: &TestVfs,
) -> Result<Value, Box<dyn std::error::Error>> {
    let observed_pointer = pointer_state(pointer_path, vfs)?;
    if &observed_pointer != expected_parent
        || candidate_generation != expected_parent.generation + 1
        || candidate_epoch != expected_parent.epoch + 1
    {
        return Ok(json!({
            "decision":"REJECTED",
            "reason":"AUTHORITY_EPOCH_CAS_FAILED",
            "observed_pointer":observed_pointer,
            "expected_parent":expected_parent,
            "candidate_slot":candidate_slot,
            "candidate_generation":candidate_generation,
            "candidate_epoch":candidate_epoch
        }));
    }
    let validation = validate_fixture_manifest(
        candidate_database,
        candidate_manifest,
        expected_candidate_manifest_sha256,
        candidate_slot,
        candidate_generation,
        expected_candidate_digest,
        expected_source_id,
        expected_source_revision,
        vfs,
    );
    let allowed = validation["valid"] == true;
    Ok(json!({
        "decision":if allowed {"ADMITTED"} else {"REJECTED"},
        "reason":if allowed {"AUTHORITY_CAS_AND_MANIFEST_VALID"} else {"CANDIDATE_MANIFEST_OR_SLOT_INVALID"},
        "observed_pointer":observed_pointer,
        "expected_parent":expected_parent,
        "candidate_slot":candidate_slot,
        "candidate_generation":candidate_generation,
        "candidate_epoch":candidate_epoch,
        "candidate_manifest_validation":validation
    }))
}

fn remove_sqlite_sidecars(path: &Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
        let _ = std::fs::remove_file(sidecar);
    }
}

fn user_facts_snapshot(path: &Path, vfs: &TestVfs) -> Result<Value, rusqlite::Error> {
    let connection = open_vfs_readonly(path, vfs)?;
    let mut statement = connection.prepare(
        "SELECT legacy_file_id,fact_key,fact_value FROM user_facts ORDER BY legacy_file_id,fact_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let facts = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(json!(facts))
}

fn pointer_operation_snapshot(path: &Path, vfs: &TestVfs) -> Result<Value, rusqlite::Error> {
    let pointer = pointer_state(path, vfs)?;
    let connection = open_vfs_readonly(path, vfs)?;
    let mut statement = connection.prepare(
        "SELECT op_id,expected_epoch,target_slot,target_generation,target_epoch,target_digest,\
         target_manifest_sha256 FROM pointer_ops ORDER BY op_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let operations = rows.collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    drop(connection);
    Ok(
        json!({"pointer":pointer,"operations":operations,"user_facts":user_facts_snapshot(path,vfs)?}),
    )
}

#[derive(Clone, Debug)]
struct PublishOperation {
    op_id: String,
    expected_epoch: i64,
    target_slot: String,
    target_generation: i64,
    target_digest: String,
    target_manifest_sha256: String,
}

fn apply_publish_operation(
    path: &Path,
    vfs: &TestVfs,
    operation: &PublishOperation,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut connection = open_pointer_db(path, vfs)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = transaction
        .query_row(
            "SELECT expected_epoch,target_slot,target_generation,target_epoch,target_digest,\
             target_manifest_sha256 FROM pointer_ops WHERE op_id=?1",
            [&operation.op_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        let identical = existing
            == (
                operation.expected_epoch,
                operation.target_slot.clone(),
                operation.target_generation,
                operation.expected_epoch + 1,
                operation.target_digest.clone(),
                operation.target_manifest_sha256.clone(),
            );
        transaction.rollback()?;
        return Ok(json!({
            "status":if identical {"ALREADY_APPLIED"} else {"REJECTED_OPERATION_ID_REUSE"},
            "operation_id":operation.op_id,
            "existing_operation_matches_replay":identical,
            "sqlite_commit_executed":false
        }));
    }
    let observed_epoch: i64 =
        transaction.query_row("SELECT epoch FROM pointer WHERE singleton=1", [], |row| {
            row.get(0)
        })?;
    if observed_epoch != operation.expected_epoch {
        transaction.rollback()?;
        return Ok(json!({
            "status":"REJECTED_EPOCH_CAS",
            "operation_id":operation.op_id,
            "expected_epoch":operation.expected_epoch,
            "observed_epoch":observed_epoch,
            "sqlite_commit_executed":false
        }));
    }
    let target_epoch = operation.expected_epoch + 1;
    let changed = transaction.execute(
        "UPDATE pointer SET slot=?1,generation=?2,epoch=?3,digest=?4,manifest_sha256=?5 \
         WHERE singleton=1 AND epoch=?6",
        params![
            &operation.target_slot,
            operation.target_generation,
            target_epoch,
            &operation.target_digest,
            &operation.target_manifest_sha256,
            operation.expected_epoch
        ],
    )?;
    if changed != 1 {
        transaction.rollback()?;
        return Ok(
            json!({"status":"REJECTED_EPOCH_CAS","changed_rows":changed,"sqlite_commit_executed":false}),
        );
    }
    transaction.execute(
        "INSERT INTO pointer_ops(op_id,expected_epoch,target_slot,target_generation,target_epoch,\
         target_digest,target_manifest_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            &operation.op_id,
            operation.expected_epoch,
            &operation.target_slot,
            operation.target_generation,
            target_epoch,
            &operation.target_digest,
            &operation.target_manifest_sha256
        ],
    )?;
    transaction.commit()?;
    Ok(json!({
        "status":"APPLIED",
        "operation_id":operation.op_id,
        "expected_epoch":operation.expected_epoch,
        "target_epoch":target_epoch,
        "sqlite_commit_executed":true,
        "sqlite_commit_primary_code":sqlite::SQLITE_OK,
        "sqlite_commit_extended_code":sqlite::SQLITE_OK
    }))
}

fn run_b01_partial_slot_page(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B01-PARTIAL-SLOT-PAGE";
    let e0 = directory.join("b01-E0.sqlite");
    let e1 = directory.join("b01-E1-candidate.sqlite");
    let pointer_path = directory.join("b01-pointer.sqlite");
    let e0_manifest = directory.join("b01-E0.manifest.json");
    let e1_manifest = directory.join("b01-E1.manifest.json");
    create_seed(&e0, vfs)?;
    create_seed(&e1, vfs)?;
    add_generation_marker(&e1, vfs, "b01-E1")?;
    {
        let connection = open_vfs_rw(&e1, vfs)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
    }
    let (e0_digest, _) = fixture_file_digest(&e0)?;
    let (e1_expected_digest, _) = fixture_file_digest(&e1)?;
    let expected_source_id = fixture_source_id(&e0, vfs)?;
    let (_, e0_manifest_sha256) = create_fixture_manifest(&e0, &e0_manifest, "E0", 0, 1, vfs)?;
    create_pointer_db(
        &pointer_path,
        vfs,
        "E0",
        0,
        0,
        &e0_digest,
        &e0_manifest_sha256,
    )?;
    let expected_parent = pointer_state(&pointer_path, vfs)?;
    let e0_before = file_hash_snapshot(&e0);
    let connection = open_vfs_rw(&e1, vfs)?;
    connection.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    connection.execute_batch("BEGIN IMMEDIATE")?;
    connection.execute(
        "UPDATE docs SET payload=?1 WHERE id=1",
        [format!("partial-page-{}", "x".repeat(12_000))],
    )?;
    let plan = fault_plan(
        id,
        IoOp::Write,
        FileKind::MainDb,
        &e1,
        sqlite::SQLITE_IOERR_WRITE,
        Some(256),
    )?;
    vfs.arm(plan.clone())?;
    let commit = connection.execute_batch("COMMIT");
    let commit_details = error_details(commit, &connection);
    let rollback = if connection.is_autocommit() {
        json!({"ok":true,"skipped":"SQLite returned to autocommit after COMMIT"})
    } else {
        error_details(connection.execute_batch("ROLLBACK"), &connection)
    };
    drop(connection);
    let plan_after = vfs.fault_plan()?.ok_or("B01 fault plan disappeared")?;
    let events = vfs
        .events()?
        .into_iter()
        .filter(|event| event.injection_id.as_deref() == Some(id))
        .collect::<Vec<_>>();
    let e0_after = file_hash_snapshot(&e0);
    let e0_connection = open_vfs_readonly(&e0, vfs)?;
    let e0_integrity: String =
        e0_connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    drop(e0_connection);
    let pointer = pointer_state(&pointer_path, vfs)?;
    let candidate_reopen = verify_db(&e1);
    let candidate_reopen_ok = candidate_reopen
        .as_ref()
        .is_ok_and(|result| result["integrity_check"] == "ok");
    let (candidate_digest_after, _) = fixture_file_digest(&e1)?;
    let recovery = resolve_generation(
        &pointer_path,
        &e0,
        &e0_manifest,
        &e1,
        &e1_manifest,
        &e0_digest,
        &e1_expected_digest,
        &expected_source_id,
        1,
        vfs,
    )?;
    let publish_admission = publication_admission(
        &pointer_path,
        &e1,
        &e1_manifest,
        &expected_parent,
        "E1",
        1,
        1,
        &candidate_digest_after,
        "",
        &expected_source_id,
        1,
        vfs,
    )?;
    let candidate_was_published = pointer.slot == "E1"
        && pointer.generation == 1
        && pointer.epoch == 1
        && pointer.digest == candidate_digest_after
        && recovery["decision"] == "VALIDATED_GENERATION"
        && recovery["selected_slot"] == "E1";
    let candidate_publish_allowed = publish_admission["decision"] == "ADMITTED";
    let expected_rc = i64::from(sqlite::SQLITE_IOERR_WRITE);
    let callback_ok = events.len() == 1
        && events[0].file_kind == FileKind::MainDb
        && events[0]
            .prefix_bytes
            .is_some_and(|bytes| bytes > 0 && bytes < 12_000)
        && events[0].result_code == sqlite::SQLITE_IOERR_WRITE;
    let sqlite_error_ok = commit_details["extended_code"].as_i64() == Some(expected_rc);
    let e0_unchanged = e0_before == e0_after;
    let safe = plan_after.fired
        && callback_ok
        && sqlite_error_ok
        && journal_mode == "delete"
        && e0_unchanged
        && e0_integrity == "ok"
        && pointer.slot == "E0"
        && pointer.generation == 0
        && pointer.epoch == 0
        && recovery["decision"] == "VALIDATED_GENERATION"
        && recovery["selected_slot"] == "E0"
        && publish_admission["decision"] == "REJECTED"
        && !candidate_publish_allowed
        && candidate_reopen_ok
        && publish_admission["candidate_manifest_validation"]["reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason == "MANIFEST_MISSING"))
        && !candidate_was_published;
    let details = json!({
        "evidence_class":"ACTUAL_OS plus actual named-VFS xWrite; model publication authority",
        "fault_model_claim_only":true,
        "failure_injection_location":{"file_kind":"MAIN_DB","callback":"xWrite","path":e1.display().to_string(),"prefix_bytes":256,"returned_extended_code":sqlite::SQLITE_IOERR_WRITE},
        "actual_injection_events":events,
        "sqlite_commit_result":commit_details,
        "sqlite_rollback_result":rollback,
        "journal_mode":journal_mode,
        "E0_sha256_before_after":[e0_before,e0_after],
        "E0_integrity_after_fault":e0_integrity,
        "publication_pointer_after_fault":pointer,
        "candidate_reopen_after_rollback":match candidate_reopen { Ok(v) => json!({"ok":true,"integrity":v["integrity_check"],"row_count":v["row_count"]}), Err(e) => json!({"ok":false,"error":e.to_string()}) },
        "candidate_was_published":candidate_was_published,
        "candidate_publish_allowed":candidate_publish_allowed,
        "candidate_publish_admission":publish_admission,
        "read_resolution_after_fault":recovery,
        "ok":safe
    });
    emit(
        "case_result",
        id,
        if safe { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !safe {
        return Err(format!("{id} assertions failed: {details}").into());
    }
    Ok(details)
}

fn run_b02_wal_checkpoint_truncate(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B02-WAL-CHECKPOINT-TRUNCATE";
    let path = directory.join("b02-wal.sqlite");
    create_seed(&path, vfs)?;
    let mut connection = open_vfs_rw(&path, vfs)?;
    configure_fixture_connection(&mut connection)?;
    connection.pragma_update(None, "wal_autocheckpoint", 0_i64)?;
    connection.execute_batch("BEGIN IMMEDIATE")?;
    connection.execute(
        "INSERT INTO docs(uid,name,path,payload) VALUES(?1,?2,?3,?4)",
        params![
            "b02-committed",
            "wal checkpoint boundary",
            "/synthetic/b02.txt",
            "wal".repeat(2048)
        ],
    )?;
    let commit = connection.execute_batch("COMMIT");
    let commit_details = error_details(commit, &connection);
    let wal_path = PathBuf::from(format!("{}-wal", path.display()));
    let wal_bytes_before_checkpoint = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
    let plan = fault_plan(
        id,
        IoOp::Truncate,
        FileKind::Wal,
        &wal_path,
        sqlite::SQLITE_IOERR_TRUNCATE,
        None,
    )?;
    vfs.arm(plan.clone())?;
    let checkpoint = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    let checkpoint_details = match checkpoint {
        Ok((busy, log, checkpointed)) => {
            json!({"ok":true,"busy":busy,"log_frames":log,"checkpointed_frames":checkpointed})
        }
        Err(error) => {
            let (primary, extended) = sqlite_codes(&error, &connection);
            json!({"ok":false,"primary_code":primary,"primary_code_name":sqlite_code_name(primary),"extended_code":extended,"extended_code_name":sqlite_code_name(extended),"error":error.to_string()})
        }
    };
    let integrity_while_open: Result<String, rusqlite::Error> =
        connection.query_row("PRAGMA integrity_check", [], |r| r.get(0));
    drop(connection);
    let plan_after = vfs.fault_plan()?.ok_or("B02 fault plan disappeared")?;
    let events = vfs
        .events()?
        .into_iter()
        .filter(|event| event.injection_id.as_deref() == Some(id))
        .collect::<Vec<_>>();
    let after = verify_db(&path)?;
    let sidecars_after_close = file_hash_snapshot(&path);
    let marker_count: i64 = Connection::open(&path)?.query_row(
        "SELECT count(*) FROM docs WHERE uid='b02-committed'",
        [],
        |r| r.get(0),
    )?;
    let callback_ok = events.len() == 1
        && events[0].operation == "xTruncate"
        && events[0].file_kind == FileKind::Wal
        && events[0].result_code == sqlite::SQLITE_IOERR_TRUNCATE;
    let safe = plan_after.fired
        && callback_ok
        && commit_details["ok"] == true
        && after["integrity_check"] == "ok"
        && after["fts5_integrity_check"] == "PASS"
        && after["row_count"] == 17
        && marker_count == 1;
    let details = json!({
        "evidence_class":"ACTUAL_OS and actual named-VFS callbacks; model recovery check",
        "commit_result":commit_details,
        "wal_bytes_before_checkpoint":wal_bytes_before_checkpoint,
        "injection_location":{"file_kind":"WAL","callback":"xTruncate","target_size":0,"returned_extended_code":sqlite::SQLITE_IOERR_TRUNCATE},
        "checkpoint_result":checkpoint_details,
        "integrity_check_while_connection_open":match integrity_while_open { Ok(v) => json!(v), Err(e) => json!({"error":e.to_string()}) },
        "actual_injection_events":events,
        "fresh_default_vfs_recovery":after,
        "committed_row_count_after_recovery":marker_count,
        "WAL_and_sidecar_state_after_close":sidecars_after_close,
        "fault_model_claim_only":true,
        "ok":safe
    });
    emit(
        "case_result",
        id,
        if safe { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !safe {
        return Err(format!("{id} assertions failed: {details}").into());
    }
    Ok(details)
}

fn run_b03_incomplete_manifest(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B03-INCOMPLETE-MANIFEST";
    let e0 = directory.join("b03-E0.sqlite");
    let e1 = directory.join("b03-E1.sqlite");
    let pointer = directory.join("b03-pointer.sqlite");
    let e0_manifest = directory.join("b03-E0.manifest.json");
    let manifest = directory.join("b03-E1.manifest.json");
    let staging = directory.join("b03-E1.manifest.tmp");
    let positive_manifest = directory.join("b03-E1-positive.manifest.json");
    create_seed(&e0, vfs)?;
    create_seed(&e1, vfs)?;
    add_generation_marker(&e1, vfs, "b03-E1")?;
    let (e0_digest, _) = fixture_file_digest(&e0)?;
    let (e1_digest, _) = fixture_file_digest(&e1)?;
    let expected_source_id = fixture_source_id(&e0, vfs)?;
    let (_, e0_manifest_sha256) = create_fixture_manifest(&e0, &e0_manifest, "E0", 0, 1, vfs)?;
    let (_, candidate_manifest_sha256) =
        create_fixture_manifest(&e1, &positive_manifest, "E1", 1, 1, vfs)?;
    let full = std::fs::read(&positive_manifest)?;
    create_pointer_db(&pointer, vfs, "E0", 0, 0, &e0_digest, &e0_manifest_sha256)?;
    let expected_parent = pointer_state(&pointer, vfs)?;
    let cut = (full.len() / 2).max(1);
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&staging)?;
        file.write_all(&full[..cut])?;
        file.sync_all()?;
    }
    let observed_pointer = pointer_state(&pointer, vfs)?;
    let staged = std::fs::read(&staging)?;
    let stage_digest = format!("{:x}", sha2::Sha256::digest(&staged));
    let candidate_digest = fixture_file_digest(&e1)?.0;
    let recovery = resolve_generation(
        &pointer,
        &e0,
        &e0_manifest,
        &e1,
        &manifest,
        &e0_digest,
        &e1_digest,
        &expected_source_id,
        1,
        vfs,
    )?;
    let damaged_admission = publication_admission(
        &pointer,
        &e1,
        &staging,
        &expected_parent,
        "E1",
        1,
        1,
        &candidate_digest,
        &candidate_manifest_sha256,
        &expected_source_id,
        1,
        vfs,
    )?;
    let valid_admission = publication_admission(
        &pointer,
        &e1,
        &positive_manifest,
        &expected_parent,
        "E1",
        1,
        1,
        &candidate_digest,
        &candidate_manifest_sha256,
        &expected_source_id,
        1,
        vfs,
    )?;
    let candidate_publish_allowed = damaged_admission["decision"] == "ADMITTED";
    let candidate_was_published = observed_pointer.slot == "E1"
        && observed_pointer.generation == 1
        && observed_pointer.epoch == 1
        && observed_pointer.digest == candidate_digest
        && recovery["decision"] == "VALIDATED_GENERATION"
        && recovery["selected_slot"] == "E1";
    let safe = !manifest.exists()
        && staged.len() == cut
        && observed_pointer.slot == "E0"
        && observed_pointer.generation == 0
        && observed_pointer.epoch == 0
        && recovery["decision"] == "VALIDATED_GENERATION"
        && recovery["selected_slot"] == "E0"
        && damaged_admission["decision"] == "REJECTED"
        && damaged_admission["candidate_manifest_validation"]["reasons"]
            .as_array()
            .is_some_and(|reasons| {
                reasons
                    .iter()
                    .any(|reason| reason == "MANIFEST_DIGEST_MISMATCH")
                    && reasons
                        .iter()
                        .any(|reason| reason == "MANIFEST_INVALID_JSON")
            })
        && valid_admission["decision"] == "ADMITTED"
        && !candidate_publish_allowed
        && !candidate_was_published;
    let details = json!({
        "evidence_class":"MODEL_ONLY; real temporary-file prefix write and fsync on current OS",
        "injection_point":"after manifest staging prefix is written; before atomic rename/publication",
        "staging_manifest":{"path":staging.display().to_string(),"prefix_bytes":staged.len(),"expected_complete_bytes":full.len(),"sha256":stage_digest,"expected_manifest_sha256":candidate_manifest_sha256},
        "final_manifest_exists":manifest.exists(),
        "candidate_E1_file_sha256":candidate_digest,
        "pointer_after_interruption":observed_pointer,
        "candidate_publish_allowed":candidate_publish_allowed,
        "candidate_publish_admission":damaged_admission,
        "valid_manifest_positive_control":valid_admission,
        "read_resolution_after_interruption":recovery,
        "candidate_was_published":candidate_was_published,
        "real_process_crash_or_power_loss":false,
        "ok":safe
    });
    emit(
        "case_result",
        id,
        if safe { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !safe {
        return Err(format!("{id} assertions failed: {details}").into());
    }
    Ok(details)
}

fn run_b04_manifest_rename_directory_sync(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B04-MANIFEST-RENAME-DIRECTORY-SYNC";
    let e1 = directory.join("b04-E1.sqlite");
    create_seed(&e1, vfs)?;
    let parent = directory.join("b04-win-os5-model");
    std::fs::create_dir_all(&parent)?;
    let temp = parent.join("E1.manifest.tmp");
    let final_path = parent.join("E1.manifest.json");
    let manifest = serde_json::to_vec(
        &json!({"slot":"E1","file_sha256":sha256_file(&e1)["sha256"],"sqlite_file_bytes":std::fs::metadata(&e1)?.len()}),
    )?;
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&manifest)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, &final_path)?;
    let directory_sync = std::fs::File::open(&parent).and_then(|file| file.sync_all());
    let actual_dir_sync = match directory_sync {
        Ok(()) => json!({"outcome":"SYNCED","os":"linux"}),
        Err(error) => {
            json!({"outcome":"FAILED_OR_UNAVAILABLE","kind":format!("{:?}",error.kind()),"raw_os_error":error.raw_os_error(),"os":"linux"})
        }
    };
    let final_bytes = std::fs::read(&final_path)?;
    let rename_ok = !temp.exists() && final_bytes == manifest;
    let details = json!({
        "evidence_class":"ACTUAL_OS for Linux rename/directory sync; NOT_TESTED for Windows OS error 5",
        "linux_atomic_rename_completed":rename_ok,
        "linux_directory_sync":actual_dir_sync,
        "windows_sharing_or_access_denied_os5_injection_attempted":false,
        "windows_native_handle_close_rename_retry_semantics":"ENVIRONMENT_UNAVAILABLE",
        "reason":"Current executor is Linux; the Windows OS 5 path requires native Windows handle/share semantics.",
        "final_manifest_sha256":format!("{:x}",sha2::Sha256::digest(final_bytes)),
        "ok":rename_ok
    });
    emit(
        "case_result",
        id,
        "ENVIRONMENT_UNAVAILABLE",
        details.clone(),
    );
    Ok(details)
}

fn run_b05_pointer_transactions(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B05-PUBLISH-POINTER-TRANSACTIONS";
    let path = directory.join("b05-ack-lost-pointer.sqlite");
    let e0_digest = "sha256:model-E0";
    let e0_manifest = "sha256:model-E0-manifest";
    let e1_digest = "sha256:model-E1";
    let e1_manifest = "sha256:model-E1-manifest";
    create_pointer_db(&path, vfs, "E0", 0, 0, e0_digest, e0_manifest)?;
    let user_facts_before = user_facts_snapshot(&path, vfs)?;
    let connection = open_pointer_db(&path, vfs)?;
    connection.execute_batch("BEGIN IMMEDIATE")?;
    connection.execute(
        "UPDATE pointer SET slot='E1',generation=1,epoch=1,digest=?1,manifest_sha256=?2 \
         WHERE singleton=1 AND epoch=0",
        params![e1_digest, e1_manifest],
    )?;
    connection.execute(
        "INSERT INTO pointer_ops VALUES('pre-commit',0,'E1',1,1,?1,?2)",
        params![e1_digest, e1_manifest],
    )?;
    connection.execute_batch("ROLLBACK")?;
    drop(connection);
    let after_rollback = pointer_operation_snapshot(&path, vfs)?;
    let operation = PublishOperation {
        op_id: "ack-lost-op".to_string(),
        expected_epoch: 0,
        target_slot: "E1".to_string(),
        target_generation: 1,
        target_digest: e1_digest.to_string(),
        target_manifest_sha256: e1_manifest.to_string(),
    };
    let first_apply = apply_publish_operation(&path, vfs, &operation)?;
    let after_commit = pointer_operation_snapshot(&path, vfs)?;
    let facts_after_commit = user_facts_snapshot(&path, vfs)?;
    let replay = apply_publish_operation(&path, vfs, &operation)?;
    let after_replay = pointer_operation_snapshot(&path, vfs)?;
    let mut rebound_operation = operation.clone();
    rebound_operation.target_digest = "sha256:rebound-target".to_string();
    let operation_id_reuse = apply_publish_operation(&path, vfs, &rebound_operation)?;
    let stale_operation = PublishOperation {
        op_id: "stale-epoch-op".to_string(),
        expected_epoch: 0,
        target_slot: "E0".to_string(),
        target_generation: 0,
        target_digest: e0_digest.to_string(),
        target_manifest_sha256: e0_manifest.to_string(),
    };
    let epoch_cas = apply_publish_operation(&path, vfs, &stale_operation)?;
    let after_all_replays = pointer_operation_snapshot(&path, vfs)?;
    let replay_user_facts = user_facts_snapshot(&path, vfs)?;

    let restart_path = directory.join("b05-commit-lost-on-model-restart.sqlite");
    let precommit_snapshot = directory.join("b05-precommit-durable-snapshot.sqlite");
    create_pointer_db(&restart_path, vfs, "E0", 0, 0, e0_digest, e0_manifest)?;
    let restart_user_facts_before = user_facts_snapshot(&restart_path, vfs)?;
    std::fs::copy(&restart_path, &precommit_snapshot)?;
    let restart_commit = apply_publish_operation(&restart_path, vfs, &operation)?;
    let commit_seen_by_caller = restart_commit["status"] == "APPLIED"
        && restart_commit["sqlite_commit_primary_code"] == sqlite::SQLITE_OK
        && restart_commit["sqlite_commit_extended_code"] == sqlite::SQLITE_OK;
    remove_sqlite_sidecars(&restart_path);
    std::fs::remove_file(&restart_path)?;
    std::fs::copy(&precommit_snapshot, &restart_path)?;
    let after_simulated_restart = pointer_operation_snapshot(&restart_path, vfs)?;
    let restart_user_facts_after = user_facts_snapshot(&restart_path, vfs)?;
    let op_rows_after_restart = after_simulated_restart["operations"]
        .as_array()
        .map_or(0, Vec::len);

    let safe = after_rollback["pointer"]["slot"] == "E0"
        && after_rollback["pointer"]["epoch"] == 0
        && after_rollback["operations"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && first_apply["status"] == "APPLIED"
        && first_apply["sqlite_commit_primary_code"] == sqlite::SQLITE_OK
        && replay["status"] == "ALREADY_APPLIED"
        && replay["existing_operation_matches_replay"] == true
        && after_commit == after_replay
        && operation_id_reuse["status"] == "REJECTED_OPERATION_ID_REUSE"
        && epoch_cas["status"] == "REJECTED_EPOCH_CAS"
        && after_commit == after_all_replays
        && after_all_replays["pointer"]["slot"] == "E1"
        && after_all_replays["pointer"]["epoch"] == 1
        && after_all_replays["operations"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1)
        && user_facts_before == facts_after_commit
        && user_facts_before == replay_user_facts
        && restart_user_facts_before == restart_user_facts_after
        && commit_seen_by_caller
        && after_simulated_restart["pointer"]["slot"] == "E0"
        && after_simulated_restart["pointer"]["epoch"] == 0
        && op_rows_after_restart == 0;
    let details = json!({
        "evidence_class":"MODEL_ONLY SQLite transactions through actual named VFS; no power-loss claim",
        "pre_commit_transaction_rolled_back":{"snapshot":after_rollback},
        "commit_succeeded_but_response_lost":{
            "first_apply_result":first_apply,
            "simulated_response_delivery":"DROPPED_AFTER_SQLITE_COMMIT_RETURNED_SQLITE_OK",
            "actual_second_invocation":replay,
            "state_after_first_commit":after_commit,
            "state_after_replay":after_replay,
            "state_idempotent":after_commit == after_replay,
            "duplicate_operation_id_rebind_attempt":operation_id_reuse,
            "stale_epoch_CAS_attempt":epoch_cas,
            "state_after_negative_controls":after_all_replays,
            "user_facts_before":user_facts_before,
            "user_facts_after":replay_user_facts
        },
        "commit_reported_success_but_lost_in_simulated_restart":{
            "commit_result":restart_commit,
            "caller_observed_success":commit_seen_by_caller,
            "recovery_model":"replace the post-commit temp database with a captured pre-commit snapshot",
            "before_restart_model":restart_user_facts_before,
            "after_restart_model":after_simulated_restart,
            "user_facts_after":restart_user_facts_after,
            "operation_rows_after_restart":op_rows_after_restart,
            "physical_restart_or_power_loss_tested":false
        },
        "no_cross_file_ACID_or_power_loss_claim":true,
        "ok":safe
    });
    emit(
        "case_result",
        id,
        if safe { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !safe {
        return Err(format!("{id} assertions failed: {details}").into());
    }
    Ok(details)
}

fn run_b06_e1_unavailable(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B06-EXECUTABLE-RECOVERY-DECISIONS";
    let cases = [
        "valid_e1",
        "missing_e1",
        "truncated_e1",
        "digest_mismatch_e1",
        "invalid_e0",
    ];
    let mut generations = Vec::new();
    for case in cases {
        let e0 = directory.join(format!("b06-{case}-E0.sqlite"));
        let e1 = directory.join(format!("b06-{case}-E1.sqlite"));
        let pointer = directory.join(format!("b06-{case}-pointer.sqlite"));
        let e0_manifest = directory.join(format!("b06-{case}-E0.manifest.json"));
        let e1_manifest = directory.join(format!("b06-{case}-E1.manifest.json"));
        create_seed(&e0, vfs)?;
        create_seed(&e1, vfs)?;
        add_generation_marker(&e0, vfs, &format!("b06-{case}-E0"))?;
        add_generation_marker(&e1, vfs, &format!("b06-{case}-E1"))?;
        let (e0_digest, _) = fixture_file_digest(&e0)?;
        let (e1_digest, _) = fixture_file_digest(&e1)?;
        let expected_source_id = fixture_source_id(&e0, vfs)?;
        let (_, e0_manifest_sha256) = create_fixture_manifest(&e0, &e0_manifest, "E0", 0, 1, vfs)?;
        let (_, e1_manifest_sha256) = create_fixture_manifest(&e1, &e1_manifest, "E1", 1, 1, vfs)?;
        let selected_e0 = case == "invalid_e0";
        let (selected_slot, selected_generation, selected_digest, selected_manifest_sha256) =
            if selected_e0 {
                ("E0", 0, e0_digest.as_str(), e0_manifest_sha256.as_str())
            } else {
                ("E1", 1, e1_digest.as_str(), e1_manifest_sha256.as_str())
            };
        create_pointer_db(
            &pointer,
            vfs,
            selected_slot,
            selected_generation,
            selected_generation,
            selected_digest,
            selected_manifest_sha256,
        )?;
        match case {
            "missing_e1" => std::fs::remove_file(&e1)?,
            "truncated_e1" => {
                let file = std::fs::OpenOptions::new().write(true).open(&e1)?;
                file.set_len(100)?;
                file.sync_all()?;
            }
            "digest_mismatch_e1" => {
                let connection = open_vfs_rw(&e1, vfs)?;
                connection.execute(
                    "UPDATE docs SET payload=substr(payload,1,length(payload)-1)||'x' WHERE id=1",
                    [],
                )?;
                drop(connection);
            }
            "invalid_e0" => {
                let file = std::fs::OpenOptions::new().write(true).open(&e0)?;
                file.set_len(100)?;
                file.sync_all()?;
            }
            _ => {}
        }
        let before = json!({
            "pointer":pointer_state(&pointer,vfs)?,
            "E0":sha256_file(&e0),
            "E1":sha256_file(&e1),
            "E0_manifest":sha256_file(&e0_manifest),
            "E1_manifest":sha256_file(&e1_manifest)
        });
        let resolution = resolve_generation(
            &pointer,
            &e0,
            &e0_manifest,
            &e1,
            &e1_manifest,
            &e0_digest,
            &e1_digest,
            &expected_source_id,
            1,
            vfs,
        )?;
        let after = json!({
            "pointer":pointer_state(&pointer,vfs)?,
            "E0":sha256_file(&e0),
            "E1":sha256_file(&e1),
            "E0_manifest":sha256_file(&e0_manifest),
            "E1_manifest":sha256_file(&e1_manifest)
        });
        let expected_decision = if case == "valid_e1" {
            "VALIDATED_GENERATION"
        } else {
            "UNAVAILABLE"
        };
        let state_unchanged = before == after;
        generations.push(json!({
            "case":case,
            "expected_decision":expected_decision,
            "actual_resolution":resolution,
            "state_before":before,
            "state_after":after,
            "resolver_did_not_mutate_files_or_authority":state_unchanged,
            "E1_file_exists_after_resolution":e1.exists(),
            "automatic_generation_substitution":resolution["automatic_generation_substitution"],
            "pointer_rewrite":resolution["pointer_rewrite"]
        }));
    }
    let safe = generations.iter().all(|case| {
        case["actual_resolution"]["decision"] == case["expected_decision"]
            && case["resolver_did_not_mutate_files_or_authority"] == true
            && case["automatic_generation_substitution"] == false
            && case["pointer_rewrite"] == false
    }) && generations[0]["actual_resolution"]["selected_slot"] == "E1"
        && generations[1]["actual_resolution"]["selected_slot"] == "E1"
        && generations[1]["actual_resolution"]["validation"]["reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason == "SLOT_FILE_MISSING"))
        && generations[2]["actual_resolution"]["selected_slot"] == "E1"
        && generations[2]["actual_resolution"]["validation"]["reasons"]
            .as_array()
            .is_some_and(|reasons| {
                reasons
                    .iter()
                    .any(|reason| reason == "DISK_DIGEST_MISMATCH")
            })
        && generations[3]["actual_resolution"]["selected_slot"] == "E1"
        && generations[3]["actual_resolution"]["validation"]["reasons"]
            .as_array()
            .is_some_and(|reasons| {
                reasons
                    .iter()
                    .any(|reason| reason == "DISK_DIGEST_MISMATCH")
            })
        && generations[4]["actual_resolution"]["selected_slot"] == "E0";
    let details = json!({
        "evidence_class":"MODEL_ONLY test-only recovery resolver using actual pointer queries, actual manifests, read-only SQLite integrity checks and file digests",
        "resolver_inputs":"authority pointer, expected E0/E1 SQLite digests, SQLite source ID, source revision, on-disk Manifest bytes and SQLite integrity",
        "cases":generations,
        "mutable_FTS_fallback":"not part of this resolver",
        "automatic_repair_or_pointer_rewrite":"the resolver only returns a decision; it does not write the pointer or slot files",
        "ok":safe
    });
    emit(
        "case_result",
        id,
        if safe { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !safe {
        return Err(format!("{id} assertions failed: {details}").into());
    }
    Ok(details)
}

fn run_b12_io_measurement(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let id = "B12-VFS-LATENCY-TEMP-OVERHEAD";
    let samples = 30_usize;
    let root = directory.join("b12");
    std::fs::create_dir_all(&root)?;
    let vfs_event_start = vfs.events()?.len();
    let mut default_ms = Vec::with_capacity(samples);
    let mut wrapped_ms = Vec::with_capacity(samples);
    let mut initial_bytes = 0_u64;
    for sample in 0..samples {
        let control = root.join(format!("control-{sample:02}.sqlite"));
        let wrapped = root.join(format!("wrapped-{sample:02}.sqlite"));
        {
            let c = Connection::open(&control)?;
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.pragma_update(None, "synchronous", "FULL")?;
            c.execute_batch("CREATE TABLE samples(id INTEGER PRIMARY KEY,value TEXT NOT NULL);")?;
        }
        std::fs::copy(&control, &wrapped)?;
        let start = std::time::Instant::now();
        {
            let c = Connection::open(&control)?;
            c.execute_batch("BEGIN IMMEDIATE")?;
            c.execute(
                "INSERT INTO samples(value) VALUES(?1)",
                [format!("control-{sample}")],
            )?;
            c.execute_batch("COMMIT")?;
        }
        default_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
        let start = std::time::Instant::now();
        {
            let c = open_vfs_rw(&wrapped, vfs)?;
            c.execute_batch("BEGIN IMMEDIATE")?;
            c.execute(
                "INSERT INTO samples(value) VALUES(?1)",
                [format!("wrapped-{sample}")],
            )?;
            c.execute_batch("COMMIT")?;
        }
        wrapped_ms.push(start.elapsed().as_secs_f64() * 1_000.0);
        initial_bytes += std::fs::metadata(&control)?.len() + std::fs::metadata(&wrapped)?.len();
    }
    let events = vfs.events()?;
    let b12_events = events.into_iter().skip(vfs_event_start).collect::<Vec<_>>();
    let callbacks = json!({
        "xOpen":b12_events.iter().filter(|e| e.operation == "xOpen").count(),
        "xRead":b12_events.iter().filter(|e| e.operation == "xRead").count(),
        "xWrite":b12_events.iter().filter(|e| e.operation == "xWrite").count(),
        "xSync":b12_events.iter().filter(|e| e.operation == "xSync").count()
    });
    let final_bytes = directory_tree_bytes(&root)?;
    let overhead_bytes = final_bytes.saturating_sub(initial_bytes);
    let delta = wrapped_ms
        .iter()
        .zip(&default_ms)
        .map(|(wrapped, control)| wrapped - control)
        .collect::<Vec<_>>();
    let details = json!({
        "evidence_class":"ACTUAL_OS micro-observation with named VFS and default VFS controls",
        "sample_count":samples,
        "sample_definition":"paired disposable SQLite WAL database, one BEGIN/INSERT/COMMIT each, synchronous=FULL; includes open/SQL/close elapsed time",
        "default_vfs_latency_ms":{"p50":percentile_f64(default_ms.clone(),50),"p95":percentile_f64(default_ms.clone(),95),"samples":default_ms},
        "named_vfs_latency_ms":{"p50":percentile_f64(wrapped_ms.clone(),50),"p95":percentile_f64(wrapped_ms.clone(),95),"samples":wrapped_ms},
        "paired_delta_ms":{"p50":percentile_f64(delta.clone(),50),"p95":percentile_f64(delta.clone(),95),"samples":delta},
        "named_vfs_callback_counts":callbacks,
        "measured_db_file_bytes_sum":initial_bytes,
        "temporary_tree_bytes_after_samples":final_bytes,
        "additional_temp_bytes_approx":overhead_bytes,
        "real_storage_pressure_or_product_latency_claim":false,
        "ok":b12_events.iter().any(|e| e.operation == "xWrite") && final_bytes >= initial_bytes
    });
    let ok = details["ok"] == true;
    emit(
        "case_result",
        id,
        if ok { "PASS" } else { "FAIL" },
        details.clone(),
    );
    if !ok {
        return Err(format!("{id} assertions failed: {details}").into());
    }
    Ok(details)
}

fn percentile_f64(values: Vec<f64>, p: usize) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values;
    sorted.sort_by(f64::total_cmp);
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

fn directory_tree_bytes(path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(directory_tree_bytes(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn add_matrix_result(
    id: &str,
    outcome: Result<Value, Box<dyn std::error::Error>>,
    expected_status: Option<&'static str>,
    results: &mut Vec<Value>,
) -> bool {
    match outcome {
        Ok(details) => {
            let status = if details["ok"] == false {
                "FAIL"
            } else {
                expected_status.unwrap_or(if details["ok"] == true {
                    "PASS"
                } else {
                    "FAIL"
                })
            };
            results.push(json!({"case_id":id,"status":status,"details":details}));
            status != "FAIL"
        }
        Err(error) => {
            let details = json!({"error":error.to_string(),"evidence_class":"MODEL_ONLY unless an actual VFS event preceded the failure"});
            emit("case_result", id, "FAIL", details.clone());
            results.push(json!({"case_id":id,"status":"FAIL","details":details}));
            false
        }
    }
}

fn run_matrix_scenarios(
    vfs: &TestVfs,
    directory: &Path,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut results = Vec::with_capacity(12);
    add_matrix_result(
        "B01",
        run_b01_partial_slot_page(vfs, directory),
        None,
        &mut results,
    );
    add_matrix_result(
        "B02",
        run_b02_wal_checkpoint_truncate(vfs, directory),
        None,
        &mut results,
    );
    add_matrix_result(
        "B03",
        run_b03_incomplete_manifest(vfs, directory),
        None,
        &mut results,
    );
    add_matrix_result(
        "B04",
        run_b04_manifest_rename_directory_sync(vfs, directory),
        Some("ENVIRONMENT_UNAVAILABLE"),
        &mut results,
    );
    add_matrix_result(
        "B05",
        run_b05_pointer_transactions(vfs, directory),
        None,
        &mut results,
    );
    add_matrix_result(
        "B06",
        run_b06_e1_unavailable(vfs, directory),
        None,
        &mut results,
    );

    let b07 = match crate::run_b1_gc_model() {
        Ok(ok) => {
            let details = json!({
                "evidence_class":"MODEL_ONLY existing P1A-T05 generation lease/GC prototype",
                "reused_test_case":"P1A-T05",
                "covered":"pin-first, GC-first late-pin rejection, live-handle pin, failed-open release, expiry, child-process exit/reopen, generation ABA, idempotent GC and pool settings",
                "ok":ok
            });
            emit(
                "case_result",
                "B07-PIN-GC-LEASE-ABA",
                if ok { "PASS" } else { "FAIL" },
                details.clone(),
            );
            if ok {
                results.push(json!({"case_id":"B07","status":"PASS","details":details}));
                true
            } else {
                results.push(json!({"case_id":"B07","status":"FAIL","details":details}));
                false
            }
        }
        Err(error) => {
            let details = json!({"error":error.to_string(),"evidence_class":"MODEL_ONLY"});
            emit(
                "case_result",
                "B07-PIN-GC-LEASE-ABA",
                "FAIL",
                details.clone(),
            );
            results.push(json!({"case_id":"B07","status":"FAIL","details":details}));
            false
        }
    };

    let mut b08_subcases = Vec::new();
    for (id, outcome) in [
        (
            "B08-SQLITE-FULL",
            run_write_or_sync_fault(
                vfs,
                directory,
                "B08-SQLITE-FULL",
                IoOp::Write,
                sqlite::SQLITE_FULL,
                None,
                1,
            ),
        ),
        (
            "B08-WAL-PARTIAL-IOERR-WRITE",
            run_write_or_sync_fault(
                vfs,
                directory,
                "B08-WAL-PARTIAL-IOERR-WRITE",
                IoOp::Write,
                sqlite::SQLITE_IOERR_WRITE,
                Some(128),
                3,
            ),
        ),
        (
            "B08-IOERR-FSYNC",
            run_write_or_sync_fault(
                vfs,
                directory,
                "B08-IOERR-FSYNC",
                IoOp::Sync,
                sqlite::SQLITE_IOERR_FSYNC,
                None,
                1,
            ),
        ),
    ] {
        match outcome {
            Ok(details) => b08_subcases.push(json!({"case_id":id,"status":if details["ok"] == true { "PASS" } else { "FAIL" },"details":details})),
            Err(error) => b08_subcases.push(json!({"case_id":id,"status":"FAIL","error":error.to_string()})),
        }
    }
    let short_read = run_short_read(vfs, directory, "B08-SHORT-READ", "b08-short-read.sqlite");
    match short_read {
        Ok(details) => b08_subcases.push(json!({"case_id":"B08-SHORT-READ","status":if details["ok"] == true { "PASS" } else { "FAIL" },"details":details})),
        Err(error) => b08_subcases.push(json!({"case_id":"B08-SHORT-READ","status":"FAIL","error":error.to_string()})),
    }
    let truncate_seen = vfs.events()?.iter().any(|event| {
        event.injection_id.as_deref() == Some("B02-WAL-CHECKPOINT-TRUNCATE")
            && event.operation == "xTruncate"
            && event.result_code == sqlite::SQLITE_IOERR_TRUNCATE
    });
    let b08_ok = b08_subcases.iter().all(|item| item["status"] == "PASS") && truncate_seen;
    let b08_details = json!({
        "evidence_class":"ACTUAL_OS; actual SQLite xWrite/xSync/xRead/xTruncate callbacks",
        "subcases":b08_subcases,
        "wal_truncate_boundary_fault_injected_in":"B02-WAL-CHECKPOINT-TRUNCATE",
        "wal_truncate_callback_observed":truncate_seen,
        "model_only_claim_boundary":"No claim of real media exhaustion, power loss, or torn hardware writes.",
        "ok":b08_ok
    });
    emit(
        "case_result",
        "B08-SQLITE-IO-FAULTS",
        if b08_ok { "PASS" } else { "FAIL" },
        b08_details.clone(),
    );
    results.push(json!({"case_id":"B08","status":if b08_ok { "PASS" } else { "FAIL" },"details":b08_details}));

    let b09 = match crate::run_b1_barrier_model() {
        Ok(ok) => {
            let details = json!({
                "evidence_class":"MODEL_ONLY Linux child-process kill at deterministic barriers",
                "reused_test_case":"P1A-T08 and P1A-T11",
                "barriers":"build, close, sync, seal, pointer before/after commit, GC before/after unlink; missing/corrupt target reconciliation",
                "physical_power_loss_tested":false,
                "ok":ok
            });
            emit(
                "case_result",
                "B09-BARRIER-ORDER-AND-RECOVERY",
                if ok { "PASS" } else { "FAIL" },
                details.clone(),
            );
            results.push(json!({"case_id":"B09","status":if ok { "PASS" } else { "FAIL" },"details":details}));
            ok
        }
        Err(error) => {
            let details = json!({"error":error.to_string(),"evidence_class":"MODEL_ONLY"});
            emit(
                "case_result",
                "B09-BARRIER-ORDER-AND-RECOVERY",
                "FAIL",
                details.clone(),
            );
            results.push(json!({"case_id":"B09","status":"FAIL","details":details}));
            false
        }
    };

    let b10_details = json!({
        "evidence_class":"NOT_TESTED",
        "status_reason":"ENVIRONMENT_UNAVAILABLE",
        "real_bounded_enospc_attempted":false,
        "separate_quota_or_mount_approval_obtained":false,
        "safe_alternative":"Actual SQLite SQLITE_FULL VFS return code is covered by B1-F0 and B08; it does not model real ENOSPC."
    });
    emit(
        "case_result",
        "B10-REAL-ENOSPC",
        "ENVIRONMENT_UNAVAILABLE",
        b10_details.clone(),
    );
    results.push(json!({"case_id":"B10","status":"ENVIRONMENT_UNAVAILABLE","details":b10_details}));

    let b11_details = json!({
        "evidence_class":"NOT_TESTED",
        "status_reason":"ENVIRONMENT_UNAVAILABLE",
        "executor_os":std::env::consts::OS,
        "native_windows_share_flags_permissions_and_av_contention_tested":false,
        "native_macos_file_handle_behavior_tested":false,
        "existing_t10_a_gate":"P1A-T10 remains ENVIRONMENT_UNAVAILABLE on this Linux executor; see T10-A-HOSTED-NATIVE-RESEARCH.md and preserved parent matrix evidence."
    });
    emit(
        "case_result",
        "B11-NATIVE-SHARE-PERMISSION-CONTENTION",
        "ENVIRONMENT_UNAVAILABLE",
        b11_details.clone(),
    );
    results.push(json!({"case_id":"B11","status":"ENVIRONMENT_UNAVAILABLE","details":b11_details}));

    add_matrix_result(
        "B12",
        run_b12_io_measurement(vfs, directory),
        None,
        &mut results,
    );

    let any_failure = results.iter().any(|result| result["status"] == "FAIL");
    let unavailable = results
        .iter()
        .filter(|result| result["status"] == "ENVIRONMENT_UNAVAILABLE")
        .map(|result| result["case_id"].clone())
        .collect::<Vec<_>>();
    emit(
        "matrix_summary",
        "T10-B1-B01-B12",
        if any_failure {
            "FAIL"
        } else {
            "READY_FOR_OWNER_REVIEW"
        },
        json!({
            "case_results":results,
            "all_executable_cases_passed":!any_failure,
            "environment_unavailable_cases":unavailable,
            "native_and_real_enospc_gate_state":if unavailable.is_empty() { "NONE" } else { "BLOCKED_ENVIRONMENT" },
            "power_loss_proven":false,
            "real_media_enospc_proven":false,
            "model_test_scope":"isolated synthetic fixtures and SQLite VFS return-code injection"
        }),
    );
    let _ = (b07, b09);
    Ok(results)
}

fn run_f0_scenarios(vfs: &TestVfs) -> Result<(), Box<dyn std::error::Error>> {
    let directory = temp_directory()?;
    let nofault_path = directory.join("f0-no-fault.sqlite");
    create_seed(&nofault_path, vfs)?;
    let nofault_integrity = verify_db(&nofault_path)?;
    let nofault_events = vfs.events()?;
    let seen_open = nofault_events
        .iter()
        .any(|event| event.operation == "xOpen");
    let seen_write = nofault_events
        .iter()
        .any(|event| event.operation == "xWrite");
    let seen_sync = nofault_events
        .iter()
        .any(|event| event.operation == "xSync");
    let nofault_ok = nofault_integrity["integrity_check"] == "ok"
        && nofault_integrity["fts5_integrity_check"] == "PASS"
        && seen_open
        && seen_write
        && seen_sync;
    emit(
        "case_result",
        "B1-F0-NO-FAULT",
        if nofault_ok { "PASS" } else { "FAIL" },
        json!({
            "seed":F0_SEED,
            "vfs":vfs_summary(vfs),
            "actual_callback_counts":{
                "xOpen":seen_open,
                "xWrite":seen_write,
                "xSync":seen_sync
            },
            "fresh_default_vfs_integrity":nofault_integrity,
            "ok":nofault_ok
        }),
    );
    if !nofault_ok {
        return Err(
            "no-fault VFS round-trip did not preserve FTS5 or reach xOpen/xWrite/xSync".into(),
        );
    }
    run_pool_smoke(vfs, &nofault_path)?;
    run_write_or_sync_fault(
        vfs,
        &directory,
        "B1-F0-SQLITE_FULL",
        IoOp::Write,
        sqlite::SQLITE_FULL,
        None,
        1,
    )?;
    run_write_or_sync_fault(
        vfs,
        &directory,
        "B1-F0-IOERR_FSYNC",
        IoOp::Sync,
        sqlite::SQLITE_IOERR_FSYNC,
        None,
        1,
    )?;
    run_short_read(vfs, &directory, "B1-F0-SHORT_READ", "f0-short-read.sqlite")?;
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

pub fn run_f0() -> i32 {
    let mut vfs = match TestVfs::register() {
        Ok(vfs) => vfs,
        Err(error) => {
            emit(
                "run_result",
                "B1-F0",
                "FAIL",
                json!({
                    "error":error,
                    "failure_code":"T10_B1_BLOCKED_VFS_CONTRACT"
                }),
            );
            return 1;
        }
    };
    emit(
        "run_start",
        "B1-F0",
        "STARTED",
        json!({
            "seed":F0_SEED,
            "vfs":vfs_summary(&vfs),
            "claim_classification":"MODEL_ONLY",
            "real_enospc_attempted":false,
            "power_loss_attempted":false
        }),
    );
    let result = run_f0_scenarios(&vfs);
    match vfs.events() {
        Ok(events) => emit_events("B1-F0", events),
        Err(error) => emit("vfs_log_error", "B1-F0", "FAIL", json!({"error":error})),
    }
    let open_handles = vfs.open_handle_count();
    let unregister = if open_handles == 0 {
        vfs.unregister()
    } else {
        Err(format!(
            "VFS retained {open_handles} open SQLite file handles"
        ))
    };
    let result = result.map_err(|error| error.to_string());
    let final_result = match (result, unregister) {
        (Ok(()), Ok(())) => {
            emit(
                "run_result",
                "B1-F0",
                "PASS",
                json!({
                    "failure_code":null,
                    "sqlite_faults":[
                        {"operation":"xWrite","result_code":sqlite::SQLITE_FULL},
                        {"operation":"xSync","result_code":sqlite::SQLITE_IOERR_FSYNC},
                        {"operation":"xRead","result_code":sqlite::SQLITE_IOERR_SHORT_READ}
                    ],
                    "all_connections_and_handles_closed_before_unregister":true,
                    "fts5_integrity_checked_after_each_fault":true,
                    "model_only":true,
                    "real_enospc_tested":false,
                    "physical_power_loss_tested":false
                }),
            );
            0
        }
        (Err(error), _) => {
            emit(
                "run_result",
                "B1-F0",
                "FAIL",
                json!({
                    "error":error,
                    "failure_code":"T10_B1_BLOCKED_VFS_CONTRACT",
                    "open_handle_count_before_unregister":open_handles
                }),
            );
            1
        }
        (_, Err(error)) => {
            emit(
                "run_result",
                "B1-F0",
                "FAIL",
                json!({
                    "error":error,
                    "failure_code":"T10_B1_BLOCKED_VFS_CONTRACT",
                    "open_handle_count_before_unregister":open_handles
                }),
            );
            1
        }
    };
    final_result
}

pub fn run_matrix() -> i32 {
    let mut vfs = match TestVfs::register() {
        Ok(vfs) => vfs,
        Err(error) => {
            emit(
                "run_result",
                "T10-B1-B01-B12",
                "BLOCKED",
                json!({"error":error,"failure_code":"T10_B1_BLOCKED_VFS_CONTRACT"}),
            );
            return 1;
        }
    };
    emit(
        "run_start",
        "T10-B1-B01-B12",
        "STARTED",
        json!({
            "frozen_start_head":"b9c44a0a05fc8dc34744563b43d88eeab7509b35",
            "seed":F0_SEED,
            "vfs":vfs_summary(&vfs),
            "claim_classification":"MODEL_ONLY except explicitly identified Linux OS observations",
            "real_enospc_attempted":false,
            "physical_power_loss_attempted":false
        }),
    );
    let f0 = run_f0_scenarios(&vfs);
    if let Err(error) = f0 {
        let events = vfs.events().unwrap_or_default();
        emit_events("T10-B1-F0-GATE", events);
        let open_handles = vfs.open_handle_count();
        let unregister = if open_handles == 0 {
            vfs.unregister()
        } else {
            Err(format!("VFS retained {open_handles} handles"))
        };
        emit(
            "run_result",
            "T10-B1-B01-B12",
            "BLOCKED",
            json!({"failure_code":"T10_B1_BLOCKED_VFS_CONTRACT","f0_error":error.to_string(),"unregister_result":unregister.err()}),
        );
        return 1;
    }
    emit(
        "gate_result",
        "B1-F0",
        "PASS",
        json!({"required_before_matrix":true,"evidence_class":"ACTUAL_SQLITE_VFS_CALLBACKS","open_sqlite_file_handles_before_matrix":vfs.open_handle_count(),"all_f0_connections_closed":vfs.open_handle_count() == 0}),
    );
    let directory = match temp_directory() {
        Ok(directory) => directory,
        Err(error) => {
            emit(
                "run_result",
                "T10-B1-B01-B12",
                "FAIL",
                json!({"error":error.to_string()}),
            );
            return 1;
        }
    };
    let matrix_result = run_matrix_scenarios(&vfs, &directory);
    let events = vfs.events().unwrap_or_default();
    emit_events("T10-B1-B01-B12", events);
    let open_handles = vfs.open_handle_count();
    let unregister = if open_handles == 0 {
        vfs.unregister()
    } else {
        Err(format!(
            "VFS retained {open_handles} open SQLite file handles"
        ))
    };
    let _ = std::fs::remove_dir_all(&directory);
    match (matrix_result, unregister) {
        (Ok(results), Ok(())) => {
            let failed = results.iter().any(|r| r["status"] == "FAIL");
            emit(
                "run_result",
                "T10-B1-B01-B12",
                if failed {
                    "FAIL"
                } else {
                    "T10_B1_FAULT_MODEL_READY_FOR_OWNER_REVIEW"
                },
                json!({
                    "failure_code":null,
                    "executable_cases_failed":failed,
                    "all_connections_and_handles_closed_before_unregister":true,
                    "vfs_unregistered":true,
                    "real_enospc_tested":false,
                    "physical_power_loss_tested":false,
                    "overall_native_gate_state":"BLOCKED_ENVIRONMENT",
                    "owner_review_ready":!failed
                }),
            );
            i32::from(failed)
        }
        (Err(error), unregister) => {
            emit(
                "run_result",
                "T10-B1-B01-B12",
                "FAIL",
                json!({"error":error.to_string(),"open_handles_before_unregister":open_handles,"unregister_error":unregister.err()}),
            );
            1
        }
        (Ok(_), Err(error)) => {
            emit(
                "run_result",
                "T10-B1-B01-B12",
                "FAIL",
                json!({"error":error,"open_handles_before_unregister":open_handles,"all_connections_and_handles_closed_before_unregister":false}),
            );
            1
        }
    }
}
