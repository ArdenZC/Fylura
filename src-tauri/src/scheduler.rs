//! Non-durable resource coordination for W1 Foundation work.
//!
//! `WorkScheduler` only decides whether a bounded unit of work may hold a
//! resource lease. Durable jobs, cancellation state, retries, recovery and
//! filesystem truth remain owned by their existing authorities.

use crate::{file_workspace::WorkClass, resource_governor::RuntimeResourceGovernor};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use thiserror::Error;

const DEFAULT_MAX_QUEUED: usize = 128;
const DEFAULT_BACKGROUND_FAIRNESS_AFTER: u32 = 4;
const DEFAULT_OPEN_HANDLES: u32 = 64;
const DEFAULT_DECODER_SLOTS: u32 = 2;
const DEFAULT_NATIVE_PREVIEW_SLOTS: u32 = 1;
const DEFAULT_PROVIDER_NETWORK_SLOTS: u32 = 4;
const EXTERNAL_CANCELLATION_RECHECK: Duration = Duration::from_secs(1);
const POLICY_RECHECK: Duration = Duration::from_secs(5);

/// The bounded resource dimensions understood by the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceClass {
    Cpu,
    Io,
    OpenHandles,
    Decoder,
    NativePreview,
    ProviderNetwork,
}

/// A per-request bounded resource declaration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceHints {
    pub cpu: u32,
    pub io: u32,
    pub open_handles: u32,
    pub decoder: u32,
    pub native_preview: u32,
    pub provider_network: u32,
}

impl ResourceHints {
    pub const fn cpu(cpu: u32) -> Self {
        Self {
            cpu,
            ..Self::empty()
        }
    }

    pub const fn cpu_io(cpu: u32, io: u32) -> Self {
        Self {
            cpu,
            io,
            ..Self::empty()
        }
    }

    pub const fn empty() -> Self {
        Self {
            cpu: 0,
            io: 0,
            open_handles: 0,
            decoder: 0,
            native_preview: 0,
            provider_network: 0,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.cpu == 0
            && self.io == 0
            && self.open_handles == 0
            && self.decoder == 0
            && self.native_preview == 0
            && self.provider_network == 0
    }

    fn fits_in(self, available: ResourceHints) -> bool {
        self.cpu <= available.cpu
            && self.io <= available.io
            && self.open_handles <= available.open_handles
            && self.decoder <= available.decoder
            && self.native_preview <= available.native_preview
            && self.provider_network <= available.provider_network
    }

    fn saturating_add(self, other: Self) -> Self {
        Self {
            cpu: self.cpu.saturating_add(other.cpu),
            io: self.io.saturating_add(other.io),
            open_handles: self.open_handles.saturating_add(other.open_handles),
            decoder: self.decoder.saturating_add(other.decoder),
            native_preview: self.native_preview.saturating_add(other.native_preview),
            provider_network: self.provider_network.saturating_add(other.provider_network),
        }
    }

    fn saturating_sub(self, other: Self) -> Self {
        Self {
            cpu: self.cpu.saturating_sub(other.cpu),
            io: self.io.saturating_sub(other.io),
            open_handles: self.open_handles.saturating_sub(other.open_handles),
            decoder: self.decoder.saturating_sub(other.decoder),
            native_preview: self.native_preview.saturating_sub(other.native_preview),
            provider_network: self.provider_network.saturating_sub(other.provider_network),
        }
    }
}

/// The process-local resource budget. Every dimension is at least one after
/// scheduler configuration normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceCapacities {
    pub cpu: u32,
    pub io: u32,
    pub open_handles: u32,
    pub decoder: u32,
    pub native_preview: u32,
    pub provider_network: u32,
}

impl ResourceCapacities {
    pub const fn new(
        cpu: u32,
        io: u32,
        open_handles: u32,
        decoder: u32,
        native_preview: u32,
        provider_network: u32,
    ) -> Self {
        Self {
            cpu,
            io,
            open_handles,
            decoder,
            native_preview,
            provider_network,
        }
    }

    fn normalized(self) -> Self {
        Self {
            cpu: self.cpu.max(1),
            io: self.io.max(1),
            open_handles: self.open_handles.max(1),
            decoder: self.decoder.max(1),
            native_preview: self.native_preview.max(1),
            provider_network: self.provider_network.max(1),
        }
    }

    fn as_hints(self) -> ResourceHints {
        ResourceHints {
            cpu: self.cpu,
            io: self.io,
            open_handles: self.open_handles,
            decoder: self.decoder,
            native_preview: self.native_preview,
            provider_network: self.provider_network,
        }
    }
}

impl Default for ResourceCapacities {
    fn default() -> Self {
        let cpu = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get() as u32)
            .unwrap_or(1)
            .max(1);
        Self {
            cpu,
            io: cpu.clamp(1, 8),
            open_handles: DEFAULT_OPEN_HANDLES,
            decoder: DEFAULT_DECODER_SLOTS,
            native_preview: DEFAULT_NATIVE_PREVIEW_SLOTS,
            provider_network: DEFAULT_PROVIDER_NETWORK_SLOTS,
        }
    }
}

/// A cancellation signal owned by the caller of a disposable unit of work.
///
/// `from_flag` lets an existing authority keep its own cancellation token. The
/// scheduler observes that token but never changes the authority's lifecycle.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

