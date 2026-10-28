//! Measurement helpers for the #494 experiment.
//!
//! Every number this module produces is a wall-clock observation of one call on
//! this machine. Nothing is summed across stages and presented as an end-to-end
//! percentile: each row states exactly which call it timed.

use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub min: f64,
    pub max: f64,
    pub samples: usize,
}

impl Percentiles {
    pub(super) fn triple(self) -> [f64; 3] {
        [self.p50, self.p95, self.p99]
    }
}

pub(super) fn percentiles(mut values: Vec<f64>) -> Percentiles {
    if values.is_empty() {
        return Percentiles::default();
    }
    values.sort_by(f64::total_cmp);
    let pick = |q: f64| -> f64 {
        let index = ((values.len() - 1) as f64 * q).ceil() as usize;
        values[index.min(values.len() - 1)]
    };
    Percentiles {
        p50: pick(0.50),
        p95: pick(0.95),
        p99: pick(0.99),
        min: values[0],
        max: values[values.len() - 1],
        samples: values.len(),
    }
}

/// Run `operation` `warmup` times, then time `samples` runs.
pub(super) fn timed(mut operation: impl FnMut(), warmup: usize, samples: usize) -> Percentiles {
    for _ in 0..warmup {
        operation();
    }
    let mut elapsed = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        operation();
        elapsed.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    percentiles(elapsed)
}

/// Time a single call once.
pub(super) fn once(operation: impl FnOnce()) -> f64 {
    let started = Instant::now();
    operation();
    started.elapsed().as_secs_f64() * 1_000.0
}

/// Sampled process resources. Peak RSS is a sampled maximum, not an OS peak.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub(super) struct ResourceSample {
    pub rss_start_bytes: u64,
    pub rss_peak_bytes: u64,
    pub rss_end_bytes: u64,
    pub cpu_peak_percent: f32,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub samples: u64,
    pub poll_interval_ms: u64,
}

pub(super) struct ResourceSampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<ResourceSample>>,
    start: sysinfo::System,
    pid: sysinfo::Pid,
}

fn process_of(system: &mut sysinfo::System, pid: sysinfo::Pid) -> Option<&sysinfo::Process> {
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid)
}

impl ResourceSampler {
    pub(super) fn start() -> Self {
        let pid = sysinfo::get_current_pid().expect("current pid");
        let mut system = sysinfo::System::new();
        if let Some(process) = process_of(&mut system, pid) {
            process.disk_usage();
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut system = sysinfo::System::new();
            let mut sample = ResourceSample {
                poll_interval_ms: 20,
                ..ResourceSample::default()
            };
            let mut last_read = 0u64;
            let mut last_written = 0u64;
            if let Some(process) = process_of(&mut system, pid) {
                sample.rss_start_bytes = process.memory();
                sample.rss_peak_bytes = sample.rss_start_bytes;
                let usage = process.disk_usage();
                last_read = usage.read_bytes;
                last_written = usage.written_bytes;
            }
            while !thread_stop.load(Ordering::Relaxed) {
                if let Some(process) = process_of(&mut system, pid) {
                    let rss = process.memory();
                    sample.rss_peak_bytes = sample.rss_peak_bytes.max(rss);
                    sample.rss_end_bytes = rss;
                    sample.cpu_peak_percent = sample.cpu_peak_percent.max(process.cpu_usage());
                    let usage = process.disk_usage();
                    sample.read_bytes = usage.read_bytes.saturating_sub(last_read);
                    sample.written_bytes = usage.written_bytes.saturating_sub(last_written);
                    sample.samples += 1;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            sample
        });
        Self {
            stop,
            handle: Some(handle),
            start: system,
            pid,
        }
    }

    pub(super) fn finish(mut self) -> ResourceSample {
        self.stop.store(true, Ordering::Relaxed);
        let mut sample = self
            .handle
            .take()
            .expect("sampler thread")
            .join()
            .unwrap_or_default();
        if sample.rss_end_bytes == 0 {
            if let Some(process) = process_of(&mut self.start, self.pid) {
                sample.rss_end_bytes = process.memory();
                sample.rss_peak_bytes = sample.rss_peak_bytes.max(sample.rss_end_bytes);
            }
        }
        sample
    }
}

pub(super) fn emit(record: serde_json::Value) {
    eprintln!("{record}");
}

pub(super) fn record(kind: &str, fields: serde_json::Value) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("experiment".to_string(), json!("issue494_filename_ab"));
    object.insert("record".to_string(), json!(kind));
    object.insert("os".to_string(), json!(std::env::consts::OS));
    object.insert(
        "build".to_string(),
        json!(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }),
    );
    if let serde_json::Value::Object(fields) = fields {
        for (key, value) in fields {
            object.insert(key, value);
        }
    }
    serde_json::Value::Object(object)
}
