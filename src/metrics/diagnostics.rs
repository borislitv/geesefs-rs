//! Opt-in phase observations. Labels are closed application vocabularies; only
//! diagnostic records carry request/inode identities, never paths or S3 errors.
use std::{
    collections::BTreeMap,
    future::Future,
    io::Write,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::config::DiagnosticLogLevel;
use prometheus_client::{
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use tracing_appender::non_blocking::{NonBlocking, NonBlockingBuilder, WorkerGuard};

type Labels = Vec<(&'static str, &'static str)>;
type Histograms = Family<Labels, Histogram, fn() -> Histogram>;
const BUCKETS: [f64; 19] = [
    0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1., 2.5, 5., 10., 30.,
    60., 120., 300., 600.,
];
fn histogram() -> Histogram {
    Histogram::new(BUCKETS)
}

macro_rules! vocabulary {
    ($name:ident, $count:ident, $( $variant:ident => $label:literal ),+ $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        pub(crate) enum $name { $($variant),+ }
        const $count: usize = [$($name::$variant),+].len();
        impl $name {
            const ALL: [Self; $count] = [$(Self::$variant),+];
            fn label(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
        }
    };
}
vocabulary!(Opcode, OPCODES,
    Lookup => "lookup", Getattr => "getattr", Read => "read", Readdir => "readdir",
    Readdirplus => "readdirplus", Open => "open", Write => "write", Fsync => "fsync",
    Mutation => "mutation", Other => "other",
);
vocabulary!(Stage, STAGES,
    PayloadAdmission => "payload_admission", FillWait => "fill_wait", CacheRead => "cache_read",
    NamespaceWait => "namespace_wait_sampled", NamespaceHold => "namespace_hold_sampled",
    MetadataSlots => "metadata_slots", S3Head => "s3_head", S3List => "s3_list",
    S3Headers => "s3_get_headers", S3Stream => "s3_get_stream", FirstChunk => "s3_first_chunk",
    Delivery => "chunk_delivery",
);
const ERRNOS: [&str; 11] = [
    "OK",
    "ENOENT",
    "EAGAIN",
    "ETIMEDOUT",
    "EINTR",
    "EIO",
    "EACCES",
    "ESTALE",
    "ENOMEM",
    "ENODATA",
    "OTHER",
];
fn errno_index(errno: i32) -> usize {
    match errno {
        0 => 0,
        libc::ENOENT => 1,
        libc::EAGAIN => 2,
        libc::ETIMEDOUT => 3,
        libc::EINTR => 4,
        libc::EIO => 5,
        libc::EACCES | libc::EPERM => 6,
        libc::ESTALE => 7,
        libc::ENOMEM => 8,
        libc::ENODATA => 9,
        _ => 10,
    }
}

#[derive(Debug)]
struct Timing {
    duration: Histogram,
    inflight: Gauge,
    outcomes: [Counter; 3],
}
#[derive(Debug)]
struct Operation {
    duration: Histogram,
    replies: [Counter; 11],
}
#[derive(Debug)]
struct Inner {
    origin: Instant,
    sequence: AtomicU64,
    stages: [Timing; STAGES],
    operations: [Operation; OPCODES],
    phase_histograms: Histograms,
    phase_inflight: Family<Labels, Gauge>,
    phase_outcomes: Family<Labels, Counter>,
    reply_histograms: Histograms,
    replies: Family<Labels, Counter>,
    executor: Histogram,
    post_reply: Histogram,
    events: Family<Labels, Counter>,
    attempts: Family<Labels, Counter>,
    retries: Family<Labels, Counter>,
    body_bytes: Counter,
    active: Mutex<BTreeMap<u64, Weak<Trace>>>,
    next_log_ns: [AtomicU64; 3],
    suppressed_logs: Counter,
    lock_samples: AtomicU64,
    log_level: DiagnosticLogLevel,
    log_writer: Option<NonBlocking>,
    _log_guard: Option<WorkerGuard>,
}

impl Inner {
    fn admit_log(&self, level: DiagnosticLogLevel) -> bool {
        // Debug bypasses rate sampling, but not the bounded lossy writer.
        if level == DiagnosticLogLevel::Debug {
            return true;
        }
        let next_log_ns = &self.next_log_ns[match level {
            DiagnosticLogLevel::Error => 0,
            DiagnosticLogLevel::Warn => 1,
            DiagnosticLogLevel::Info => 2,
            DiagnosticLogLevel::Off | DiagnosticLogLevel::Debug => unreachable!(),
        }];
        let now = self
            .origin
            .elapsed()
            .as_nanos()
            .min(u64::MAX as u128 - 1_000_000_000) as u64;
        let next = next_log_ns.load(Ordering::Relaxed);
        if now < next
            || next_log_ns
                .compare_exchange(
                    next,
                    now + 1_000_000_000,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            self.suppressed_logs.inc();
            return false;
        }
        true
    }
}

/// Cloneable shared observations for a filesystem and its real storage backend.
/// Disabled instances do not read the clock or allocate a request trace.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics(Option<Arc<Inner>>);

tokio::task_local! { static CURRENT: Option<Arc<Trace>>; }
pub(crate) fn context() -> Option<Arc<Trace>> {
    CURRENT.try_with(Clone::clone).ok().flatten()
}
pub(crate) async fn scope<T>(context: Option<Arc<Trace>>, future: impl Future<Output = T>) -> T {
    if context.is_some() {
        CURRENT.scope(context, future).await
    } else {
        future.await
    }
}

pub(crate) async fn observe<T>(
    diagnostics: &Diagnostics,
    stage: Stage,
    future: impl Future<Output = crate::error::Result<T>>,
) -> crate::error::Result<T> {
    let timer = diagnostics.stage(stage);
    let result = future.await;
    timer.finish(&result);
    result
}

pub(crate) fn error_reason(error: &crate::error::Error) -> &'static str {
    use crate::error::Error;
    match error {
        Error::Fs(_) => "filesystem",
        Error::Config(_) => "configuration",
        Error::Unsupported(_) => "unsupported",
        Error::Budget { .. } => "payload_budget",
        Error::Cancelled => "cancelled",
        Error::Timeout(_) => "timeout",
        Error::InvalidResponse(_) => "invalid_response",
        Error::Backend { code, .. } if code == "TransportOrProtocol" => {
            "backend_transport_or_protocol"
        }
        Error::Backend { .. } => "backend_response",
    }
}

impl Diagnostics {
    pub(crate) fn new(enabled: bool, log_level: DiagnosticLogLevel) -> Self {
        if !enabled {
            return Self::default();
        }
        let (log_writer, log_guard) = if log_level == DiagnosticLogLevel::Off {
            (None, None)
        } else {
            let (writer, guard) = NonBlockingBuilder::default()
                .buffered_lines_limit(if log_level == DiagnosticLogLevel::Debug {
                    256
                } else {
                    16
                })
                .lossy(true)
                .thread_name("geesefs-log")
                .finish(std::io::stderr());
            (Some(writer), Some(guard))
        };
        let phase_histograms = Histograms::new_with_constructor(histogram as fn() -> Histogram);
        let phase_inflight = Family::<Labels, Gauge>::default();
        let phase_outcomes = Family::<Labels, Counter>::default();
        let stages = Stage::ALL.map(|stage| Timing {
            duration: phase_histograms
                .get_or_create(&vec![("stage", stage.label())])
                .clone(),
            inflight: phase_inflight
                .get_or_create(&vec![("stage", stage.label())])
                .clone(),
            outcomes: ["ok", "error", "cancelled"].map(|outcome| {
                phase_outcomes
                    .get_or_create(&vec![("stage", stage.label()), ("outcome", outcome)])
                    .clone()
            }),
        });
        let reply_histograms = Histograms::new_with_constructor(histogram as fn() -> Histogram);
        let replies = Family::<Labels, Counter>::default();
        let operations = Opcode::ALL.map(|opcode| Operation {
            duration: reply_histograms
                .get_or_create(&vec![("opcode", opcode.label())])
                .clone(),
            replies: ERRNOS.map(|errno| {
                replies
                    .get_or_create(&vec![("opcode", opcode.label()), ("errno", errno)])
                    .clone()
            }),
        });
        Self(Some(Arc::new(Inner {
            origin: Instant::now(),
            sequence: AtomicU64::new(0),
            stages,
            operations,
            phase_histograms,
            phase_inflight,
            phase_outcomes,
            reply_histograms,
            replies,
            executor: histogram(),
            post_reply: histogram(),
            events: Family::default(),
            attempts: Family::default(),
            retries: Family::default(),
            body_bytes: Counter::default(),
            active: Mutex::new(BTreeMap::new()),
            next_log_ns: std::array::from_fn(|_| AtomicU64::new(0)),
            suppressed_logs: Counter::default(),
            lock_samples: AtomicU64::new(0),
            log_level,
            log_writer,
            _log_guard: log_guard,
        })))
    }
    pub(crate) fn enabled(&self) -> bool {
        self.0.is_some()
    }
    pub(crate) fn debug(&self, event: &'static str, details: impl FnOnce() -> serde_json::Value) {
        if self
            .0
            .as_ref()
            .is_some_and(|inner| inner.log_level == DiagnosticLogLevel::Debug)
            && let Some(trace) = context()
        {
            trace.debug(event, details);
        }
    }
    pub(crate) fn event(&self, reason: &'static str) {
        if let Some(inner) = &self.0 {
            inner.events.get_or_create(&vec![("reason", reason)]).inc();
            self.debug("diagnostic_event", || serde_json::json!({"reason": reason}));
        }
    }
    pub(crate) fn background_error(&self, error: &crate::error::Error) {
        let reason = match error {
            crate::error::Error::Timeout(_) => "read_ahead_timeout",
            crate::error::Error::Cancelled => "read_ahead_cancelled",
            _ => "read_ahead_error",
        };
        self.event(reason);
        if let Some(trace) = context() {
            trace.log(
                DiagnosticLogLevel::Error,
                "read_ahead_error",
                0,
                error_reason(error),
            );
        }
    }
    pub(crate) fn attempt(&self, operation: &'static str, transport: &'static str, retry: bool) {
        if let Some(inner) = &self.0 {
            let labels = vec![("operation", operation), ("transport", transport)];
            inner.attempts.get_or_create(&labels).inc();
            if retry {
                inner.retries.get_or_create(&labels).inc();
            }
            self.debug("s3_attempt", || serde_json::json!({"operation": operation, "transport": transport, "retry": retry}));
        }
    }
    pub(crate) fn body_bytes(&self, bytes: usize) {
        if let Some(inner) = &self.0 {
            inner.body_bytes.inc_by(bytes as u64);
            self.debug("s3_body_chunk", || serde_json::json!({"bytes": bytes}));
        }
    }
    pub(crate) fn stage(&self, stage: Stage) -> Timer<'_> {
        let active = self.0.as_ref().map(|inner| {
            inner.stages[stage as usize].inflight.inc();
            let context = context();
            let scope_id = context
                .as_ref()
                .filter(|_| inner.log_level == DiagnosticLogLevel::Debug)
                .map(|_| inner.sequence.fetch_add(1, Ordering::Relaxed));
            if let Some(trace) = &context {
                trace.phases[stage as usize].fetch_add(1, Ordering::Relaxed);
                trace.debug(
                    "phase_start",
                    || serde_json::json!({"stage": stage.label(), "scope_id": scope_id}),
                );
            }
            (Instant::now(), context, scope_id)
        });
        Timer {
            diagnostics: self,
            stage,
            active,
            outcome: 2,
            reason: "cancelled",
        }
    }
    pub(crate) fn request(
        &self,
        opcode: Opcode,
        request: u64,
        ino: u64,
        offset: u64,
        size: u64,
    ) -> Option<Arc<Trace>> {
        let inner = self.0.as_ref()?;
        let id = inner.sequence.fetch_add(1, Ordering::Relaxed);
        let trace = Arc::new(Trace {
            inner: inner.clone(),
            id,
            opcode,
            request,
            ino,
            offset,
            size,
            started: Instant::now(),
            replied: AtomicBool::new(false),
            reply_ns: AtomicU64::new(0),
            again_reason: Mutex::new(None),
            phases: std::array::from_fn(|_| AtomicU64::new(0)),
            elapsed_ns: std::array::from_fn(|_| AtomicU64::new(0)),
            failed_stages: AtomicU64::new(0),
            cancelled_stages: AtomicU64::new(0),
        });
        inner
            .active
            .lock()
            .unwrap()
            .insert(id, Arc::downgrade(&trace));
        trace.log(DiagnosticLogLevel::Debug, "fuse_request", 0, "started");
        Some(trace)
    }
    pub(crate) fn sampled_lock(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|inner| inner.lock_samples.fetch_add(1, Ordering::Relaxed) % 1024 == 0)
    }
    pub(crate) fn register(&self, registry: &mut Registry) {
        super::gauge(
            registry,
            "geesefs_diagnostics_enabled",
            "Detailed diagnostic observations enabled",
            u64::from(self.enabled()),
        );
        let Some(inner) = &self.0 else {
            return;
        };
        registry.register(
            "geesefs_stage_duration_seconds",
            "Phase wall time, including failed and cancelled scopes; phases can overlap",
            inner.phase_histograms.clone(),
        );
        registry.register(
            "geesefs_stage_inflight",
            "Live phase scopes, including read-ahead",
            inner.phase_inflight.clone(),
        );
        registry.register(
            "geesefs_stage_completions",
            "Phase outcomes; dropped futures are cancelled, not successful",
            inner.phase_outcomes.clone(),
        );
        registry.register("geesefs_fuse_reply_duration_seconds", "Callback entry to terminal reply invocation, excluding work after reply and kernel queueing", inner.reply_histograms.clone());
        registry.register(
            "geesefs_fuse_replies",
            "Terminal FUSE reply invocations with bounded errno groups",
            inner.replies.clone(),
        );
        registry.register(
            "geesefs_executor_queue_seconds",
            "Admitted FUSE spawn to first poll; excludes kernel queueing",
            inner.executor.clone(),
        );
        registry.register(
            "geesefs_read_post_reply_seconds",
            "READ lifetime after reply until job and owned child observations end",
            inner.post_reply.clone(),
        );
        registry.register(
            "geesefs_diagnostic_events",
            "Conditions at their source, not a count of final failed requests",
            inner.events.clone(),
        );
        registry.register(
            "geesefs_s3_attempts",
            "SDK and extension HTTP transport attempts; excludes presigning and body consumption, not TCP transmissions",
            inner.attempts.clone(),
        );
        registry.register(
            "geesefs_s3_retries",
            "Transmission attempts after the first in the same SDK execution or extension loop",
            inner.retries.clone(),
        );
        registry.register("geesefs_s3_body_bytes", "GET body bytes yielded by the SDK, including subsequently rejected or unused fragments; not TCP wire bytes", inner.body_bytes.clone());
        registry.register(
            "geesefs_diagnostic_logs_suppressed",
            "Error/warn/info records suppressed by their per-level one-per-second limiters; debug is unsampled",
            inner.suppressed_logs.clone(),
        );
        super::counter(
            registry,
            "geesefs_diagnostic_logs_dropped",
            "Records dropped by a full/disconnected output queue; excludes stderr I/O errors and abrupt exits",
            inner
                .log_writer
                .as_ref()
                .map_or(0, |writer| writer.error_counter().dropped_lines() as u64),
        );
        let inflight = Family::<Labels, Gauge>::default();
        let oldest = Family::<Labels, super::Observation>::default();
        for opcode in Opcode::ALL {
            for state in ["reply_pending", "after_reply"] {
                let labels = vec![("opcode", opcode.label()), ("state", state)];
                inflight.get_or_create(&labels).set(0);
                oldest.get_or_create(&labels).set(0.);
            }
        }
        // Clone strong owners under the telemetry-only lock, then release it
        // before dropping any owner (Trace::drop removes its own weak entry).
        let active: Vec<_> = {
            inner
                .active
                .lock()
                .unwrap()
                .values()
                .filter_map(Weak::upgrade)
                .collect()
        };
        let mut slowest: Option<&Arc<Trace>> = None;
        for trace in &active {
            let labels = vec![
                ("opcode", trace.opcode.label()),
                (
                    "state",
                    if trace.replied.load(Ordering::Relaxed) {
                        "after_reply"
                    } else {
                        "reply_pending"
                    },
                ),
            ];
            inflight.get_or_create(&labels).inc();
            let age = trace.age();
            let gauge = oldest.get_or_create(&labels);
            gauge.set(gauge.get().max(age));
            if age >= 30. && slowest.is_none_or(|previous| trace.started < previous.started) {
                slowest = Some(trace);
            }
        }
        if let Some(trace) = slowest {
            trace.log(DiagnosticLogLevel::Info, "fuse_inflight", 0, "unfinished");
        }
        registry.register(
            "geesefs_fuse_inflight",
            "Owned FUSE jobs split at reply publication, not just completed histograms",
            inflight,
        );
        registry.register(
            "geesefs_fuse_oldest_seconds",
            "Oldest callback age of each live job group; zero means no live job",
            oldest,
        );
    }
}

