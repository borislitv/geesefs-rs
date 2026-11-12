//! fuser owns the wire protocol and exactly-once replies. Admission and borrowed
//! write ownership are acquired before detached work, never in an unbounded queue.
use crate::{
    error::Error,
    filesystem::{Filesystem as Core, OpenFile},
    metrics::{self, Opcode, Stage, Trace},
    write::MAX_IO,
};
use fuser::*;
use futures_util::{StreamExt, stream::FuturesOrdered};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    future::Future,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::SystemTime,
};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};

pub fn errno(error: Error) -> Errno {
    match error {
        Error::Fs(code) => Errno::from_i32(code),
        Error::Config(_) => Errno::EINVAL,
        Error::Unsupported(_) => Errno::EOPNOTSUPP,
        Error::Budget { .. } => Errno::ENOMEM,
        Error::Timeout(_) => Errno::ETIMEDOUT,
        Error::Cancelled => Errno::EINTR,
        Error::Backend { code, .. } => backend_errno(&code),
        Error::InvalidResponse(_) => Errno::EIO,
    }
}

fn backend_errno(code: &str) -> Errno {
    match code {
        "NoSuchKey" | "NotFound" | "404" => Errno::ENOENT,
        "AccessDenied" | "InvalidAccessKeyId" | "SignatureDoesNotMatch" => Errno::EACCES,
        "PreconditionFailed" => Errno::ESTALE,
        _ => Errno::EIO,
    }
}

trait Failure: Send + 'static {
    fn fail(self, error: Errno);
}
macro_rules! failures{($($reply:ty),*)=>{$(impl Failure for $reply{fn fail(self,error:Errno){self.error(error);}})*};}
failures!(
    ReplyEntry,
    ReplyAttr,
    ReplyData,
    ReplyWrite,
    ReplyOpen,
    ReplyEmpty,
    ReplyCreate,
    ReplyDirectory,
    DirectoryReply,
    ReplyXattr
);

/// The job exclusively owns an unsent reply. Deadline cancellation must send
/// ETIMEDOUT instead of fuser's generic Drop fallback (EIO).
/// Taking the reply before publication also makes early READ replies terminal:
/// subsequent read-ahead cancellation owns no reply and cannot send a second one.
struct DeadlineReply<R: Failure> {
    reply: Option<R>,
    expired: Option<Arc<AtomicBool>>,
    trace: Option<Arc<Trace>>,
}

impl<R: Failure> DeadlineReply<R> {
    fn finish(mut self, send: impl FnOnce(R)) {
        send(self.reply.take().expect("reply is owned until publication"));
        if let Some(trace) = &self.trace {
            trace.reply(0, "ok");
        }
    }

    fn error(self, error: Errno) {
        self.error_reason(error, "filesystem");
    }

    fn failure(self, error: Error) {
        let reason = metrics::error_reason(&error);
        self.error_reason(errno(error), reason);
    }

    fn error_reason(mut self, error: Errno, reason: &'static str) {
        self.reply
            .take()
            .expect("reply is owned until publication")
            .fail(error);
        if let Some(trace) = &self.trace {
            trace.reply(error.code(), reason);
        }
    }
}

impl<R: Failure> Drop for DeadlineReply<R> {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            let expired = self
                .expired
                .as_ref()
                .is_some_and(|expired| expired.load(Ordering::Relaxed));
            let (error, reason) = if expired {
                (Errno::ETIMEDOUT, "deadline")
            } else {
                (Errno::EIO, "abandoned")
            };
            reply.fail(error);
            if let Some(trace) = &self.trace {
                trace.reply(error.code(), reason);
            }
        }
        // Unexpected destruction preserves fuser's EIO fallback and observes it.
    }
}

struct Admission {
    closing: bool,
    semaphore: Arc<Semaphore>,
}
struct File {
    file: Arc<OpenFile>,
    _permit: OwnedSemaphorePermit,
}
struct Directory {
    _permit: OwnedSemaphorePermit,
    _metadata_charge: crate::metadata_budget::MetadataCharge,
    last: crate::metadata_budget::MetadataString,
    page_after: crate::metadata_budget::MetadataString,
    cookie: u64,
}

impl Directory {
    fn seek(&mut self, offset: u64) {
        if offset != self.cookie {
            self.last = Default::default();
            self.page_after = Default::default();
            self.cookie = 0;
        }
    }

    async fn dots(
        &mut self,
        fs: &Core,
        ino: INodeNo,
        offset: u64,
        reply: &mut DirectoryReply,
    ) -> crate::error::Result<bool> {
        if self.cookie == 0 {
            if offset == 0
                && reply
                    .add(fs, ino, 1, FileType::Directory, ".", None)
                    .await?
            {
                return Ok(true);
            }
            self.cookie = 1;
        }
        if self.cookie == 1 {
            if offset <= 1
                && reply
                    .add(
                        fs,
                        INodeNo(fs.parent_ino(ino.0)),
                        2,
                        FileType::Directory,
                        "..",
                        None,
                    )
                    .await?
            {
                return Ok(true);
            }
            self.cookie = 2;
        }
        Ok(false)
    }