struct CancellationState {
    cancelled: AtomicBool,
    external_flags: Vec<Arc<AtomicBool>>,
    waiter_signal: Mutex<Option<Weak<SchedulerInner>>>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                external_flags: Vec::new(),
                waiter_signal: Mutex::new(None),
            }),
        }
    }

    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self::from_flags(vec![flag])
    }

    pub(crate) fn from_flags(flags: Vec<Arc<AtomicBool>>) -> Self {
        Self {
            inner: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                external_flags: flags,
                waiter_signal: Mutex::new(None),
            }),
        }
    }

    pub fn cancel(&self) {
        self.cancel_without_notification();
        let scheduler = self
            .inner
            .waiter_signal
            .lock()
            .ok()
            .and_then(|signal| signal.as_ref().and_then(Weak::upgrade));
        if let Some(scheduler) = scheduler {
            // Pair the cancellation predicate with the same mutex used by the
            // condition-variable waiter. Without this lock, cancellation can
            // notify after the waiter's last predicate check but before it
            // actually sleeps, losing the wake and hanging shutdown forever.
            let _state = scheduler
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            scheduler.changed.notify_all();
        }
    }

    fn cancel_without_notification(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
            || self
                .inner
                .external_flags
                .iter()
                .any(|flag| flag.load(Ordering::Acquire))
    }

    fn attach_waiter_signal(&self, scheduler: &Arc<SchedulerInner>) {
        if let Ok(mut waiter_signal) = self.inner.waiter_signal.lock() {
            *waiter_signal = Some(Arc::downgrade(scheduler));
        }
        if self.is_cancelled() {
            scheduler.changed.notify_all();
        }
    }

    fn has_unobservable_external_cancel(&self) -> bool {
        !self.inner.external_flags.is_empty()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// A request submitted to the scheduler. The request ID is diagnostic and is
/// never a durable job ID.
#[derive(Debug, Clone)]
pub struct WorkRequest {
    pub request_id: String,
    pub session_id: Option<String>,
    pub coalesce_key: Option<String>,
    pub class: WorkClass,
    pub resources: ResourceHints,
    pub cancellation: CancellationToken,
}

impl WorkRequest {
    pub fn new(request_id: impl Into<String>, class: WorkClass, resources: ResourceHints) -> Self {
        Self {
            request_id: request_id.into(),
            session_id: None,
            coalesce_key: None,
            class,
            resources,
            cancellation: CancellationToken::new(),
        }
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_coalesce_key(mut self, coalesce_key: impl Into<String>) -> Self {
        self.coalesce_key = Some(coalesce_key.into());
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

/// The current platform pressure decision consumed by the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePolicyDecision {
    pub effective_capacity: ResourceCapacities,
    pub allow_background: bool,
    pub efficiency_qos: bool,
}

/// Platform adapters provide pressure and essential-work policy without
/// reimplementing platform activity/thermal authorities in the scheduler.
pub trait PlatformResourcePolicy: Send + Sync {
    fn decision(
        &self,
        class: WorkClass,
        configured_capacity: ResourceCapacities,
    ) -> ResourcePolicyDecision;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PermissiveResourcePolicy;

impl PlatformResourcePolicy for PermissiveResourcePolicy {
    fn decision(
        &self,
        class: WorkClass,
        configured_capacity: ResourceCapacities,
    ) -> ResourcePolicyDecision {
        ResourcePolicyDecision {
            effective_capacity: configured_capacity,
            allow_background: true,
            efficiency_qos: matches!(class, WorkClass::Background),
        }
    }
}

/// Conservative policy used where no product-specific native resource policy
/// exists yet. It remains non-blocking for background work and only caps CPU.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConservativeResourcePolicy;

impl PlatformResourcePolicy for ConservativeResourcePolicy {
    fn decision(
        &self,
        class: WorkClass,
        configured_capacity: ResourceCapacities,
    ) -> ResourcePolicyDecision {
        ResourcePolicyDecision {
            effective_capacity: ResourceCapacities {
                cpu: configured_capacity.cpu.clamp(1, 2),
                ..configured_capacity
            },
            allow_background: true,
            efficiency_qos: matches!(class, WorkClass::Background),
        }
    }
}

fn default_platform_policy() -> Arc<dyn PlatformResourcePolicy> {
    Arc::new(RuntimeResourceGovernor::current())
}

/// Scheduler configuration. All limits are process-local and non-durable.
#[derive(Clone)]
pub struct SchedulerConfig {
    pub capacities: ResourceCapacities,
    pub max_queued: usize,
    pub background_fairness_after: u32,
    pub policy: Arc<dyn PlatformResourcePolicy>,
}

impl fmt::Debug for SchedulerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SchedulerConfig")
            .field("capacities", &self.capacities)
            .field("max_queued", &self.max_queued)
            .field("background_fairness_after", &self.background_fairness_after)
            .finish_non_exhaustive()
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            capacities: ResourceCapacities::default(),
            max_queued: DEFAULT_MAX_QUEUED,
            background_fairness_after: DEFAULT_BACKGROUND_FAIRNESS_AFTER,
            policy: default_platform_policy(),
        }
    }
}

impl SchedulerConfig {
    pub fn with_capacities(mut self, capacities: ResourceCapacities) -> Self {
        self.capacities = capacities;
        self
    }

    pub fn with_max_queued(mut self, max_queued: usize) -> Self {
        self.max_queued = max_queued;
        self
    }

    pub fn with_background_fairness_after(mut self, grants: u32) -> Self {
        self.background_fairness_after = grants;
        self
    }

    pub fn with_policy(mut self, policy: Arc<dyn PlatformResourcePolicy>) -> Self {
        self.policy = policy;
        self
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AcquireError {
    #[error("scheduler request is invalid: {0}")]
    InvalidRequest(String),
    #[error("scheduler queue is full")]
    QueueFull,
    #[error("scheduler request was cancelled")]
    Cancelled,
    #[error("scheduler is unavailable")]
    Unavailable,
    #[error("scheduler request would block")]
    WouldBlock,
    #[error("platform policy does not admit this background request")]
    PolicyDenied,
}

/// A successfully granted, RAII resource lease.
pub struct ResourceLease {
    inner: Arc<ResourceLeaseInner>,
}

struct ResourceLeaseInner {
    scheduler: Weak<SchedulerInner>,
    lease_id: u64,
    request_id: String,
    class: WorkClass,
    resources: ResourceHints,
    cancellation: CancellationToken,
    released: AtomicBool,
}

impl fmt::Debug for ResourceLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceLease")
            .field("lease_id", &self.lease_id())
            .field("request_id", &self.request_id())
            .field("class", &self.class())
            .field("resources", &self.resources())
            .finish()
    }
}

impl ResourceLease {
    pub fn lease_id(&self) -> u64 {
        self.inner.lease_id
    }

    pub fn request_id(&self) -> &str {
        &self.inner.request_id
    }

    pub fn class(&self) -> WorkClass {
        self.inner.class
    }

    pub fn resources(&self) -> ResourceHints {
        self.inner.resources
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancellation.is_cancelled()
    }

    /// Explicit release is idempotent; `Drop` releases the same lease if the
    /// caller does not use this method.
    pub fn release(&self) {
        if self.inner.released.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(scheduler) = self.inner.scheduler.upgrade() {
            WorkScheduler { inner: scheduler }.release_lease(self.inner.lease_id);
        }
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        self.release();
    }
}

struct Waiter {
    lease: Mutex<Option<ResourceLease>>,
}

impl Waiter {
    fn new() -> Self {
        Self {
            lease: Mutex::new(None),
        }
    }

    fn set_lease(&self, lease: ResourceLease) {
        if let Ok(mut current) = self.lease.lock() {
            *current = Some(lease);
        }
    }

    fn take_lease(&self) -> Option<ResourceLease> {
        self.lease.lock().ok()?.take()
    }
}

struct QueuedRequest {
    sequence: u64,
    request: WorkRequest,
    waiter: Arc<Waiter>,
}

struct ActiveLease {
    request_id: String,
    session_id: Option<String>,
    coalesce_key: Option<String>,
    class: WorkClass,
    resources: ResourceHints,
    cancellation: CancellationToken,
}

#[derive(Default)]
struct SchedulerMetrics {
    total_grants: u64,
    total_releases: u64,
    total_cancellations: u64,
    total_rejections: u64,
    total_timed_wait_wakeups: u64,
}

struct SchedulerState {
    queue: VecDeque<QueuedRequest>,
    active: HashMap<u64, ActiveLease>,
    granted: ResourceHints,
    next_sequence: u64,
    high_priority_grants_since_background: u32,
    metrics: SchedulerMetrics,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            active: HashMap::new(),
            granted: ResourceHints::empty(),
            next_sequence: 1,
            high_priority_grants_since_background: 0,
            metrics: SchedulerMetrics::default(),
        }
    }
}

struct SchedulerInner {
    state: Mutex<SchedulerState>,
    changed: Arc<Condvar>,
    config: SchedulerConfig,
    next_lease_id: AtomicU64,
    resource_change_waker: Mutex<Option<SyncSender<()>>>,
    native_policy_notifications: AtomicBool,
}

/// A process-local, bounded resource scheduler.
#[derive(Clone)]
pub struct WorkScheduler {
    inner: Arc<SchedulerInner>,
}

