use serde::Serialize;
#[cfg(target_os = "windows")]
use std::time::Instant;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn malloc_zone_pressure_relief(
        zone: *mut libc::malloc_zone_t,
        goal: libc::size_t,
    ) -> libc::size_t;
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub(super) struct ProcessResources {
    pub(super) rss_bytes: Option<u64>,
    /// Windows process-private committed bytes from
    /// `PROCESS_MEMORY_COUNTERS_EX::PrivateUsage`. Unlike working-set RSS,
    /// this remains accounted when the settled sampler trims resident pages.
    pub(super) private_committed_bytes: Option<u64>,
    pub(super) handle_count: Option<u64>,
    pub(super) fd_count: Option<u64>,
}

/// Test-only diagnostic of currently busy allocations in the Windows process
/// heaps. This complements PrivateUsage: it does not participate in the hard
/// resource classifier and does not identify which subsystem owns a block.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub(super) struct ProcessHeapResources {
    pub(super) available: bool,
    pub(super) heap_count: u32,
    pub(super) heaps_walked: u32,
    pub(super) heap_region_count: u64,
    /// Bytes committed within regions in the process heaps. This does
    /// not include private committed memory outside these heaps.
    pub(super) heap_region_committed_bytes: u64,
    pub(super) heap_region_uncommitted_bytes: u64,
    pub(super) busy_allocation_count: u64,
    pub(super) busy_allocation_bytes: u64,
    pub(super) elapsed_us: u64,
    pub(super) error_code: Option<u32>,
}

#[cfg(target_os = "windows")]
pub(super) fn process_heap_snapshot() -> Option<ProcessHeapResources> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_NO_MORE_ITEMS},
        System::Memory::{GetProcessHeaps, HeapLock, HeapUnlock, HeapWalk, PROCESS_HEAP_ENTRY},
    };

    const MAX_PROCESS_HEAPS: usize = 64;
    const PROCESS_HEAP_REGION: u16 = 0x0001;
    const PROCESS_HEAP_ENTRY_BUSY: u16 = 0x0004;
    let started = Instant::now();
    let mut heaps = [std::ptr::null_mut(); MAX_PROCESS_HEAPS];
    let heap_count = unsafe { GetProcessHeaps(heaps.len() as u32, heaps.as_mut_ptr()) };
    let mut sample = ProcessHeapResources {
        heap_count,
        ..ProcessHeapResources::default()
    };
    if heap_count == 0 {
        sample.error_code = Some(unsafe { GetLastError() });
        sample.elapsed_us = started.elapsed().as_micros() as u64;
        return Some(sample);
    }
    if heap_count as usize > heaps.len() {
        sample.elapsed_us = started.elapsed().as_micros() as u64;
        return Some(sample);
    }

    sample.available = true;
    for heap in heaps.iter().take(heap_count as usize).copied() {
        if unsafe { HeapLock(heap) } == 0 {
            sample.available = false;
            sample
                .error_code
                .get_or_insert_with(|| unsafe { GetLastError() });
            continue;
        }

        let mut entry = unsafe { std::mem::zeroed::<PROCESS_HEAP_ENTRY>() };
        let mut heap_walk_completed = false;
        while unsafe { HeapWalk(heap, &mut entry) } != 0 {
            if entry.wFlags & PROCESS_HEAP_REGION != 0 {
                let region = unsafe { entry.Anonymous.Region };
                sample.heap_region_count += 1;
                sample.heap_region_committed_bytes += u64::from(region.dwCommittedSize);
                sample.heap_region_uncommitted_bytes += u64::from(region.dwUnCommittedSize);
            }
            if entry.wFlags & PROCESS_HEAP_ENTRY_BUSY != 0 {
                sample.busy_allocation_count += 1;
                sample.busy_allocation_bytes += u64::from(entry.cbData);
            }
        }
        let walk_error = unsafe { GetLastError() };
        if walk_error == ERROR_NO_MORE_ITEMS {
            sample.heaps_walked += 1;
            heap_walk_completed = true;
        } else {
            sample.available = false;
            sample.error_code.get_or_insert(walk_error);
        }

        if unsafe { HeapUnlock(heap) } == 0 {
            sample.available = false;
            sample
                .error_code
                .get_or_insert_with(|| unsafe { GetLastError() });
        }
        if !heap_walk_completed {
            sample.available = false;
        }
    }

    sample.available &= sample.heaps_walked == sample.heap_count;
    sample.elapsed_us = started.elapsed().as_micros() as u64;
    Some(sample)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn process_heap_snapshot() -> Option<ProcessHeapResources> {
    None
}