    /// Return whether this reply has room to continue into another backend page.
    async fn page(
        &mut self,
        fs: &Core,
        ino: INodeNo,
        offset: u64,
        limit: usize,
        reply: &mut DirectoryReply,
    ) -> crate::error::Result<bool> {
        let page = fs
            .directory_remainder(ino.0, &self.page_after, &self.last, limit)
            .await?;
        let Some(next_after) = page.next_after else {
            return Ok(false);
        };
        let stopped = if let DirectoryReply::Basic(reply, charges) = reply {
            self.basic_entries(fs, offset, page.entries, reply, charges)?
        } else {
            self.plus_entries(fs, offset, page.entries, reply).await?
        };
        if !stopped {
            // An exhausted cached remainder only advances the backend-page
            // cursor. It must never publish an empty wire reply as false EOF.
            self.page_after = next_after;
        }
        Ok(!stopped)
    }

    fn basic_entries(
        &mut self,
        fs: &Core,
        offset: u64,
        entries: Vec<crate::filesystem::DirEntry>,
        reply: &mut ReplyDirectory,
        charges: &mut Vec<crate::metadata_budget::MetadataCharge>,
    ) -> crate::error::Result<bool> {
        for entry in entries {
            let charge = if self.cookie >= offset {
                DirectoryReply::entry_charge(fs, &entry.name)?
            } else {
                None
            };
            let next = self.cookie + 1;
            if self.cookie >= offset
                && reply.add(INodeNo(entry.ino), next, entry.kind, entry.name.as_str())
            {
                return Ok(true);
            }
            if let Some(charge) = charge {
                charges.push(charge);
            }
            self.cookie = next;
            self.last = entry.name;
        }
        Ok(false)
    }