impl fmt::Debug for WorkScheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkScheduler")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl WorkScheduler {
    pub fn new(mut config: SchedulerConfig) -> Self {
        config.capacities = config.capacities.normalized();
        config.max_queued = config.max_queued.max(1);
        config.background_fairness_after = config.background_fairness_after.max(1);
        Self {
            inner: Arc::new(SchedulerInner {
                state: Mutex::new(SchedulerState::default()),
                changed: Arc::new(Condvar::new()),
                config,
                next_lease_id: AtomicU64::new(1),
                resource_change_waker: Mutex::new(None),
                native_policy_notifications: AtomicBool::new(false),
            }),
        }
    }

    pub fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<WorkScheduler>> = OnceLock::new();
        GLOBAL
            .get_or_init(|| Arc::new(WorkScheduler::new(SchedulerConfig::default())))
            .clone()
    }

    pub fn config(&self) -> &SchedulerConfig {
        &self.inner.config
    }

    /// Read the current transient policy for a class. Admission still occurs
    /// only through this scheduler's acquire methods.
    pub fn policy_decision(&self, class: WorkClass) -> ResourcePolicyDecision {
        self.inner
            .config
            .policy
            .decision(class, self.inner.config.capacities)
    }

    /// Mark whether an OS event source currently wakes policy-blocked work.
    /// Without one, timed rechecks are used only while Background work is
    /// actually blocked by policy.
    pub fn set_native_policy_notifications_available(&self, available: bool) {
        self.inner
            .native_policy_notifications
            .store(available, Ordering::Release);
        self.notify_resource_policy_changed();
    }

    pub fn native_policy_notifications_available(&self) -> bool {
        self.inner
            .native_policy_notifications
            .load(Ordering::Acquire)
    }

    /// Register the one coalescing resource-change wake slot used by
    /// Managed AI. This is a wake hint only; durable queue state stays in
    /// SQLite and WorkScheduler remains the admission authority.
    pub(crate) fn set_resource_change_waker(&self, sender: SyncSender<()>) {
        if let Ok(mut slot) = self.inner.resource_change_waker.lock() {
            *slot = Some(sender.clone());
        }
    }

    pub(crate) fn clear_resource_change_waker(&self) {
        if let Ok(mut slot) = self.inner.resource_change_waker.lock() {
            *slot = None;
        }
    }

    /// Re-evaluate queued work after a platform power/thermal change.
    pub fn notify_resource_policy_changed(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            self.dispatch_locked(&mut state);
        }
        self.signal_resource_change_waker();
    }

    fn signal_resource_change_waker(&self) {
        let sender = self
            .inner
            .resource_change_waker
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(sender) = sender {
            let _ = sender.try_send(());
        }
    }

    /// Wait for a lease, respecting priority, bounded fairness and caller
    /// cancellation. A queued request never owns durable job state.
    pub fn acquire(&self, request: WorkRequest) -> Result<ResourceLease, AcquireError> {
        self.acquire_until(request, None)
    }

    /// Acquire a lease while respecting the bounded queue as backpressure.
    /// The request remains caller-owned and cancellation/deadline behavior is
    /// unchanged; this does not create a scheduler-owned job.
    pub fn acquire_with_backpressure(
        &self,
        request: WorkRequest,
    ) -> Result<ResourceLease, AcquireError> {
        loop {
            match self.acquire(request.clone()) {
                Err(AcquireError::QueueFull) => {
                    self.wait_for_queue_capacity(&request.cancellation, None)?;
                }
                result => return result,
            }
        }
    }

    /// Wait until bounded queue backpressure has room for a request. This is
    /// used by existing authorities that own a durable request and must not
    /// turn transient scheduler overload into a durable job failure.
    pub fn wait_for_queue_capacity(
        &self,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<(), AcquireError> {
        cancellation.attach_waiter_signal(&self.inner);
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| AcquireError::Unavailable)?;
        loop {
            if cancellation.is_cancelled() {
                return Err(AcquireError::Cancelled);
            }
            if state.queue.len() < self.inner.config.max_queued {
                return Ok(());
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(AcquireError::WouldBlock);
            }
            let timeout = if cancellation.has_unobservable_external_cancel() {
                Some(deadline.map_or(EXTERNAL_CANCELLATION_RECHECK, |deadline| {
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(EXTERNAL_CANCELLATION_RECHECK)
                }))
            } else {
                deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()))
            };
            if timeout.is_some_and(|duration| duration.is_zero()) {
                return Err(AcquireError::WouldBlock);
            }
            state = if let Some(timeout) = timeout {
                self.inner
                    .changed
                    .wait_timeout(state, timeout)
                    .map_err(|_| AcquireError::Unavailable)?
                    .0
            } else {
                self.inner
                    .changed
                    .wait(state)
                    .map_err(|_| AcquireError::Unavailable)?
            };
        }
    }

    /// Wait for a lease until a caller-owned deadline. This remains a
    /// process-local admission primitive: cancellation and the deadline only
    /// control the queued request and never create scheduler-owned work.
    pub(crate) fn acquire_until(
        &self,
        request: WorkRequest,
        deadline: Option<Instant>,
    ) -> Result<ResourceLease, AcquireError> {
        self.validate_request(&request)?;
        if request.cancellation.is_cancelled() {
            return Err(AcquireError::Cancelled);
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(AcquireError::WouldBlock);
        }
        let waiter = Arc::new(Waiter::new());
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| AcquireError::Unavailable)?;
        self.remove_cancelled_locked(&mut state);
        self.cancel_superseded_locked(&mut state, &request);
        if state.queue.len() >= self.inner.config.max_queued {
            state.metrics.total_rejections += 1;
            return Err(AcquireError::QueueFull);
        }
        let cancellation = request.cancellation.clone();
        request.cancellation.attach_waiter_signal(&self.inner);
        let queued = QueuedRequest {
            sequence: next_sequence(&mut state),
            request,
            waiter: Arc::clone(&waiter),
        };
        state.queue.push_back(queued);

        loop {
            if cancellation.is_cancelled() {
                if self.remove_waiter_locked(&mut state, &waiter) {
                    state.metrics.total_cancellations += 1;
                }
                self.inner.changed.notify_all();
                return Err(AcquireError::Cancelled);
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                self.remove_waiter_locked(&mut state, &waiter);
                self.inner.changed.notify_all();
                return Err(AcquireError::WouldBlock);
            }
            self.dispatch_locked(&mut state);
            if let Some(lease) = waiter.take_lease() {
                if cancellation.is_cancelled() {
                    drop(state);
                    lease.release();
                    return Err(AcquireError::Cancelled);
                }
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    drop(state);
                    lease.release();
                    return Err(AcquireError::WouldBlock);
                }
                return Ok(lease);
            }
            let wait_duration = self.wait_duration(&state, &cancellation, deadline);
            if wait_duration.is_some_and(|duration| duration.is_zero()) {
                self.remove_waiter_locked(&mut state, &waiter);
                self.inner.changed.notify_all();
                return Err(AcquireError::WouldBlock);
            }
            state = if let Some(wait_duration) = wait_duration {
                let (mut state, result) = self
                    .inner
                    .changed
                    .wait_timeout(state, wait_duration)
                    .map_err(|_| AcquireError::Unavailable)?;
                if result.timed_out() {
                    state.metrics.total_timed_wait_wakeups =
                        state.metrics.total_timed_wait_wakeups.saturating_add(1);
                }
                state
            } else {
                self.inner
                    .changed
                    .wait(state)
                    .map_err(|_| AcquireError::Unavailable)?
            };
        }
    }

    /// Attempt a lease without waiting. Existing queued work is still
    /// dispatched according to the same ordering rules.
    pub fn try_acquire(&self, request: WorkRequest) -> Result<ResourceLease, AcquireError> {
        self.validate_request(&request)?;
        if request.cancellation.is_cancelled() {
            return Err(AcquireError::Cancelled);
        }
        let waiter = Arc::new(Waiter::new());
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| AcquireError::Unavailable)?;
        let cancellation = request.cancellation.clone();
        self.remove_cancelled_locked(&mut state);
        self.cancel_superseded_locked(&mut state, &request);
        if !self.policy_allows(&request) {
            return Err(AcquireError::PolicyDenied);
        }
        if state.queue.is_empty() {
            if !self.request_fits(&state, &request) {
                return Err(AcquireError::WouldBlock);
            }
            let queued = QueuedRequest {
                sequence: next_sequence(&mut state),
                request,
                waiter: Arc::clone(&waiter),
            };
            self.grant_locked(&mut state, queued);
            let lease = waiter.take_lease().ok_or(AcquireError::Unavailable)?;
            if cancellation.is_cancelled() {
                drop(state);
                lease.release();
                return Err(AcquireError::Cancelled);
            }
            return Ok(lease);
        }
        if state.queue.len() >= self.inner.config.max_queued {
            state.metrics.total_rejections += 1;
            return Err(AcquireError::QueueFull);
        }
        request.cancellation.attach_waiter_signal(&self.inner);
        let sequence = next_sequence(&mut state);
        state.queue.push_back(QueuedRequest {
            sequence,
            request,
            waiter: Arc::clone(&waiter),
        });
        self.dispatch_locked(&mut state);
        if let Some(lease) = waiter.take_lease() {
            if cancellation.is_cancelled() {
                drop(state);
                lease.release();
                return Err(AcquireError::Cancelled);
            }
            return Ok(lease);
        }
        let removed = self.remove_waiter_by_sequence_locked(&mut state, sequence);
        if removed {
            self.inner.changed.notify_all();
        }
        if cancellation.is_cancelled() {
            if removed {
                state.metrics.total_cancellations += 1;
            }
            return Err(AcquireError::Cancelled);
        }
        Err(AcquireError::WouldBlock)
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        self.cancel_matching(|request, active| {
            request
                .map(|request| request.request_id == request_id.trim())
                .or_else(|| active.map(|active| active.request_id == request_id.trim()))
                .unwrap_or(false)
        }) > 0
    }

    pub fn cancel_session(&self, session_id: &str) -> usize {
        self.cancel_matching(|request, active| {
            request
                .and_then(|request| request.session_id.as_deref())
                .or_else(|| active.and_then(|active| active.session_id.as_deref()))
                .is_some_and(|session| session == session_id.trim())
        })
    }

    pub fn snapshot(&self) -> SchedulerSnapshot {
        let Ok(mut state) = self.inner.state.lock() else {
            return SchedulerSnapshot::default();
        };
        self.remove_cancelled_locked(&mut state);
        let mut snapshot = SchedulerSnapshot {
            queued: state.queue.len(),
            running: state.active.len(),
            granted: state.granted,
            available: self
                .inner
                .config
                .capacities
                .as_hints()
                .saturating_sub(state.granted),
            total_grants: state.metrics.total_grants,
            total_releases: state.metrics.total_releases,
            total_cancellations: state.metrics.total_cancellations,
            total_rejections: state.metrics.total_rejections,
            total_timed_wait_wakeups: state.metrics.total_timed_wait_wakeups,
            ..SchedulerSnapshot::default()
        };
        for queued in &state.queue {
            increment_class_counts(
                queued.request.class,
                &mut snapshot.queued_foreground,
                &mut snapshot.queued_interactive,
                &mut snapshot.queued_background,
            );
        }
        for active in state.active.values() {
            increment_class_counts(
                active.class,
                &mut snapshot.running_foreground,
                &mut snapshot.running_interactive,
                &mut snapshot.running_background,
            );
        }
        snapshot
    }

    fn validate_request(&self, request: &WorkRequest) -> Result<(), AcquireError> {
        if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
            return Err(AcquireError::InvalidRequest(
                "request_id must contain 1..=128 characters".to_string(),
            ));
        }
        for (name, value) in [
            ("session_id", request.session_id.as_deref()),
            ("coalesce_key", request.coalesce_key.as_deref()),
        ] {
            if value.is_some_and(|value| value.trim().is_empty() || value.len() > 128) {
                return Err(AcquireError::InvalidRequest(format!(
                    "{name} must contain 1..=128 characters when present"
                )));
            }
        }
        if request.resources.is_empty() {
            return Err(AcquireError::InvalidRequest(
                "at least one resource hint is required".to_string(),
            ));
        }
        if !request
            .resources
            .fits_in(self.inner.config.capacities.as_hints())
        {
            return Err(AcquireError::InvalidRequest(
                "resource hints exceed configured capacity".to_string(),
            ));
        }
        Ok(())
    }

    fn policy_allows(&self, request: &WorkRequest) -> bool {
        let decision = self
            .inner
            .config
            .policy
            .decision(request.class, self.inner.config.capacities);
        !matches!(request.class, WorkClass::Background) || decision.allow_background
    }

    fn policy_blocks_request(&self, request: &WorkRequest) -> bool {
        if !matches!(request.class, WorkClass::Background) {
            return false;
        }
        let decision = self
            .inner
            .config
            .policy
            .decision(request.class, self.inner.config.capacities);
        !decision.allow_background
            || (request
                .resources
                .fits_in(self.inner.config.capacities.as_hints())
                && !request
                    .resources
                    .fits_in(decision.effective_capacity.as_hints()))
    }

    fn request_fits(&self, state: &SchedulerState, request: &WorkRequest) -> bool {
        let decision = self
            .inner
            .config
            .policy
            .decision(request.class, self.inner.config.capacities);
        if matches!(request.class, WorkClass::Background) && !decision.allow_background {
            return false;
        }
        request.resources.fits_in(
            decision
                .effective_capacity
                .as_hints()
                .saturating_sub(state.granted),
        )
    }

    fn wait_duration(
        &self,
        state: &SchedulerState,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Option<Duration> {
        let mut timeout = if cancellation.has_unobservable_external_cancel() {
            Some(EXTERNAL_CANCELLATION_RECHECK)
        } else {
            None
        };
        if !self.native_policy_notifications_available()
            && state.queue.iter().any(|queued| {
                matches!(queued.request.class, WorkClass::Background)
                    && self.policy_blocks_request(&queued.request)
            })
        {
            timeout = Some(timeout.map_or(POLICY_RECHECK, |current| current.min(POLICY_RECHECK)));
        }
        if let Some(deadline) = deadline {
            let until_deadline = deadline.saturating_duration_since(Instant::now());
            timeout = Some(timeout.map_or(until_deadline, |current| current.min(until_deadline)));
        }
        timeout
    }

    fn dispatch_locked(&self, state: &mut SchedulerState) {
        self.remove_cancelled_locked(state);
        while let Some(index) = self.select_candidate_index(state) {
            let Some(queued) = state.queue.remove(index) else {
                break;
            };
            if queued.request.cancellation.is_cancelled() {
                state.metrics.total_cancellations += 1;
                continue;
            }
            self.grant_locked(state, queued);
        }
        self.inner.changed.notify_all();
    }

    fn select_candidate_index(&self, state: &SchedulerState) -> Option<usize> {
        let background = self.oldest_eligible_index(state, WorkClass::Background);
        let background_pending = state
            .queue
            .iter()
            .any(|queued| matches!(queued.request.class, WorkClass::Background));
        if background_pending
            && state.high_priority_grants_since_background
                >= self.inner.config.background_fairness_after
            && background.is_some()
        {
            return background;
        }
        let foreground = self.oldest_eligible_index(state, WorkClass::Foreground);
        if foreground.is_some() {
            return foreground;
        }
        self.oldest_eligible_index(state, WorkClass::Interactive)
            .or(background)
    }

    fn oldest_eligible_index(&self, state: &SchedulerState, class: WorkClass) -> Option<usize> {
        state
            .queue
            .iter()
            .enumerate()
            .filter(|(_, queued)| queued.request.class == class)
            .find(|(_, queued)| self.request_fits(state, &queued.request))
            .map(|(index, _)| index)
    }

    fn grant_locked(&self, state: &mut SchedulerState, queued: QueuedRequest) {
        let lease_id = self.inner.next_lease_id.fetch_add(1, Ordering::Relaxed);
        let request = queued.request;
        state.granted = state.granted.saturating_add(request.resources);
        state.active.insert(
            lease_id,
            ActiveLease {
                request_id: request.request_id.clone(),
                session_id: request.session_id.clone(),
                coalesce_key: request.coalesce_key.clone(),
                class: request.class,
                resources: request.resources,
                cancellation: request.cancellation.clone(),
            },
        );
        state.metrics.total_grants += 1;
        if matches!(request.class, WorkClass::Background) {
            state.high_priority_grants_since_background = 0;
        } else if state
            .queue
            .iter()
            .any(|queued| matches!(queued.request.class, WorkClass::Background))
        {
            state.high_priority_grants_since_background = state
                .high_priority_grants_since_background
                .saturating_add(1);
        } else {
            state.high_priority_grants_since_background = 0;
        }
        queued.waiter.set_lease(ResourceLease {
            inner: Arc::new(ResourceLeaseInner {
                scheduler: Arc::downgrade(&self.inner),
                lease_id,
                request_id: request.request_id,
                class: request.class,
                resources: request.resources,
                cancellation: request.cancellation,
                released: AtomicBool::new(false),
            }),
        });
    }

    fn release_lease(&self, lease_id: u64) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        let Some(active) = state.active.remove(&lease_id) else {
            return;
        };
        state.granted = state.granted.saturating_sub(active.resources);
        state.metrics.total_releases += 1;
        self.dispatch_locked(&mut state);
        drop(state);
        self.signal_resource_change_waker();
    }

    fn cancel_superseded_locked(&self, state: &mut SchedulerState, request: &WorkRequest) {
        let (Some(session_id), Some(coalesce_key)) = (
            request.session_id.as_deref(),
            request.coalesce_key.as_deref(),
        ) else {
            return;
        };
        self.cancel_matching_locked(state, |queued, active| {
            queued.is_some_and(|queued| {
                queued.session_id.as_deref() == Some(session_id)
                    && queued.coalesce_key.as_deref() == Some(coalesce_key)
            }) || active.is_some_and(|active| {
                active.session_id.as_deref() == Some(session_id)
                    && active.coalesce_key.as_deref() == Some(coalesce_key)
            })
        });
    }

    fn cancel_matching<F>(&self, matches: F) -> usize
    where
        F: FnMut(Option<&WorkRequest>, Option<&ActiveLease>) -> bool,
    {
        let Ok(mut state) = self.inner.state.lock() else {
            return 0;
        };
        let count = self.cancel_matching_locked(&mut state, matches);
        self.dispatch_locked(&mut state);
        count
    }

    fn cancel_matching_locked<F>(&self, state: &mut SchedulerState, mut matches: F) -> usize
    where
        F: FnMut(Option<&WorkRequest>, Option<&ActiveLease>) -> bool,
    {
        let mut count = 0;
        let mut active_cancellations = 0;
        for queued in &state.queue {
            if matches(Some(&queued.request), None) {
                queued.request.cancellation.cancel_without_notification();
                count += 1;
            }
        }
        for active in state.active.values() {
            if matches(None, Some(active)) {
                active.cancellation.cancel_without_notification();
                count += 1;
                active_cancellations += 1;
            }
        }
        state.metrics.total_cancellations = state
            .metrics
            .total_cancellations
            .saturating_add(active_cancellations as u64);
        self.remove_cancelled_locked(state);
        if count > 0 {
            self.inner.changed.notify_all();
        }
        count
    }

    fn remove_cancelled_locked(&self, state: &mut SchedulerState) {
        let mut retained = VecDeque::with_capacity(state.queue.len());
        while let Some(queued) = state.queue.pop_front() {
            if queued.request.cancellation.is_cancelled() {
                state.metrics.total_cancellations =
                    state.metrics.total_cancellations.saturating_add(1);
            } else {
                retained.push_back(queued);
            }
        }
        state.queue = retained;
    }

    fn remove_waiter_locked(&self, state: &mut SchedulerState, waiter: &Arc<Waiter>) -> bool {
        if let Some(index) = state
            .queue
            .iter()
            .position(|queued| Arc::ptr_eq(&queued.waiter, waiter))
        {
            state.queue.remove(index);
            true
        } else {
            false
        }
    }

    fn remove_waiter_by_sequence_locked(&self, state: &mut SchedulerState, sequence: u64) -> bool {
        if let Some(index) = state
            .queue
            .iter()
            .position(|queued| queued.sequence == sequence)
        {
            state.queue.remove(index);
            true
        } else {
            false
        }
    }
}