pub(crate) struct Timer<'a> {
    diagnostics: &'a Diagnostics,
    stage: Stage,
    active: Option<(Instant, Option<Arc<Trace>>, Option<u64>)>,
    outcome: usize,
    reason: &'static str,
}
impl Timer<'_> {
    pub(crate) fn finish<T>(mut self, result: &crate::error::Result<T>) {
        self.outcome = usize::from(result.is_err());
        self.reason = result.as_ref().err().map_or("ok", error_reason);
    }
    pub(crate) fn ok(mut self) {
        self.outcome = 0;
        self.reason = "ok";
    }
}
impl Drop for Timer<'_> {
    fn drop(&mut self) {
        let Some((started, trace, scope_id)) = self.active.take() else {
            return;
        };
        let elapsed = started.elapsed();
        let timing = &self.diagnostics.0.as_ref().unwrap().stages[self.stage as usize];
        timing.duration.observe(elapsed.as_secs_f64());
        timing.outcomes[self.outcome].inc();
        timing.inflight.dec();
        if let Some(trace) = trace {
            let outcomes = match self.outcome {
                1 => Some(&trace.failed_stages),
                2 => Some(&trace.cancelled_stages),
                _ => None,
            };
            if let Some(outcomes) = outcomes {
                outcomes.fetch_or(1 << self.stage as usize, Ordering::Relaxed);
            }
            trace.elapsed_ns[self.stage as usize].fetch_add(
                elapsed.as_nanos().min(u64::MAX as u128) as u64,
                Ordering::Relaxed,
            );
            trace.phases[self.stage as usize].fetch_sub(1, Ordering::Relaxed);
            trace.debug("phase_end", || serde_json::json!({"stage": self.stage.label(), "scope_id": scope_id,
                "duration_seconds": elapsed.as_secs_f64(), "outcome": (["ok", "error", "cancelled"][self.outcome]), "reason": self.reason}));
        }
    }
}

