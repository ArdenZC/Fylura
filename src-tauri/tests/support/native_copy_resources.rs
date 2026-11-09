use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct NativeProcessSample {
    pub cpu_time_ns: u64,
    pub rss_bytes: u64,
    pub private_bytes: Option<u64>,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub pageins: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NativeProcessStats {
    pub cpu_time_ns: u64,
    pub start_rss_bytes: u64,
    pub end_rss_bytes: u64,
    pub peak_rss_bytes: u64,
    pub start_private_bytes: Option<u64>,
    pub end_private_bytes: Option<u64>,
    pub peak_private_bytes: Option<u64>,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub pageins: Option<u64>,
}

pub fn format_native_process_stats(stats: Option<NativeProcessStats>) -> String {
    match stats {
        Some(stats) => format!(
            "cpuTimeNs={} startRssBytes={} endRssBytes={} peakRssBytes={} startPrivateBytes={:?} endPrivateBytes={:?} peakPrivateBytes={:?} readBytes={} writtenBytes={} pageins={:?}",
            stats.cpu_time_ns,
            stats.start_rss_bytes,
            stats.end_rss_bytes,
            stats.peak_rss_bytes,
            stats.start_private_bytes,
            stats.end_private_bytes,
            stats.peak_private_bytes,
            stats.read_bytes,
            stats.written_bytes,
            stats.pageins,
        ),
        None => "unavailable".to_string(),
    }
}

#[derive(Default)]
struct Samples {
    peak_rss_bytes: u64,
    peak_private_bytes: Option<u64>,
    latest: Option<NativeProcessSample>,
}

pub struct NativeProcessMonitor {
    baseline: NativeProcessSample,
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Samples>>,
    worker: Option<JoinHandle<()>>,
}

impl NativeProcessMonitor {
    pub fn start() -> Option<Self> {
        let baseline = sample_native_process()?;
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(Samples {
            peak_rss_bytes: baseline.rss_bytes,
            peak_private_bytes: baseline.private_bytes,
            latest: Some(baseline),
        }));
        let worker_stop = Arc::clone(&stop);
        let worker_samples = Arc::clone(&samples);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                if let Some(sample) = sample_native_process() {
                    record_sample(&worker_samples, sample);
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Some(Self {
            baseline,
            stop,
            samples,
            worker: Some(worker),
        })
    }

    pub fn finish(mut self) -> NativeProcessStats {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(sample) = sample_native_process() {
            record_sample(&self.samples, sample);
        }
        let samples = self.samples.lock().expect("native process samples");
        let end = samples.latest.unwrap_or(self.baseline);
        NativeProcessStats {
            cpu_time_ns: end.cpu_time_ns.saturating_sub(self.baseline.cpu_time_ns),
            start_rss_bytes: self.baseline.rss_bytes,
            end_rss_bytes: end.rss_bytes,
            peak_rss_bytes: samples.peak_rss_bytes,
            start_private_bytes: self.baseline.private_bytes,
            end_private_bytes: end.private_bytes,
            peak_private_bytes: max_optional(samples.peak_private_bytes, end.private_bytes),
            read_bytes: end.read_bytes.saturating_sub(self.baseline.read_bytes),
            written_bytes: end
                .written_bytes
                .saturating_sub(self.baseline.written_bytes),
            pageins: end
                .pageins
                .zip(self.baseline.pageins)
                .map(|(end, start)| end.saturating_sub(start)),
        }
    }
}

impl Drop for NativeProcessMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn record_sample(samples: &Mutex<Samples>, sample: NativeProcessSample) {
    if let Ok(mut samples) = samples.lock() {
        samples.peak_rss_bytes = samples.peak_rss_bytes.max(sample.rss_bytes);
        samples.peak_private_bytes = max_optional(samples.peak_private_bytes, sample.private_bytes);
        samples.latest = Some(sample);
    }
}

fn max_optional(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(windows)]
fn sample_native_process() -> Option<NativeProcessSample> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{
                GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
            },
            Threading::{GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, IO_COUNTERS},
        },
    };

    let process = unsafe { GetCurrentProcess() };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let memory_ok = unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    let mut io = IO_COUNTERS::default();
    let io_ok = unsafe { GetProcessIoCounters(process, &mut io) };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let cpu_ok =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    if memory_ok == 0 || io_ok == 0 || cpu_ok == 0 {
        return None;
    }
    let filetime =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    Some(NativeProcessSample {
        cpu_time_ns: filetime(kernel)
            .saturating_add(filetime(user))
            .saturating_mul(100),
        rss_bytes: counters.WorkingSetSize as u64,
        private_bytes: Some(counters.PrivateUsage as u64),
        read_bytes: io.ReadTransferCount,
        written_bytes: io.WriteTransferCount,
        pageins: None,
    })
}

#[cfg(target_os = "macos")]
fn sample_native_process() -> Option<NativeProcessSample> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let result = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast(),
        )
    };
    if result != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(NativeProcessSample {
        cpu_time_ns: usage.ri_user_time.saturating_add(usage.ri_system_time),
        rss_bytes: usage.ri_resident_size,
        private_bytes: Some(usage.ri_phys_footprint),
        read_bytes: usage.ri_diskio_bytesread,
        written_bytes: usage.ri_diskio_byteswritten,
        pageins: Some(usage.ri_pageins),
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn sample_native_process() -> Option<NativeProcessSample> {
    None
}