fn next_sequence(state: &mut SchedulerState) -> u64 {
    let sequence = state.next_sequence;
    state.next_sequence = state.next_sequence.wrapping_add(1).max(1);
    sequence
}

fn increment_class_counts(
    class: WorkClass,
    foreground: &mut usize,
    interactive: &mut usize,
    background: &mut usize,
) {
    match class {
        WorkClass::Foreground => *foreground += 1,
        WorkClass::Interactive => *interactive += 1,
        WorkClass::Background => *background += 1,
    }
}

/// Non-authoritative scheduler instrumentation for tests and future metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerSnapshot {
    pub queued: usize,
    pub running: usize,
    pub queued_foreground: usize,
    pub queued_interactive: usize,
    pub queued_background: usize,
    pub running_foreground: usize,
    pub running_interactive: usize,
    pub running_background: usize,
    pub granted: ResourceHints,
    pub available: ResourceHints,
    pub total_grants: u64,
    pub total_releases: u64,
    pub total_cancellations: u64,
    pub total_rejections: u64,
    pub total_timed_wait_wakeups: u64,
}

/// Adapters for existing heavy authorities. These wrappers only acquire
/// process-local capacity; the authority retains job lifecycle and state.
pub mod adapters {
    use super::{
        AcquireError, CancellationToken, ResourceHints, ResourceLease, WorkClass, WorkRequest,
        WorkScheduler,
    };
    use std::sync::Arc;
    #[cfg(test)]
    use std::sync::{Condvar, Mutex};
    use std::time::Instant;