#[derive(Debug)]
pub(crate) struct Trace {
    inner: Arc<Inner>,
    id: u64,
    opcode: Opcode,
    request: u64,
    ino: u64,
    offset: u64,
    size: u64,
    started: Instant,
    replied: AtomicBool,
    reply_ns: AtomicU64,
    again_reason: Mutex<Option<&'static str>>,
    phases: [AtomicU64; STAGES],
    elapsed_ns: [AtomicU64; STAGES],
    failed_stages: AtomicU64,
    cancelled_stages: AtomicU64,
}
impl Trace {
    pub(crate) fn polled(&self, queued: Instant) {
        let seconds = queued.elapsed().as_secs_f64();
        self.inner.executor.observe(seconds);
        self.debug(
            "fuse_polled",
            || serde_json::json!({"executor_queue_seconds": seconds}),
        );
    }
    pub(crate) fn replied(&self) -> bool {
        self.replied.load(Ordering::Relaxed)
    }
    pub(crate) fn again(&self, reason: &'static str) {
        *self.again_reason.lock().unwrap() = Some(reason);
    }
    pub(crate) fn background_deadline(&self) {
        self.log(
            DiagnosticLogLevel::Error,
            "fuse_background_deadline",
            libc::ETIMEDOUT,
            "deadline_after_reply",
        );
    }
    pub(crate) fn reply(&self, errno: i32, reason: &'static str) {
        assert!(
            !self.replied.swap(true, Ordering::Relaxed),
            "one terminal reply owner"
        );
        let elapsed = self.started.elapsed();
        self.reply_ns.store(
            elapsed.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        let operation = &self.inner.operations[self.opcode as usize];
        operation.duration.observe(elapsed.as_secs_f64());
        operation.replies[errno_index(errno)].inc();
        let error = errno != 0 && errno != libc::ENOENT && errno != libc::ENODATA;
        let reason = if errno == libc::EAGAIN && reason == "filesystem" {
            self.again_reason.lock().unwrap().unwrap_or(reason)
        } else {
            reason
        };
        let level = if error {
            if matches!(errno, libc::EAGAIN | libc::EINTR | libc::ESTALE) {
                DiagnosticLogLevel::Warn
            } else {
                DiagnosticLogLevel::Error
            }
        } else if elapsed >= Duration::from_secs(1) {
            DiagnosticLogLevel::Info
        } else {
            DiagnosticLogLevel::Debug
        };
        self.log(level, "fuse_reply", errno, reason);
    }
    fn age(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    fn log(
        &self,
        level: DiagnosticLogLevel,
        event: &'static str,
        errno: i32,
        reason: &'static str,
    ) {
        self.record(level, event, errno, reason, None);
    }
    fn debug(&self, event: &'static str, details: impl FnOnce() -> serde_json::Value) {
        if self.inner.log_level == DiagnosticLogLevel::Debug {
            self.record(
                DiagnosticLogLevel::Debug,
                event,
                0,
                "trace",
                Some(details()),
            );
        }
    }
    fn record(
        &self,
        level: DiagnosticLogLevel,
        event: &'static str,
        errno: i32,
        reason: &'static str,
        details: Option<serde_json::Value>,
    ) {
        if self.inner.log_level < level {
            return;
        }
        let Some(writer) = &self.inner.log_writer else {
            return;
        };
        if !self.inner.admit_log(level) {
            return;
        }
        let phases: BTreeMap<_, _> = Stage::ALL.into_iter().filter_map(|stage| {
            let active = self.phases[stage as usize].load(Ordering::Relaxed);
            let elapsed_ns = self.elapsed_ns[stage as usize].load(Ordering::Relaxed);
            let failed = self.failed_stages.load(Ordering::Relaxed) & (1 << stage as usize) != 0;
            let cancelled = self.cancelled_stages.load(Ordering::Relaxed) & (1 << stage as usize) != 0;
            (active != 0 || elapsed_ns != 0 || failed || cancelled).then_some((stage.label(), serde_json::json!({"active": active, "completed_seconds": elapsed_ns as f64 / 1e9, "failed": failed, "cancelled": cancelled})))
        }).collect();
        let record = format!(
            "{}\n",
            serde_json::json!({"level": level, "event": event, "trace_id": self.id, "process_id": std::process::id(),
                "timestamp_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|duration| duration.as_secs_f64()),
                "opcode": self.opcode.label(), "request": self.request, "ino": self.ino, "offset": self.offset, "size": self.size, "errno": errno, "reason": reason, "elapsed_seconds": self.age(), "reply_sent": self.replied(), "phases": phases, "details": details})
        );
        // Lossy mode queues this entire record once; stderr backpressure stays
        // on the library's worker, with bounded memory and observable drops.
        let _ = writer.clone().write_all(record.as_bytes());
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        self.log(DiagnosticLogLevel::Debug, "fuse_complete", 0, "completed");
        if matches!(self.opcode, Opcode::Read) && self.replied() {
            self.inner.post_reply.observe(
                (self.started.elapsed().as_secs_f64()
                    - self.reply_ns.load(Ordering::Relaxed) as f64 / 1e9)
                    .max(0.),
            );
        }
        self.inner.active.lock().unwrap().remove(&self.id);
    }
}

/// Sample existing critical sections; no replacement mutex or lock ordering.
pub(crate) struct NamespaceGuard<'a, T> {
    guard: std::sync::MutexGuard<'a, T>,
    _hold: Option<Timer<'a>>,
}
pub(crate) fn namespace_lock<'a, T>(
    mutex: &'a Mutex<T>,
    diagnostics: &'a Diagnostics,
) -> NamespaceGuard<'a, T> {
    let sampled = diagnostics.sampled_lock();
    let wait = sampled.then(|| diagnostics.stage(Stage::NamespaceWait));
    let guard = mutex.lock().unwrap();
    if let Some(wait) = wait {
        wait.ok();
    }
    NamespaceGuard {
        guard,
        _hold: sampled.then(|| diagnostics.stage(Stage::NamespaceHold)),
    }
}
impl<T> std::ops::Deref for NamespaceGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}
impl<T> std::ops::DerefMut for NamespaceGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}
impl<T> Drop for NamespaceGuard<'_, T> {
    fn drop(&mut self) {
        if let Some(hold) = self._hold.take() {
            hold.ok();
        }
    }
}