impl ProcessResources {
    pub(super) fn max(self, other: Self) -> Self {
        Self {
            rss_bytes: max_optional(self.rss_bytes, other.rss_bytes),
            private_committed_bytes: max_optional(
                self.private_committed_bytes,
                other.private_committed_bytes,
            ),
            handle_count: max_optional(self.handle_count, other.handle_count),
            fd_count: max_optional(self.fd_count, other.fd_count),
        }
    }
}

pub(super) fn snapshot() -> ProcessResources {
    let (rss_bytes, private_committed_bytes) = current_process_memory();
    ProcessResources {
        rss_bytes,
        private_committed_bytes,
        handle_count: current_handle_count(),
        fd_count: current_fd_count(),
    }
}

pub(super) fn settle_allocator() {
    #[cfg(target_os = "macos")]
    unsafe {
        // A native test process can have more than the default malloc zone
        // (for example through SQLite/Tauri dependencies). Ask macOS to
        // scavenge every zone so the settled RSS sample is not only a
        // snapshot of a non-default allocator cache. This is test-only
        // pressure relief; live allocations remain valid.
        let _ = malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};

        // Remove pages retained only by the process working set before the
        // settled RSS sample. Live allocations remain valid and will fault
        // back in if the workload needs them again.
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

fn max_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(target_os = "macos")]
fn current_rss_bytes() -> Option<u64> {
    // Avoid allocating a new sysinfo process table for every settled and
    // in-workload sample. The sampler must not manufacture allocator
    // retention that the repeated workload then reports as a leak.
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let result = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr() as *mut libc::c_void,
            std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int,
        )
    };
    (result == std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int)
        .then(|| unsafe { info.assume_init().pti_resident_size })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn current_rss_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}

#[cfg(target_os = "windows")]
fn current_process_memory() -> (Option<u64>, Option<u64>) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let result = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if result == 0 {
        return (None, None);
    }
    // WorkingSetSize is retained as an observed post-trim diagnostic only;
    // PrivateUsage is the hard memory signal for Windows leak detection.
    (
        Some(counters.WorkingSetSize as u64),
        Some(counters.PrivateUsage as u64),
    )
}

#[cfg(target_os = "macos")]
fn current_process_memory() -> (Option<u64>, Option<u64>) {
    (current_rss_bytes(), None)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn current_process_memory() -> (Option<u64>, Option<u64>) {
    (current_rss_bytes(), None)
}

#[cfg(target_os = "windows")]
fn current_handle_count() -> Option<u64> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};

    let mut count = 0u32;
    let result = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    (result != 0).then_some(u64::from(count))
}

#[cfg(target_os = "macos")]
fn current_handle_count() -> Option<u64> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn current_handle_count() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn current_fd_count() -> Option<u64> {
    // Count descriptors without opening `/dev/fd` for every sample. The
    // fixed stack buffer keeps the sampler from adding allocator retention to
    // the RSS trend; a full buffer is reported as unavailable rather than a
    // silently truncated count.
    let mut buffer = [0u8; 16 * 1024];
    let result = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDLISTFDS,
            0,
            buffer.as_mut_ptr() as *mut libc::c_void,
            buffer.len() as libc::c_int,
        )
    };
    let bytes = usize::try_from(result).ok()?;
    (bytes < buffer.len() && bytes % std::mem::size_of::<libc::proc_fdinfo>() == 0)
        .then_some((bytes / std::mem::size_of::<libc::proc_fdinfo>()) as u64)
}

#[cfg(not(target_os = "macos"))]
fn current_fd_count() -> Option<u64> {
    None
}