    /// Bounded resource adapter for the existing managed scan/reconciliation
    /// authority. It does not claim, cancel, persist or finalize scan runs.
    #[derive(Clone)]
    pub struct ManagedScanResourceLeaseAdapter {
        scheduler: Arc<WorkScheduler>,
    }

    impl ManagedScanResourceLeaseAdapter {
        pub fn new(scheduler: Arc<WorkScheduler>) -> Self {
            Self { scheduler }
        }

        pub fn global() -> Self {
            Self::new(WorkScheduler::global())
        }

        pub fn try_acquire(
            &self,
            run_id: &str,
            class: WorkClass,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                run_id.to_string(),
                class,
                ResourceHints {
                    cpu: 1,
                    io: 1,
                    open_handles: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(run_id.to_string())
            .with_cancellation(cancellation);
            self.scheduler.try_acquire(request)
        }

        pub fn acquire(
            &self,
            run_id: &str,
            class: WorkClass,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                run_id.to_string(),
                class,
                ResourceHints {
                    cpu: 1,
                    io: 1,
                    open_handles: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(run_id.to_string())
            .with_cancellation(cancellation);
            self.scheduler.acquire_with_backpressure(request)
        }

        pub fn scheduler(&self) -> Arc<WorkScheduler> {
            Arc::clone(&self.scheduler)
        }
    }

    /// Bounded admission for one live Folder Preview directory traversal.
    /// The lease is held for the lifetime of the temporary Browse
    /// enumeration, including its live directory handle, and is released by
    /// RAII when the adapter exits.
    #[derive(Clone)]
    pub struct FolderPreviewResourceLeaseAdapter {
        scheduler: Arc<WorkScheduler>,
    }

    impl FolderPreviewResourceLeaseAdapter {
        pub fn new(scheduler: Arc<WorkScheduler>) -> Self {
            Self { scheduler }
        }

        pub fn global() -> Self {
            Self::new(WorkScheduler::global())
        }

        fn request(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> WorkRequest {
            WorkRequest::new(
                request_id.to_string(),
                WorkClass::Interactive,
                ResourceHints {
                    io: 1,
                    open_handles: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(session_id.to_string())
            .with_coalesce_key("folder-preview-enumeration")
            .with_cancellation(cancellation)
        }

        pub fn acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            self.scheduler
                .acquire(self.request(request_id, session_id, cancellation))
        }

        pub fn try_acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            self.scheduler
                .try_acquire(self.request(request_id, session_id, cancellation))
        }

        pub fn scheduler(&self) -> Arc<WorkScheduler> {
            Arc::clone(&self.scheduler)
        }
    }

    /// Thin admission adapter for bounded Preview decoders. The Preview
    /// provider owns decode lifecycle; WorkScheduler remains the sole
    /// authority for decoder capacity and the returned lease releases it by
    /// RAII.
    #[derive(Clone)]
    pub struct PreviewDecoderResourceLeaseAdapter {
        scheduler: Arc<WorkScheduler>,
        #[cfg(test)]
        test_barrier: Option<Arc<PreviewDecoderAdmissionTestBarrier>>,
    }

    #[cfg(test)]
    #[derive(Default)]
    pub(crate) struct PreviewDecoderAdmissionTestBarrier {
        state: Mutex<(bool, bool)>,
        wake: Condvar,
    }

    #[cfg(test)]
    impl PreviewDecoderAdmissionTestBarrier {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn wait_for_entry(&self) {
            let mut state = self.state.lock().expect("decoder admission barrier lock");
            while !state.0 {
                state = self
                    .wake
                    .wait(state)
                    .expect("decoder admission barrier wait");
            }
        }

        pub(crate) fn release(&self) {
            let mut state = self.state.lock().expect("decoder admission barrier lock");
            state.1 = true;
            self.wake.notify_all();
        }

        fn pause_after_acquire(&self) {
            let mut state = self.state.lock().expect("decoder admission barrier lock");
            state.0 = true;
            self.wake.notify_all();
            while !state.1 {
                state = self
                    .wake
                    .wait(state)
                    .expect("decoder admission barrier wait");
            }
        }
    }

    impl PreviewDecoderResourceLeaseAdapter {
        pub fn new(scheduler: Arc<WorkScheduler>) -> Self {
            Self {
                scheduler,
                #[cfg(test)]
                test_barrier: None,
            }
        }

        pub fn global() -> Self {
            Self::new(WorkScheduler::global())
        }

        #[cfg(test)]
        pub(crate) fn new_with_test_barrier(
            scheduler: Arc<WorkScheduler>,
            test_barrier: Arc<PreviewDecoderAdmissionTestBarrier>,
        ) -> Self {
            Self {
                scheduler,
                test_barrier: Some(test_barrier),
            }
        }

        pub fn acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                request_id.to_string(),
                WorkClass::Interactive,
                ResourceHints {
                    decoder: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(session_id.to_string())
            .with_cancellation(cancellation);
            let lease = self.scheduler.acquire(request)?;
            #[cfg(test)]
            if let Some(test_barrier) = self.test_barrier.as_ref() {
                test_barrier.pause_after_acquire();
            }
            Ok(lease)
        }

        pub fn try_acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                request_id.to_string(),
                WorkClass::Interactive,
                ResourceHints {
                    decoder: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(session_id.to_string())
            .with_cancellation(cancellation);
            let lease = self.scheduler.try_acquire(request)?;
            #[cfg(test)]
            if let Some(test_barrier) = self.test_barrier.as_ref() {
                test_barrier.pause_after_acquire();
            }
            Ok(lease)
        }

        pub fn scheduler(&self) -> Arc<WorkScheduler> {
            Arc::clone(&self.scheduler)
        }
    }

    /// Thin admission adapter for bounded ZIP central-directory indexing.
    /// The archive provider owns the disposable parser; WorkScheduler remains
    /// the only CPU/I/O capacity authority and the returned lease releases by
    /// RAII on every provider exit path.
    #[derive(Clone)]
    pub struct PreviewArchiveResourceLeaseAdapter {
        scheduler: Arc<WorkScheduler>,
    }

    impl PreviewArchiveResourceLeaseAdapter {
        pub fn new(scheduler: Arc<WorkScheduler>) -> Self {
            Self { scheduler }
        }

        pub fn global() -> Self {
            Self::new(WorkScheduler::global())
        }

        pub fn try_acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                request_id.to_string(),
                WorkClass::Interactive,
                ResourceHints {
                    cpu: 1,
                    io: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(session_id.to_string())
            .with_cancellation(cancellation);
            self.scheduler.try_acquire(request)
        }

        pub fn scheduler(&self) -> Arc<WorkScheduler> {
            Arc::clone(&self.scheduler)
        }
    }

    /// Thin admission adapter for the complete Native Preview staging copy.
    /// The Native Preview Access registry retains staging/token lifecycle;
    /// this adapter only holds the shared scheduler lease for the expensive
    /// source-plus-staged-output operation.
    #[derive(Clone)]
    pub struct NativePreviewResourceLeaseAdapter {
        scheduler: Arc<WorkScheduler>,
    }

    impl NativePreviewResourceLeaseAdapter {
        pub fn new(scheduler: Arc<WorkScheduler>) -> Self {
            Self { scheduler }
        }

        pub fn global() -> Self {
            Self::new(WorkScheduler::global())
        }

        pub fn acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
            deadline: Instant,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                request_id.to_string(),
                WorkClass::Interactive,
                ResourceHints {
                    io: 1,
                    open_handles: 2,
                    native_preview: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(session_id.to_string())
            .with_cancellation(cancellation);
            self.scheduler.acquire_until(request, Some(deadline))
        }

        #[cfg(test)]
        pub(crate) fn try_acquire(
            &self,
            request_id: &str,
            session_id: &str,
            cancellation: CancellationToken,
        ) -> Result<ResourceLease, AcquireError> {
            let request = WorkRequest::new(
                request_id.to_string(),
                WorkClass::Interactive,
                ResourceHints {
                    io: 1,
                    open_handles: 2,
                    native_preview: 1,
                    ..ResourceHints::empty()
                },
            )
            .with_session_id(session_id.to_string())
            .with_cancellation(cancellation);
            self.scheduler.try_acquire(request)
        }

        pub fn scheduler(&self) -> Arc<WorkScheduler> {
            Arc::clone(&self.scheduler)
        }
    }

    pub type ScanResourceLeaseAdapter = ManagedScanResourceLeaseAdapter;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        platform::macos::activity::{MacActivitySnapshot, MacThermalState},
        resource_governor::RuntimeResourceGovernor,
    };
    use std::sync::mpsc;
    use std::thread;

    fn test_scheduler(cpu: u32) -> WorkScheduler {
        WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(cpu, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        )
    }

    struct SwitchableResourcePolicy(AtomicBool);

    impl PlatformResourcePolicy for SwitchableResourcePolicy {
        fn decision(
            &self,
            class: WorkClass,
            configured_capacity: ResourceCapacities,
        ) -> ResourcePolicyDecision {
            let background = matches!(class, WorkClass::Background);
            let admitted = !background || self.0.load(Ordering::Acquire);
            ResourcePolicyDecision {
                effective_capacity: if admitted {
                    configured_capacity
                } else {
                    ResourceCapacities {
                        cpu: configured_capacity.cpu.min(1),
                        ..configured_capacity
                    }
                },
                allow_background: admitted,
                efficiency_qos: background,
            }
        }
    }

    struct SwitchableCapacityPolicy(AtomicBool);

    impl PlatformResourcePolicy for SwitchableCapacityPolicy {
        fn decision(
            &self,
            class: WorkClass,
            configured_capacity: ResourceCapacities,
        ) -> ResourcePolicyDecision {
            let constrained =
                matches!(class, WorkClass::Background) && !self.0.load(Ordering::Acquire);
            ResourcePolicyDecision {
                effective_capacity: if constrained {
                    ResourceCapacities {
                        cpu: configured_capacity.cpu.min(1),
                        ..configured_capacity
                    }
                } else {
                    configured_capacity
                },
                allow_background: true,
                efficiency_qos: matches!(class, WorkClass::Background),
            }
        }
    }

    fn request(id: &str, class: WorkClass) -> WorkRequest {
        WorkRequest::new(id, class, ResourceHints::cpu_io(1, 1))
    }

    fn wait_for_queued(scheduler: &WorkScheduler, queued: usize) {
        for _ in 0..10_000 {
            if scheduler.snapshot().queued >= queued {
                return;
            }
            thread::yield_now();
        }
        panic!("scheduler did not reach queued count {queued}");
    }

    #[test]
    fn foreground_is_admitted_before_queued_lower_priority_work() {
        let scheduler = Arc::new(test_scheduler(1));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Interactive))
            .expect("holder lease");
        let (tx, rx) = mpsc::channel();
        let background_scheduler = Arc::clone(&scheduler);
        let background = thread::spawn(move || {
            let lease = background_scheduler
                .acquire(request("background", WorkClass::Background))
                .expect("background lease");
            tx.send(lease.request_id().to_string())
                .expect("background result");
            drop(lease);
        });
        wait_for_queued(&scheduler, 1);
        let (tx_foreground, rx_foreground) = mpsc::channel();
        let foreground_scheduler = Arc::clone(&scheduler);
        let foreground = thread::spawn(move || {
            let lease = foreground_scheduler
                .acquire(request("foreground", WorkClass::Foreground))
                .expect("foreground lease");
            tx_foreground
                .send(lease.request_id().to_string())
                .expect("foreground result");
            drop(lease);
        });
        wait_for_queued(&scheduler, 2);
        drop(holder);
        assert_eq!(
            rx_foreground.recv().expect("foreground admission"),
            "foreground"
        );
        assert_eq!(rx.recv().expect("background admission"), "background");
        foreground.join().expect("foreground thread");
        background.join().expect("background thread");
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn background_makes_eventual_progress_under_sustained_interactive_load() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_background_fairness_after(2)
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Foreground))
            .expect("holder lease");
        let (tx_all, rx_all) = mpsc::channel();
        let background_scheduler = Arc::clone(&scheduler);
        let tx_background = tx_all.clone();
        let background = thread::spawn(move || {
            let lease = background_scheduler
                .acquire(request("background", WorkClass::Background))
                .expect("background lease");
            tx_background
                .send(lease.request_id().to_string())
                .expect("background result");
            drop(lease);
        });
        wait_for_queued(&scheduler, 1);
        let mut interactive_threads = Vec::new();
        for index in 0..3 {
            let interactive_scheduler = Arc::clone(&scheduler);
            let tx_interactive = tx_all.clone();
            let id = format!("interactive-{index}");
            interactive_threads.push(thread::spawn(move || {
                let lease = interactive_scheduler
                    .acquire(request(&id, WorkClass::Interactive))
                    .expect("interactive lease");
                tx_interactive
                    .send(lease.request_id().to_string())
                    .expect("interactive result");
                drop(lease);
            }));
        }
        wait_for_queued(&scheduler, 4);
        drop(holder);
        drop(tx_all);
        let admissions = (0..4)
            .map(|_| rx_all.recv().expect("scheduler admission"))
            .collect::<Vec<_>>();
        assert!(admissions
            .iter()
            .position(|id| id == "background")
            .is_some_and(|index| index < 3));
        for thread in interactive_threads {
            thread.join().expect("interactive thread");
        }
        background.join().expect("background thread");
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn background_makes_eventual_progress_under_sustained_foreground_load() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_background_fairness_after(2)
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Foreground))
            .expect("holder lease");
        let (tx, rx) = mpsc::channel();
        let background_scheduler = Arc::clone(&scheduler);
        let background_tx = tx.clone();
        let background = thread::spawn(move || {
            let lease = background_scheduler
                .acquire(request("background", WorkClass::Background))
                .expect("background lease");
            background_tx
                .send(lease.request_id().to_string())
                .expect("background result");
            drop(lease);
        });
        wait_for_queued(&scheduler, 1);

        let mut foreground_threads = Vec::new();
        for index in 0..2 {
            let foreground_scheduler = Arc::clone(&scheduler);
            let foreground_tx = tx.clone();
            let id = format!("foreground-{index}");
            foreground_threads.push(thread::spawn(move || {
                let lease = foreground_scheduler
                    .acquire(request(&id, WorkClass::Foreground))
                    .expect("foreground lease");
                foreground_tx
                    .send(lease.request_id().to_string())
                    .expect("foreground result");
                drop(lease);
            }));
            wait_for_queued(&scheduler, index + 2);
        }

        drop(holder);
        drop(tx);
        let admissions = (0..3)
            .map(|_| rx.recv().expect("scheduler admission"))
            .collect::<Vec<_>>();
        assert_eq!(
            admissions,
            vec![
                "foreground-0".to_string(),
                "foreground-1".to_string(),
                "background".to_string()
            ]
        );
        for thread in foreground_threads {
            thread.join().expect("foreground thread");
        }
        background.join().expect("background thread");
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn dropping_or_releasing_a_lease_returns_capacity_deterministically() {
        let scheduler = test_scheduler(1);
        let lease = scheduler
            .try_acquire(request("first", WorkClass::Foreground))
            .expect("first lease");
        assert_eq!(scheduler.snapshot().granted.cpu, 1);
        lease.release();
        lease.release();
        assert_eq!(scheduler.snapshot().granted, ResourceHints::empty());
        assert_eq!(scheduler.snapshot().total_releases, 1);
        let second = scheduler
            .try_acquire(request("second", WorkClass::Foreground))
            .expect("second lease");
        drop(second);
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn cancelled_waiter_cannot_consume_a_later_lease() {
        let scheduler = Arc::new(test_scheduler(1));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Foreground))
            .expect("holder lease");
        let cancellation = CancellationToken::new();
        let mut waiter_request = request("cancelled", WorkClass::Interactive);
        waiter_request.cancellation = cancellation.clone();
        let waiter_scheduler = Arc::clone(&scheduler);
        let waiter = thread::spawn(move || waiter_scheduler.acquire(waiter_request));
        wait_for_queued(&scheduler, 1);
        cancellation.cancel();
        assert!(matches!(
            waiter.join().expect("waiter thread"),
            Err(AcquireError::Cancelled)
        ));
        drop(holder);
        let replacement = scheduler
            .try_acquire(request("replacement", WorkClass::Foreground))
            .expect("replacement lease");
        assert_eq!(replacement.request_id(), "replacement");
        drop(replacement);
        assert_eq!(scheduler.snapshot().queued, 0);
    }

    #[test]
    fn cancellation_waker_synchronizes_with_the_scheduler_condition_mutex() {
        let scheduler = Arc::new(test_scheduler(1));
        let cancellation = CancellationToken::new();
        let scheduler_state = scheduler.inner.state.lock().expect("scheduler state");
        cancellation.attach_waiter_signal(&scheduler.inner);

        let waiter_cancellation = cancellation.clone();
        let waiter_scheduler = Arc::clone(&scheduler);
        let (waiter_started_tx, waiter_started_rx) = mpsc::channel();
        let (waiter_result_tx, waiter_result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            waiter_started_tx.send(()).expect("waiter start signal");
            waiter_result_tx
                .send(
                    waiter_scheduler.acquire(
                        request("cancel-wake-handshake", WorkClass::Interactive)
                            .with_cancellation(waiter_cancellation),
                    ),
                )
                .expect("waiter result");
        });
        waiter_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("waiter begins before cancellation");

        let cancel_token = cancellation.clone();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let cancel_thread = thread::spawn(move || {
            cancel_token.cancel();
            cancelled_tx.send(()).expect("cancellation result");
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while !cancellation.is_cancelled() {
            assert!(Instant::now() < deadline, "cancellation flag was not set");
            thread::yield_now();
        }
        assert!(
            cancelled_rx
                .recv_timeout(Duration::from_millis(25))
                .is_err(),
            "cancellation notification must take the scheduler condition mutex"
        );

        drop(scheduler_state);
        cancelled_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("cancellation notification completes after the scheduler unlocks");
        assert!(matches!(
            waiter_result_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("cancelled waiter wakes"),
            Err(AcquireError::Cancelled)
        ));
        cancel_thread.join().expect("cancellation thread");
        waiter.join().expect("waiter thread");
        assert_eq!(scheduler.snapshot().queued, 0);
    }

    #[test]
    fn preview_decoder_adapter_cancellation_while_waiting_preserves_capacity() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let adapter = adapters::PreviewDecoderResourceLeaseAdapter::new(Arc::clone(&scheduler));
        let holder = adapter
            .try_acquire(
                "decoder-holder",
                "decoder-session",
                CancellationToken::new(),
            )
            .expect("decoder holder lease");
        let cancellation = CancellationToken::new();
        let waiter_cancellation = cancellation.clone();
        let waiter_adapter = adapter.clone();
        let (tx, rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            tx.send(waiter_adapter.acquire(
                "decoder-waiter",
                "decoder-session",
                waiter_cancellation,
            ))
            .expect("decoder waiter result");
        });

        wait_for_queued(&scheduler, 1);
        assert_eq!(scheduler.snapshot().granted.decoder, 1);
        cancellation.cancel();
        assert!(matches!(
            rx.recv().expect("cancelled decoder waiter"),
            Err(AcquireError::Cancelled)
        ));
        waiter.join().expect("decoder waiter thread");
        assert_eq!(scheduler.snapshot().granted.decoder, 1);
        assert_eq!(scheduler.snapshot().queued, 0);

        drop(holder);
        assert_eq!(scheduler.snapshot().granted.decoder, 0);
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn native_preview_adapter_uses_shared_native_io_and_handle_capacity() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 2, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let adapter = adapters::NativePreviewResourceLeaseAdapter::new(Arc::clone(&scheduler));
        let lease = adapter
            .try_acquire("native-a", "session-a", CancellationToken::new())
            .expect("native preview lease admitted");
        assert_eq!(lease.class(), WorkClass::Interactive);
        assert_eq!(lease.resources().io, 1);
        assert_eq!(lease.resources().open_handles, 2);
        assert_eq!(lease.resources().native_preview, 1);
        assert!(matches!(
            adapter.try_acquire("native-b", "session-b", CancellationToken::new()),
            Err(AcquireError::WouldBlock)
        ));
        drop(lease);
        assert_eq!(scheduler.snapshot().granted, ResourceHints::empty());
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn preview_archive_adapter_uses_shared_cpu_io_capacity_and_releases_by_raii() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let adapter = adapters::PreviewArchiveResourceLeaseAdapter::new(Arc::clone(&scheduler));
        let holder = adapter
            .try_acquire(
                "archive-holder",
                "archive-session",
                CancellationToken::new(),
            )
            .expect("archive lease");
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.granted.cpu, 1);
        assert_eq!(snapshot.granted.io, 1);
        assert!(matches!(
            adapter.try_acquire(
                "archive-blocked",
                "archive-session",
                CancellationToken::new(),
            ),
            Err(AcquireError::WouldBlock) | Err(AcquireError::QueueFull)
        ));
        drop(holder);
        assert_eq!(scheduler.snapshot().granted, ResourceHints::empty());
        let replacement = adapter
            .try_acquire(
                "archive-replacement",
                "archive-session",
                CancellationToken::new(),
            )
            .expect("replacement archive lease");
        drop(replacement);
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn queued_cancellation_is_counted_exactly_once() {
        let scheduler = Arc::new(test_scheduler(1));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Foreground))
            .expect("holder lease");
        let queued_request = request("cancelled", WorkClass::Background)
            .with_session_id("session-1")
            .with_coalesce_key("scan");
        let waiter_scheduler = Arc::clone(&scheduler);
        let waiter = thread::spawn(move || waiter_scheduler.acquire(queued_request));
        wait_for_queued(&scheduler, 1);

        assert_eq!(scheduler.cancel_session("session-1"), 1);
        assert!(matches!(
            waiter.join().expect("cancelled waiter thread"),
            Err(AcquireError::Cancelled)
        ));
        assert_eq!(scheduler.snapshot().total_cancellations, 1);
        drop(holder);
    }

    #[test]
    fn queued_request_waits_for_release_without_periodic_wakes_or_duplicate_grants() {
        let scheduler = Arc::new(test_scheduler(1));
        let holder = scheduler
            .try_acquire(request("resource-holder", WorkClass::Foreground))
            .expect("holder lease");
        let executions = Arc::new(AtomicU64::new(0));
        let worker_executions = Arc::clone(&executions);
        let waiter_scheduler = Arc::clone(&scheduler);
        let (tx, rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let lease = waiter_scheduler
                .acquire(request("resource-waiter", WorkClass::Background))
                .expect("background lease after release");
            worker_executions.fetch_add(1, Ordering::AcqRel);
            tx.send(lease.request_id().to_string())
                .expect("send admitted request");
            drop(lease);
        });
        wait_for_queued(&scheduler, 1);
        thread::sleep(Duration::from_millis(130));
        assert_eq!(scheduler.snapshot().total_timed_wait_wakeups, 0);

        drop(holder);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1))
                .expect("resource release wakes waiter"),
            "resource-waiter"
        );
        waiter.join().expect("waiter joins");
        assert_eq!(executions.load(Ordering::Acquire), 1);
        assert_eq!(scheduler.snapshot().total_grants, 2);
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn native_policy_change_wakes_blocked_background_and_restores_capacity() {
        let policy = Arc::new(SwitchableResourcePolicy(AtomicBool::new(false)));
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(4, 4, 8, 1, 1, 1))
                .with_policy(policy.clone()),
        ));
        scheduler.set_native_policy_notifications_available(true);
        assert_eq!(
            scheduler
                .policy_decision(WorkClass::Background)
                .effective_capacity
                .cpu,
            1
        );
        let waiter_scheduler = Arc::clone(&scheduler);
        let (tx, rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let result = waiter_scheduler.acquire(request("policy-waiter", WorkClass::Background));
            tx.send(result.map(|lease| lease.request_id().to_string()))
                .expect("send policy admission");
        });
        wait_for_queued(&scheduler, 1);
        thread::sleep(Duration::from_millis(130));
        assert_eq!(scheduler.snapshot().total_timed_wait_wakeups, 0);

        policy.0.store(true, Ordering::Release);
        scheduler.notify_resource_policy_changed();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1))
                .expect("policy event wakes blocked background"),
            Ok("policy-waiter".to_string())
        );
        waiter.join().expect("policy waiter joins");
        assert_eq!(
            scheduler
                .policy_decision(WorkClass::Background)
                .effective_capacity
                .cpu,
            4
        );
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn eventless_background_capacity_block_gets_only_the_slow_policy_fallback() {
        let policy = Arc::new(SwitchableCapacityPolicy(AtomicBool::new(false)));
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(4, 4, 8, 1, 1, 1))
                .with_policy(policy.clone()),
        ));
        let cancellation = CancellationToken::new();
        let waiter_cancellation = cancellation.clone();
        let waiter_scheduler = Arc::clone(&scheduler);
        let request = WorkRequest::new(
            "capacity-policy-waiter",
            WorkClass::Background,
            ResourceHints::cpu_io(2, 1),
        )
        .with_cancellation(waiter_cancellation);
        let (tx, rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            tx.send(waiter_scheduler.acquire(request))
                .expect("send capacity-policy result");
        });
        wait_for_queued(&scheduler, 1);
        let state = scheduler.inner.state.lock().expect("scheduler state");
        assert_eq!(
            scheduler.wait_duration(&state, &cancellation, None),
            Some(POLICY_RECHECK),
            "only a policy-reduced eligible capacity gets the 5s recheck"
        );
        drop(state);

        policy.0.store(true, Ordering::Release);
        scheduler.notify_resource_policy_changed();
        let lease = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("policy event wakes request")
            .expect("restored capacity grants request");
        assert_eq!(lease.resources().cpu, 2);
        drop(lease);
        waiter.join().expect("capacity-policy waiter joins");
    }

    #[test]
    fn superseded_session_work_is_cancelled_before_new_work_consumes_capacity() {
        let scheduler = Arc::new(test_scheduler(1));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Foreground))
            .expect("holder lease");
        let old_request = request("old", WorkClass::Interactive)
            .with_session_id("session-1")
            .with_coalesce_key("visible-preview");
        let old_scheduler = Arc::clone(&scheduler);
        let (old_tx, old_rx) = mpsc::channel();
        let old = thread::spawn(move || {
            let result = old_scheduler.acquire(old_request);
            old_tx
                .send(result.map(|lease| lease.request_id().to_string()))
                .expect("old result");
        });
        wait_for_queued(&scheduler, 1);

        let new_request = request("new", WorkClass::Interactive)
            .with_session_id("session-1")
            .with_coalesce_key("visible-preview");
        let new_scheduler = Arc::clone(&scheduler);
        let (new_tx, new_rx) = mpsc::channel();
        let new = thread::spawn(move || {
            let lease = new_scheduler.acquire(new_request).expect("new lease");
            new_tx
                .send(lease.request_id().to_string())
                .expect("new result");
            drop(lease);
        });
        assert!(matches!(
            old_rx.recv().expect("superseded result"),
            Err(AcquireError::Cancelled)
        ));
        drop(holder);
        assert_eq!(new_rx.recv().expect("new admission"), "new");
        old.join().expect("old thread");
        new.join().expect("new thread");
        assert_eq!(scheduler.snapshot().running, 0);
    }

    #[test]
    fn queue_backpressure_is_bounded_under_overload() {
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_max_queued(1)
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        let holder = scheduler
            .try_acquire(request("holder", WorkClass::Foreground))
            .expect("holder lease");
        let waiter_scheduler = Arc::clone(&scheduler);
        let waiter = thread::spawn(move || {
            waiter_scheduler.acquire(request("queued", WorkClass::Background))
        });
        wait_for_queued(&scheduler, 1);
        assert!(matches!(
            scheduler.try_acquire(request("overflow", WorkClass::Background)),
            Err(AcquireError::QueueFull)
        ));
        drop(holder);
        drop(waiter.join().expect("queued lease").expect("queued result"));
        assert_eq!(scheduler.snapshot().queued, 0);
    }

    #[test]
    fn repeated_acquire_release_returns_all_resource_counters_to_steady_state() {
        let scheduler = test_scheduler(2);
        for index in 0..100 {
            let lease = scheduler
                .try_acquire(request(&format!("repeat-{index}"), WorkClass::Interactive))
                .expect("repeated lease");
            assert_eq!(scheduler.snapshot().running, 1);
            drop(lease);
        }
        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.granted, ResourceHints::empty());
        assert_eq!(snapshot.available.cpu, 2);
        assert_eq!(snapshot.running, 0);
        assert_eq!(snapshot.total_grants, 100);
        assert_eq!(snapshot.total_releases, 100);
    }

    #[test]
    fn mac_policy_preserves_critical_foreground_and_blocks_nonessential_background() {
        let policy = Arc::new(RuntimeResourceGovernor::from_snapshot(
            MacActivitySnapshot {
                thermal: MacThermalState::Critical,
                low_power_mode: false,
            },
        ));
        let scheduler = WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(4, 1, 8, 1, 1, 1))
                .with_policy(policy),
        );
        let foreground = scheduler
            .try_acquire(request("essential", WorkClass::Foreground))
            .expect("critical thermal must retain essential foreground work");
        assert_eq!(foreground.resources().cpu, 1);
        assert!(matches!(
            scheduler.try_acquire(request("background", WorkClass::Background)),
            Err(AcquireError::PolicyDenied)
        ));
        drop(foreground);
    }

    #[test]
    fn mac_policy_keeps_low_power_background_bounded_but_admitted() {
        let policy = Arc::new(RuntimeResourceGovernor::from_snapshot(
            MacActivitySnapshot {
                thermal: MacThermalState::Nominal,
                low_power_mode: true,
            },
        ));
        let scheduler = WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(8, 1, 8, 1, 1, 1))
                .with_policy(policy),
        );
        let background = scheduler
            .try_acquire(request("background", WorkClass::Background))
            .expect("low power background should remain admitted");
        assert_eq!(scheduler.snapshot().granted.cpu, 1);
        drop(background);
    }

    #[test]
    fn scan_adapter_only_leases_capacity_and_leaves_caller_lifecycle_untouched() {
        let scheduler = Arc::new(test_scheduler(1));
        let adapter = adapters::ManagedScanResourceLeaseAdapter::new(Arc::clone(&scheduler));
        let authority_state = Arc::new(AtomicU64::new(0));
        let cancellation = CancellationToken::new();
        let lease = adapter
            .try_acquire("scan-run-1", WorkClass::Background, cancellation.clone())
            .expect("scan resource lease");
        authority_state.fetch_add(1, Ordering::Relaxed);
        assert_eq!(authority_state.load(Ordering::Relaxed), 1);
        cancellation.cancel();
        assert!(lease.is_cancelled());
        assert_eq!(scheduler.snapshot().running, 1);
        drop(lease);
        assert_eq!(scheduler.snapshot().running, 0);
        assert_eq!(authority_state.load(Ordering::Relaxed), 1);
    }
}