    async fn plus_entries(
        &mut self,
        fs: &Core,
        offset: u64,
        entries: Vec<crate::filesystem::DirEntry>,
        reply: &mut DirectoryReply,
    ) -> crate::error::Result<bool> {
        let mut pending = entries.into_iter();
        let mut next = pending.next();
        let mut jobs = FuturesOrdered::new();
        while next.is_some() || !jobs.is_empty() {
            let ready = tokio::select! {
                ready = jobs.next(), if !jobs.is_empty() => ready,
                prepared = async {
                    fs.prepare_inode_refresh(next.as_ref().expect("pending entry").ino).await
                }, if next.is_some() && jobs.len() < fs.config.max_requests => {
                    let prepared = prepared?;
                    let entry = next.take().expect("admitted entry");
                    jobs.push_back(async move { Ok::<_, Error>((entry, prepared.await?)) });
                    next = pending.next();
                    continue;
                }
            };
            let (entry, prepared) = ready.expect("admitted metadata job")?;
            let next = self.cookie + 1;
            if self.cookie >= offset
                && reply
                    .add(
                        fs,
                        INodeNo(entry.ino),
                        next,
                        entry.kind,
                        entry.name.as_str(),
                        Some(prepared),
                    )
                    .await?
            {
                return Ok(true);
            }
            self.cookie = next;
            self.last = entry.name;
            // Publish a nonempty prefix while kernel owners are near the bound.
            // A related notification may need the parent lock until this reply.
            if fs.namespace_pressure() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

enum DirectoryReply {
    Basic(ReplyDirectory, Vec<crate::metadata_budget::MetadataCharge>),
    Plus(
        ReplyDirectoryPlus,
        Vec<Arc<crate::inode::Inode>>,
        Vec<crate::metadata_budget::MetadataCharge>,
    ),
}

impl DirectoryReply {
    fn entry_charge(
        fs: &Core,
        name: &str,
    ) -> crate::error::Result<Option<crate::metadata_budget::MetadataCharge>> {
        // Includes aligned wire record, plus attributes, lookup owner and
        // geometrically grown bookkeeping vectors until the reply is sent.
        if fs.metadata_enabled() {
            fs.charge_metadata(2 * name.len() + 1024).map(Some)
        } else {
            Ok(None)
        }
    }
    async fn add(
        &mut self,
        fs: &Core,
        ino: INodeNo,
        offset: u64,
        kind: FileType,
        name: &str,
        prepared: Option<Arc<crate::inode::Inode>>,
    ) -> crate::error::Result<bool> {
        match self {
            Self::Basic(reply, charges) => {
                let charge = Self::entry_charge(fs, name)?;
                let full = reply.add(ino, offset, kind, name);
                if !full && let Some(charge) = charge {
                    charges.push(charge);
                }
                Ok(full)
            }
            Self::Plus(reply, entries, charges) => {
                let charge = Self::entry_charge(fs, name)?;
                let node = match prepared {
                    Some(node) => node,
                    None => fs.refresh_inode_if_expired(ino.0).await?,
                };
                let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                let full = reply.add(ino, offset, name, &ttl, &attr, Generation(0));
                if !full && let Some(charge) = charge {
                    charges.push(charge);
                }
                // Linux does not acquire lookup references for dot entries.
                // Hold accepted inodes, but transfer references only when the
                // complete buffer is sent, not on error or cancellation.
                if !full && name != "." && name != ".." {
                    entries.push(node);
                }
                Ok(full)
            }
        }
    }

    fn ok(self, fs: &Core) {
        match self {
            Self::Basic(reply, _charges) => reply.ok(),
            Self::Plus(reply, entries, _charges) => {
                for node in &entries {
                    fs.reference(node);
                }
                reply.ok();
            }
        }
    }

    fn error(self, error: Errno) {
        match self {
            Self::Basic(reply, _charges) => reply.error(error),
            Self::Plus(reply, _, _charges) => reply.error(error),
        }
    }
}

pub struct Fuse {
    fs: Arc<Core>,
    runtime: Handle,
    admission: Mutex<Admission>,
    directories: Arc<Mutex<BTreeMap<u64, Arc<tokio::sync::Mutex<Directory>>>>>,
    files: Arc<Mutex<BTreeMap<u64, File>>>,
    handle_slots: Arc<Semaphore>,
    cache_retirement: Arc<Semaphore>,
    namespace_retirement_scan: Arc<Mutex<crate::namespace::ReclaimScan>>,
    next_handle: Arc<AtomicU64>,
    pub notifier: Arc<Mutex<Option<Notifier>>>,
    pub shutdown_error: Arc<Mutex<Option<String>>>,
}

impl Fuse {
    pub fn new(fs: Arc<Core>, runtime: Handle) -> Self {
        let slots = fs.config.max_requests;
        let handle_slots = Arc::new(Semaphore::new(fs.config.entry_limit as usize));
        Self {
            fs,
            runtime,
            admission: Mutex::new(Admission {
                closing: false,
                semaphore: Arc::new(Semaphore::new(slots)),
            }),
            directories: Arc::new(Mutex::new(BTreeMap::new())),
            files: Arc::new(Mutex::new(BTreeMap::new())),
            handle_slots,
            cache_retirement: Arc::new(Semaphore::new(1)),
            namespace_retirement_scan: Arc::new(Mutex::new(Default::default())),
            next_handle: Arc::new(AtomicU64::new(1)),
            notifier: Arc::new(Mutex::new(None)),
            shutdown_error: Arc::new(Mutex::new(None)),
        }
    }
    fn track<R: Failure>(
        &self,
        opcode: Opcode,
        request: &Request,
        ino: INodeNo,
        range: (u64, u64),
        reply: R,
    ) -> DeadlineReply<R> {
        DeadlineReply {
            reply: Some(reply),
            expired: None,
            trace: self
                .fs
                .diagnostics
                .request(opcode, request.unique().0, ino.0, range.0, range.1),
        }
    }

    fn spawn<R, F, T>(&self, reply: R, job: F)
    where
        R: Failure,
        F: FnOnce(Arc<Core>, DeadlineReply<R>) -> T + Send + 'static,
        T: Future<Output = ()> + Send + 'static,
    {
        self.spawn_traced(
            DeadlineReply {
                reply: Some(reply),
                expired: None,
                trace: self.fs.diagnostics.request(Opcode::Mutation, 0, 0, 0, 0),
            },
            job,
        );
    }

    fn spawn_traced<R, F, T>(&self, reply: DeadlineReply<R>, job: F)
    where
        R: Failure,
        F: FnOnce(Arc<Core>, DeadlineReply<R>) -> T + Send + 'static,
        T: Future<Output = ()> + Send + 'static,
    {
        let deadline = {
            // fuser creates its own receiver threads. Use the clock belonging
            // to the executor that will enforce both admission and job expiry.
            let _entered = self.runtime.enter();
            Instant::now() + self.fs.config.request_timeout
        };
        let permit = match self.admit(deadline, reply.trace.clone()) {
            Ok(permit) => permit,
            Err(error) => {
                let reason = if matches!(error, Error::Timeout(_)) {
                    "admission_timeout"
                } else {
                    "closing"
                };
                self.fs.diagnostics.event(reason);
                reply.error_reason(errno(error), reason);
                return;
            }
        };
        let fs = self.fs.clone();
        let context = reply.trace.clone();
        let queued = self.fs.diagnostics.enabled().then(std::time::Instant::now);
        self.runtime
            .spawn(metrics::scope(context.clone(), async move {
                let _permit = permit;
                if let (Some(trace), Some(queued)) = (&context, queued) {
                    trace.polled(queued);
                }
                Self::run_job(fs, reply, job, deadline).await;
            }));
    }

    async fn run_job<R, F, T>(fs: Arc<Core>, mut reply: DeadlineReply<R>, job: F, deadline: Instant)
    where
        R: Failure,
        F: FnOnce(Arc<Core>, DeadlineReply<R>) -> T,
        T: Future<Output = ()>,
    {
        // Tokio timeouts poll the inner future first. An already-expired
        // callback must not start even an immediately-ready mutation.
        if Instant::now() >= deadline {
            fs.diagnostics.event("deadline_before_reply");
            reply.error_reason(Errno::ETIMEDOUT, "deadline");
            return;
        }
        let context = reply.trace.clone();
        let diagnostics = fs.diagnostics.clone();
        let expired = Arc::new(AtomicBool::new(false));
        let mut after_reply = false;
        let timed_out = {
            reply.expired = Some(expired.clone());
            let future = job(fs, reply);
            tokio::pin!(future);
            // Timeout borrows the job, so mark expiry BEFORE dropping its
            // reply/backend work. No detached operation or second reply owner.
            let timed_out = tokio::time::timeout_at(deadline, &mut future)
                .await
                .is_err();
            if timed_out {
                after_reply = context.as_ref().is_some_and(|trace| trace.replied());
                expired.store(true, Ordering::Relaxed);
            }
            timed_out
        };
        // Destroy the job (and send any deadline reply) before logging.
        if timed_out {
            diagnostics.event(if after_reply {
                "deadline_after_reply"
            } else {
                "deadline_before_reply"
            });
            if after_reply && let Some(trace) = &context {
                trace.background_deadline();
            }
            if !diagnostics.enabled() {
                eprintln!("FUSE request deadline exceeded");
            }
        }
    }

    fn admit(
        &self,
        deadline: Instant,
        context: Option<Arc<Trace>>,
    ) -> crate::error::Result<OwnedSemaphorePermit> {
        let semaphore = {
            let admission = self.admission.lock().unwrap();
            if admission.closing {
                return Err(Error::Cancelled);
            }
            if let Ok(permit) = admission.semaphore.clone().try_acquire_owned() {
                return Ok(permit);
            }
            admission.semaphore.clone()
        };
        // Fixed fuser receiver threads provide backpressure to the kernel: at
        // most one pending callback (including funded WRITE bytes) per receiver.
        // Do not spawn semaphore waiters or hold the admission mutex here. The
        // async workers remain free to finish the actual owners, including work
        // after an early READ reply; notification work uses a separate slot.
        self.fs.diagnostics.event("admission_wait");
        let permit = self.runtime.block_on(metrics::scope(context, async {
            metrics::observe(&self.fs.diagnostics, Stage::FuseAdmission, async {
                tokio::time::timeout_at(deadline, semaphore.acquire_owned())
                    .await
                    .map_err(|_| Error::Timeout("FUSE admission"))?
                    .map_err(|_| Error::Cancelled)
            })
            .await
        }))?;
        if self.admission.lock().unwrap().closing {
            return Err(Error::Cancelled);
        }
        Ok(permit)
    }
    fn flags(&self) -> FopenFlags {
        if self.fs.config.read_only_direct_io {
            FopenFlags::FOPEN_DIRECT_IO
        } else {
            FopenFlags::empty()
        }
    }

    fn reserve_handle(&self) -> crate::error::Result<OwnedSemaphorePermit> {
        self.handle_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Fs(libc::EMFILE))
    }

    fn allocate(
        &self,
        ino: INodeNo,
        offset: u64,
        length: u64,
        mode: i32,
    ) -> crate::error::Result<()> {
        if mode != 0 && mode != libc::FALLOC_FL_KEEP_SIZE {
            return Err(Error::Fs(libc::EOPNOTSUPP));
        }
        self.fs.check_write()?;
        let end = offset.checked_add(length).ok_or(Error::Fs(libc::EFBIG))?;
        let size = self.fs.node(ino.0)?.attr().size;
        if mode == 0 && end > size {
            self.fs.truncate(ino.0, end)?;
        }
        Ok(())
    }

    fn retire_file_cache(&self, ino: INodeNo) {
        let Ok(node) = self.fs.node(ino.0) else {
            return;
        };
        if !closed_clean_file(&node) {
            return;
        }
        let Some(notifier) = self.notifier.lock().unwrap().clone() else {
            return;
        };
        let permit = {
            let admission = self.admission.lock().unwrap();
            if admission.closing {
                return;
            }
            let Ok(permit) = self.cache_retirement.clone().try_acquire_owned() else {
                return;
            };
            permit
        };
        // KEEP_CACHE is unset: these pages cannot survive the next open. Release
        // them once no file/mmap owns them. Kernel notifications can block on
        // writeback, so keep them off both the session and async worker threads.
        // One separate slot bounds notifications without consuming the request
        // admission needed by any kernel writeback; shutdown joins this owner.
        self.runtime.spawn_blocking(move || {
            let _permit = permit;
            if closed_clean_file(&node)
                && let Err(error) = notifier.inval_inode(ino, 0, 0)
            {
                eprintln!(
                    "retiring closed inode {} kernel cache failed: {error}",
                    ino.0
                );
            }
        });
    }

    fn retire_namespace_cache(&self) {
        // Reserve headroom for concurrently admitted backend pages. This does
        // not release application inodes: their kernel references still own
        // them until FORGET, and active wire pages retain their own Arc pins.
        if !self.fs.namespace_pressure() {
            return;
        }
        let Some(notifier) = self.notifier.lock().unwrap().clone() else {
            return;
        };
        let permit = {
            let admission = self.admission.lock().unwrap();
            if admission.closing {
                return;
            }
            let Ok(permit) = self.cache_retirement.clone().try_acquire_owned() else {
                return;
            };
            permit
        };
        let fs = self.fs.clone();
        let scan = self.namespace_retirement_scan.clone();
        self.runtime.spawn_blocking(move || {
            let _permit = permit;
            let Ok(charge) = fs.charge_metadata(
                128 * std::mem::size_of::<(u64, crate::metadata_budget::SharedKey)>(),
            ) else {
                return;
            };
            let entries = fs
                .namespace
                .lock()
                .unwrap()
                .kernel_retirement_batch(&mut scan.lock().unwrap());
            let entries = crate::metadata_budget::MetadataOwner::new(entries, charge);
            // Never wait for notifications from a related request or hold an
            // application lock: inval_entry can wait for a kernel parent lock.
            for (parent, key) in entries.iter() {
                let name = key.rsplit('/').next().expect("a key has a final component");
                let result = notifier.inval_entry(INodeNo(*parent), OsStr::new(name));
                if let Err(error) = result {
                    eprintln!("retiring namespace kernel entry failed: {error}");
                    break;
                }
            }
        });
    }

    fn remove_entry(&self, parent: INodeNo, name: &OsStr, directory: bool, reply: ReplyEmpty) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        self.spawn(reply, move |fs, reply| async move {
            match fs.unlink(parent.0, &name, directory).await {
                Ok(()) => reply.finish(ReplyEmpty::ok),
                Err(error) => reply.failure(error),
            }
        });
    }

    fn sync_file(&self, ino: INodeNo, reply: DeadlineReply<ReplyEmpty>) {
        self.spawn_traced(reply, move |fs, reply| async move {
            match fs.fsync(ino.0).await {
                Ok(()) => reply.finish(ReplyEmpty::ok),
                Err(error) => reply.failure(error),
            }
        });
    }

    fn read_directory(
        &self,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: DeadlineReply<DirectoryReply>,
    ) {
        self.retire_namespace_cache();
        let cursor = match self.directories.lock().unwrap().get(&fh.0).cloned() {
            Some(cursor) => cursor,
            None => {
                reply.error(Errno::EBADF);
                return;
            }
        };
        self.spawn_traced(reply, move |fs, mut reply| async move {
            let result = async {
                let reply = reply
                    .reply
                    .as_mut()
                    .expect("directory reply is not yet sent");
                let mut cursor = cursor.lock().await;
                cursor.seek(offset);
                if !cursor.dots(&fs, ino, offset, reply).await? {
                    // The cursor resumes a backend page across smaller kernel
                    // frames; truncating it here causes overlapping S3 LISTs.
                    // Small namespace bounds must still leave room for the
                    // next page while kernel notifications finish between replies.
                    let limit = ((fs.config.entry_limit as usize).saturating_sub(1) / 4)
                        .clamp(1, crate::backend::extended_list::MAX_PAGE_ENTRIES);
                    while cursor.page(&fs, ino, offset, limit, reply).await? {}
                }
                Ok::<(), Error>(())
            }
            .await;
            match result {
                Ok(()) => reply.finish(|reply| reply.ok(&fs)),
                Err(error) => reply.failure(error),
            }
        });
    }
}

fn closed_clean_file(node: &crate::inode::Inode) -> bool {
    let state = node.state.lock().unwrap();
    !state.deleted
        && state.opens == 0
        && state.version == state.durable
        && state.mode & libc::S_IFMT == libc::S_IFREG
}

fn file_handle(
    fs: &Arc<Core>,
    files: &Mutex<BTreeMap<u64, File>>,
    next: &AtomicU64,
    ino: u64,
    writable: bool,
    permit: OwnedSemaphorePermit,
) -> crate::error::Result<FileHandle> {
    let file = fs.open_file(ino, writable)?;
    let handle = next.fetch_add(1, Ordering::Relaxed);
    files.lock().unwrap().insert(
        handle,
        File {
            file,
            _permit: permit,
        },
    );
    Ok(FileHandle(handle))
}

fn text(value: &OsStr) -> std::result::Result<String, Errno> {
    value.to_str().map(str::to_owned).ok_or(Errno::EILSEQ)
}
fn xattr_reply(reply: ReplyXattr, size: u32, data: &[u8]) {
    if size == 0 {
        reply.size(data.len() as u32);
    } else if data.len() > size as usize {
        reply.error(Errno::ERANGE);
    } else {
        reply.data(data);
    }
}

impl fuser::Filesystem for Fuse {
    fn init(&mut self, _: &Request, config: &mut KernelConfig) -> std::io::Result<()> {
        config
            .set_max_write(MAX_IO as u32)
            .map_err(|_| std::io::Error::other("kernel does not accept bounded max_write"))?;
        let plus = InitFlags::FUSE_DO_READDIRPLUS;
        if config
            .capabilities()
            .contains(InitFlags::FUSE_DO_READDIRPLUS)
        {
            config
                .add_capabilities(config.capabilities() & plus)
                .map_err(|_| {
                    std::io::Error::other("kernel rejected advertised READDIRPLUS support")
                })?;
        }
        Ok(())
    }
    fn destroy(&mut self) {
        let semaphore = {
            let mut a = self.admission.lock().unwrap();
            a.closing = true;
            a.semaphore.clone()
        };
        let result = self.runtime.block_on(async {
            tokio::time::timeout(self.fs.config.shutdown_timeout, async {
                let _all = semaphore
                    .acquire_many(self.fs.config.max_requests as u32)
                    .await
                    .map_err(|_| Error::Cancelled)?;
                let _retirement = self
                    .cache_retirement
                    .acquire()
                    .await
                    .map_err(|_| Error::Cancelled)?;
                self.files.lock().unwrap().clear();
                self.fs.shutdown().await
            })
            .await
        });
        let result = result.unwrap_or(Err(Error::Timeout("FUSE shutdown")));
        if let Err(error) = result {
            *self.shutdown_error.lock().unwrap() = Some(error.to_string());
        }
    }
    fn lookup(&self, request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        self.retire_namespace_cache();
        let reply = self.track(Opcode::Lookup, request, parent, (0, 0), reply);
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        self.spawn_traced(reply, move |fs, reply| async move {
            match fs.lookup(parent.0, &name).await {
                Ok(node) => {
                    fs.reference(&node);
                    let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                    reply.finish(|reply| reply.entry(&ttl, &attr, Generation(0)));
                }
                Err(error) => reply.failure(error),
            }
        });
    }
    fn forget(&self, _: &Request, ino: INodeNo, count: u64) {
        self.fs.forget(ino.0, count);
    }
    fn getattr(&self, request: &Request, ino: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        let reply = self.track(Opcode::Getattr, request, ino, (0, 0), reply);
        self.spawn_traced(reply, move |fs, reply| async move {
            match fs.refresh_inode_if_expired(ino.0).await {
                Ok(node) => {
                    let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                    reply.finish(|reply| reply.attr(&ttl, &attr));
                }
                Err(error) => reply.failure(error),
            }
        });
    }
    fn setattr(
        &self,
        _: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _: Option<SystemTime>,
        _: Option<FileHandle>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let mtime = mtime.map(|time| match time {
            TimeOrNow::Now => SystemTime::now(),
            TimeOrNow::SpecificTime(time) => time,
        });
        let result = (|| {
            if let Some(size) = size {
                self.fs.truncate(ino.0, size)?;
            }
            self.fs.setattr(ino.0, mode, uid, gid, mtime)
        })();
        match result {
            Ok(attr) => reply.attr(&self.fs.config.stat_cache_ttl, &attr),
            Err(error) => reply.error(errno(error)),
        }
    }
    fn open(&self, request: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let reply = self.track(Opcode::Open, request, ino, (0, 0), reply);
        let result = self.reserve_handle().and_then(|permit| {
            file_handle(
                &self.fs,
                &self.files,
                &self.next_handle,
                ino.0,
                flags.0 & libc::O_ACCMODE != libc::O_RDONLY,
                permit,
            )
        });
        match result {
            Ok(handle) => reply.finish(|reply| reply.opened(handle, self.flags())),
            Err(error) => reply.failure(error),
        }
    }
    fn read(
        &self,
        request: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let reply = self.track(Opcode::Read, request, ino, (offset, u64::from(size)), reply);
        let file = match self
            .files
            .lock()
            .unwrap()
            .get(&fh.0)
            .map(|entry| entry.file.clone())
        {
            Some(file) if file.ino() == ino.0 => file,
            _ => {
                reply.error(Errno::EBADF);
                return;
            }
        };
        self.spawn_traced(reply, move |fs, reply| async move {
            if let Some(error) = file
                .read_reply_data(offset, size as usize, move |result| match result {
                    Ok(data) => reply.finish(|reply| reply.data(data)),
                    Err(error) => reply.failure(error),
                })
                .await
            {
                fs.diagnostics.background_error(&error);
                if !fs.diagnostics.enabled() {
                    eprintln!("read-ahead failed after reply: {error}");
                }
            }
        });
    }
    fn write(
        &self,
        request: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        data: &[u8],
        _: WriteFlags,
        flags: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let reply = self.track(
            Opcode::Write,
            request,
            ino,
            (offset, data.len() as u64),
            reply,
        );
        if data.len() > MAX_IO {
            reply.error(Errno::EINVAL);
            return;
        }
        let owned = match self
            .fs
            .ingress
            .try_reserve(data.len())
            .and_then(|r| r.copy(data))
        {
            Ok(data) => data,
            Err(error) => {
                reply.failure(error);
                return;
            }
        };
        self.spawn_traced(reply, move |fs, reply| async move {
            let result = async {
                let written = fs.write(ino.0, offset, owned.as_slice()).await?;
                if flags.0 & (libc::O_SYNC | libc::O_DSYNC) != 0 {
                    fs.fsync(ino.0).await?;
                }
                Ok::<_, Error>(written)
            }
            .await;
            match result {
                Ok(written) => reply.finish(|reply| reply.written(written as u32)),
                Err(error) => reply.failure(error),
            }
        });
    }
    fn create(
        &self,
        request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        open_flags: i32,
        reply: ReplyCreate,
    ) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let flags = self.flags();
        let identity = (request.uid(), request.gid());
        // Reserve capacity before the async create can mutate the namespace.
        // Cancellation or an error returns the permit without a registry entry.
        let permit = match self.reserve_handle() {
            Ok(permit) => permit,
            Err(error) => {
                reply.error(errno(error));
                return;
            }
        };
        let files = self.files.clone();
        let next_handle = self.next_handle.clone();
        self.spawn(reply, move |fs, reply| async move {
            match fs
                .create_with_identity(
                    parent.0,
                    &name,
                    libc::S_IFREG | (mode & !umask & 0o7777),
                    None,
                    0,
                    identity,
                )
                .await
            {
                Ok(node) => match file_handle(
                    &fs,
                    &files,
                    &next_handle,
                    node.ino,
                    open_flags & libc::O_ACCMODE != libc::O_RDONLY,
                    permit,
                ) {
                    Ok(handle) => {
                        fs.reference(&node);
                        let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                        reply.finish(|reply| {
                            reply.created(&ttl, &attr, Generation(0), handle, flags)
                        });
                    }
                    Err(error) => reply.failure(error),
                },
                Err(error) => reply.failure(error),
            }
        });
    }
    fn mkdir(
        &self,
        request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let identity = (request.uid(), request.gid());
        self.spawn(reply, move |fs, reply| async move {
            let result = async {
                let node = fs
                    .create_with_identity(
                        parent.0,
                        &name,
                        libc::S_IFDIR | (mode & !umask & 0o7777),
                        None,
                        0,
                        identity,
                    )
                    .await?;
                fs.fsync(node.ino).await?;
                Ok::<_, Error>(node)
            }
            .await;
            match result {
                Ok(node) => {
                    fs.reference(&node);
                    let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                    reply.finish(|reply| reply.entry(&ttl, &attr, Generation(0)));
                }
                Err(error) => reply.failure(error),
            }
        });
    }
    fn mknod(
        &self,
        request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        if mode & libc::S_IFMT != libc::S_IFREG && !self.fs.config.specials_enabled() {
            reply.error(Errno::EOPNOTSUPP);
            return;
        }
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let identity = (request.uid(), request.gid());
        self.spawn(reply, move |fs, reply| async move {
            match fs
                .create_with_identity(parent.0, &name, mode & !umask, None, rdev, identity)
                .await
            {
                Ok(node) => {
                    fs.reference(&node);
                    let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                    reply.finish(|reply| reply.entry(&ttl, &attr, Generation(0)));
                }
                Err(error) => reply.failure(error),
            }
        });
    }
    fn symlink(
        &self,
        request: &Request,
        parent: INodeNo,
        name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        use std::os::unix::ffi::OsStrExt;
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let target = target.as_os_str().as_bytes().to_vec();
        if target.len() > 4096 {
            reply.error(Errno::ENAMETOOLONG);
            return;
        }
        let identity = (request.uid(), request.gid());
        self.spawn(reply, move |fs, reply| async move {
            match fs
                .create_with_identity(
                    parent.0,
                    &name,
                    libc::S_IFLNK | 0o777,
                    Some(&target),
                    0,
                    identity,
                )
                .await
            {
                Ok(node) => {
                    fs.reference(&node);
                    let (attr, ttl) = node.cached_attr(fs.config.stat_cache_ttl);
                    reply.finish(|reply| reply.entry(&ttl, &attr, Generation(0)));
                }
                Err(error) => reply.failure(error),
            }
        });
    }
    fn readlink(&self, _: &Request, ino: INodeNo, reply: ReplyData) {
        match self.fs.node(ino.0) {
            Ok(node) => {
                let target = node
                    .state
                    .lock()
                    .unwrap()
                    .metadata
                    .get(&self.fs.config.symlink_attr)
                    .map(<[u8]>::to_vec);
                match target {
                    Some(target) => reply.data(&target),
                    None => reply.error(Errno::EINVAL),
                }
            }
            Err(error) => reply.error(errno(error)),
        }
    }
    fn link(&self, _: &Request, _: INodeNo, _: INodeNo, _: &OsStr, reply: ReplyEntry) {
        // Linux FUSE LINK must return the source inode and file type. Creating
        // a distinct symlink violates that contract and fails after mutation.
        reply.error(Errno::EOPNOTSUPP);
    }
    fn unlink(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        self.remove_entry(parent, name, false, reply);
    }
    fn rmdir(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        self.remove_entry(parent, name, true, reply);
    }
    fn rename(
        &self,
        _: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        if !flags.is_empty() {
            reply.error(Errno::EOPNOTSUPP);
            return;
        }
        let (name, newname) = match (text(name), text(newname)) {
            (Ok(name), Ok(newname)) => (name, newname),
            _ => {
                reply.error(Errno::EILSEQ);
                return;
            }
        };
        // VFS updates rename dentries itself. Extra entry notifications can wait
        // for a directory operation that needs this executor or notifier lock,
        // even after our rename reply has been sent.
        self.spawn(reply, move |fs, reply| async move {
            match fs.rename(parent.0, &name, newparent.0, &newname).await {
                Ok(()) => reply.finish(ReplyEmpty::ok),
                Err(error) => reply.failure(error),
            }
        });
    }
    fn fsync(&self, request: &Request, ino: INodeNo, _: FileHandle, _: bool, reply: ReplyEmpty) {
        self.sync_file(ino, self.track(Opcode::Fsync, request, ino, (0, 0), reply));
    }
    fn fsyncdir(&self, request: &Request, ino: INodeNo, _: FileHandle, _: bool, reply: ReplyEmpty) {
        let reply = self.track(Opcode::Fsync, request, ino, (0, 0), reply);
        self.spawn_traced(reply, move |fs, reply| async move {
            match fs.flush_all().await {
                Ok(()) => reply.finish(ReplyEmpty::ok),
                Err(error) => reply.failure(error),
            }
        });
    }
    fn flush(
        &self,
        request: &Request,
        ino: INodeNo,
        _: FileHandle,
        _: LockOwner,
        reply: ReplyEmpty,
    ) {
        if self.fs.config.fsync_on_close {
            self.sync_file(ino, self.track(Opcode::Fsync, request, ino, (0, 0), reply));
        } else {
            reply.ok();
        }
    }
    fn release(
        &self,
        _: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _: OpenFlags,
        _: Option<LockOwner>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        let removed = {
            let mut files = self.files.lock().unwrap();
            if files
                .get(&fh.0)
                .is_some_and(|entry| entry.file.ino() == ino.0)
            {
                files.remove(&fh.0)
            } else {
                None
            }
        };
        if let Some(file) = removed {
            drop(file);
            reply.ok();
            self.retire_file_cache(ino);
        } else {
            reply.error(Errno::EBADF);
        }
    }
    fn opendir(&self, _: &Request, ino: INodeNo, _: OpenFlags, reply: ReplyOpen) {
        match self.fs.node(ino.0) {
            Ok(node) if node.attr().kind == FileType::Directory => {}
            Ok(_) => {
                reply.error(Errno::ENOTDIR);
                return;
            }
            Err(error) => {
                reply.error(errno(error));
                return;
            }
        }
        let permit = match self.reserve_handle() {
            Ok(permit) => permit,
            Err(error) => {
                reply.error(errno(error));
                return;
            }
        };
        if let Err(error) = self.fs.open(ino.0, false) {
            reply.error(errno(error));
            return;
        }
        let mut directories = self.directories.lock().unwrap();
        let charge = match self.fs.charge_metadata(
            std::mem::size_of::<tokio::sync::Mutex<Directory>>()
                + std::mem::size_of::<(u64, Arc<tokio::sync::Mutex<Directory>>)>()
                + 2 * crate::metadata_budget::LINKS,
        ) {
            Ok(charge) => charge,
            Err(error) => {
                drop(directories);
                self.fs.release(ino.0);
                reply.error(errno(error));
                return;
            }
        };
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        directories.insert(
            handle,
            Arc::new(tokio::sync::Mutex::new(Directory {
                _permit: permit,
                _metadata_charge: charge,
                last: Default::default(),
                page_after: Default::default(),
                cookie: 0,
            })),
        );
        drop(directories);
        reply.opened(FileHandle(handle), FopenFlags::empty());
    }
    fn readdir(
        &self,
        request: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectory,
    ) {
        self.read_directory(
            ino,
            fh,
            offset,
            self.track(
                Opcode::Readdir,
                request,
                ino,
                (offset, 0),
                DirectoryReply::Basic(reply, Vec::new()),
            ),
        );
    }
    fn readdirplus(
        &self,
        request: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectoryPlus,
    ) {
        self.read_directory(
            ino,
            fh,
            offset,
            self.track(
                Opcode::Readdirplus,
                request,
                ino,
                (offset, 0),
                DirectoryReply::Plus(reply, Vec::new(), Vec::new()),
            ),
        );
    }
    fn releasedir(
        &self,
        _: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _: OpenFlags,
        reply: ReplyEmpty,
    ) {
        let directory = self.directories.lock().unwrap().remove(&fh.0);
        drop(directory);
        self.fs.release(ino.0);
        reply.ok();
    }
    fn getxattr(&self, _: &Request, ino: INodeNo, name: &OsStr, size: u32, reply: ReplyXattr) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        if name == "geesefs" {
            xattr_reply(reply, size, env!("CARGO_PKG_VERSION").as_bytes());
            return;
        }
        if name == format!("user.{}", self.fs.config.symlink_attr) {
            reply.error(Errno::ENODATA);
            return;
        }
        match self.fs.xattrs(ino.0) {
            Ok(attrs) => match attrs.get(&name) {
                Some(value) => xattr_reply(reply, size, value),
                None => reply.error(Errno::ENODATA),
            },
            Err(error) => reply.error(errno(error)),
        }
    }
    fn listxattr(&self, _: &Request, ino: INodeNo, size: u32, reply: ReplyXattr) {
        match self.fs.xattrs(ino.0) {
            Ok(attrs) => {
                let length = attrs.keys().map(|name| name.len() + 1).sum();
                let _charge = match self.fs.charge_metadata(length) {
                    Ok(charge) => charge,
                    Err(error) => {
                        reply.error(errno(error));
                        return;
                    }
                };
                let mut names = Vec::with_capacity(length);
                for name in attrs.keys() {
                    names.extend_from_slice(name.as_bytes());
                    names.push(0);
                }
                xattr_reply(reply, size, &names);
            }
            Err(error) => reply.error(errno(error)),
        }
    }
    fn setxattr(
        &self,
        _: &Request,
        ino: INodeNo,
        name: &OsStr,
        value: &[u8],
        flags: i32,
        position: u32,
        reply: ReplyEmpty,
    ) {
        if position != 0 {
            reply.error(Errno::EINVAL);
            return;
        }
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        match self.fs.setxattr(ino.0, &name, Some(value), flags) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(errno(error)),
        }
    }
    fn removexattr(&self, _: &Request, ino: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        match self.fs.setxattr(ino.0, &name, None, 0) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(errno(error)),
        }
    }
    fn access(&self, _: &Request, _: INodeNo, _: AccessFlags, reply: ReplyEmpty) {
        reply.ok();
    }
    fn fallocate(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        length: u64,
        mode: i32,
        reply: ReplyEmpty,
    ) {
        match self.allocate(ino, offset, length, mode) {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(errno(error)),
        }
    }
}
