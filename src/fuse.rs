//! fuser owns the wire protocol and exactly-once replies. Admission and borrowed
//! write ownership are acquired before detached work, never in an unbounded queue.
use crate::{error::Error, filesystem::Filesystem as Core, write::MAX_IO};
use fuser::*;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    future::Future,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};
use tokio::{runtime::Handle, sync::Semaphore};

pub fn errno(error: Error) -> Errno {
    match error {
        Error::Fs(code) => Errno::from_i32(code),
        Error::Config(_) => Errno::EINVAL,
        Error::Unsupported(_) => Errno::EOPNOTSUPP,
        Error::Budget { .. } => Errno::ENOMEM,
        Error::Timeout(_) => Errno::ETIMEDOUT,
        Error::Cancelled => Errno::EINTR,
        Error::Backend { code, .. } => match code.as_str() {
            "NoSuchKey" | "NotFound" | "404" => Errno::ENOENT,
            "AccessDenied" | "InvalidAccessKeyId" | "SignatureDoesNotMatch" => Errno::EACCES,
            "PreconditionFailed" => Errno::ESTALE,
            _ => Errno::EIO,
        },
        Error::InvalidResponse(_) => Errno::EIO,
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
    ReplyXattr
);

struct Admission {
    closing: bool,
    semaphore: Arc<Semaphore>,
}
struct Directory {
    last: String,
    cookie: u64,
}

pub struct Fuse {
    fs: Arc<Core>,
    runtime: Handle,
    admission: Mutex<Admission>,
    directories: Arc<Mutex<BTreeMap<u64, Arc<tokio::sync::Mutex<Directory>>>>>,
    next_handle: AtomicU64,
    pub notifier: Arc<Mutex<Option<Notifier>>>,
    pub shutdown_error: Arc<Mutex<Option<String>>>,
}

impl Fuse {
    pub fn new(fs: Arc<Core>, runtime: Handle) -> Self {
        let slots = fs.config.max_requests;
        Self {
            fs,
            runtime,
            admission: Mutex::new(Admission {
                closing: false,
                semaphore: Arc::new(Semaphore::new(slots)),
            }),
            directories: Arc::new(Mutex::new(BTreeMap::new())),
            next_handle: AtomicU64::new(1),
            notifier: Arc::new(Mutex::new(None)),
            shutdown_error: Arc::new(Mutex::new(None)),
        }
    }
    fn spawn<R, F, T>(&self, reply: R, job: F)
    where
        R: Failure,
        F: FnOnce(Arc<Core>, R) -> T + Send + 'static,
        T: Future<Output = ()> + Send + 'static,
    {
        let admission = self.admission.lock().unwrap();
        if admission.closing {
            reply.fail(Errno::EINTR);
            return;
        }
        let permit = match admission.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                reply.fail(Errno::EAGAIN);
                return;
            }
        };
        let fs = self.fs.clone();
        let deadline = fs.config.request_timeout;
        self.runtime.spawn(async move {
            let _permit = permit;
            // On expiry the owned reply is dropped once; fuser sends its failure
            // reply. Dropping the job also cancels its bounded backend operations.
            if tokio::time::timeout(deadline, job(fs, reply))
                .await
                .is_err()
            {
                eprintln!("FUSE request deadline exceeded");
            }
        });
    }
    fn flags(&self) -> FopenFlags {
        if self.fs.config.read_only_direct_io {
            FopenFlags::FOPEN_DIRECT_IO
        } else {
            FopenFlags::empty()
        }
    }
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
                self.fs.shutdown().await
            })
            .await
        });
        let result = result.unwrap_or(Err(Error::Timeout("FUSE shutdown")));
        if let Err(error) = result {
            *self.shutdown_error.lock().unwrap() = Some(error.to_string());
        }
    }
    fn lookup(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        self.spawn(reply, move |fs, reply| async move {
            match fs.lookup(parent.0, &name).await {
                Ok(node) => {
                    fs.reference(&node);
                    reply.entry(&fs.config.stat_cache_ttl, &node.attr(), Generation(0));
                }
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn forget(&self, _: &Request, ino: INodeNo, count: u64) {
        self.fs.forget(ino.0, count);
    }
    fn getattr(&self, _: &Request, ino: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        self.spawn(reply, move |fs, reply| async move {
            match fs.refresh_inode_if_expired(ino.0).await {
                Ok(node) => reply.attr(&fs.config.stat_cache_ttl, &node.attr()),
                Err(error) => reply.error(errno(error)),
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
    fn open(&self, _: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let result = (|| {
            let node = self.fs.node(ino.0)?;
            if node.attr().kind == FileType::Directory {
                return Err(Error::Fs(libc::EISDIR));
            }
            self.fs
                .open(ino.0, flags.0 & libc::O_ACCMODE != libc::O_RDONLY)
        })();
        match result {
            Ok(()) => reply.opened(FileHandle(ino.0), self.flags()),
            Err(error) => reply.error(errno(error)),
        }
    }
    fn read(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.spawn(reply, move |fs, reply| async move {
            if let Some(error) = fs
                .read_reply_data(ino.0, offset, size as usize, move |result| match result {
                    Ok(data) => reply.data(data),
                    Err(error) => reply.error(errno(error)),
                })
                .await
            {
                eprintln!("read-ahead failed after reply: {error}");
            }
        });
    }
    fn write(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        data: &[u8],
        _: WriteFlags,
        flags: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
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
                reply.error(errno(error));
                return;
            }
        };
        self.spawn(reply, move |fs, reply| async move {
            let result = async {
                let written = fs.write(ino.0, offset, owned.as_slice()).await?;
                if flags.0 & (libc::O_SYNC | libc::O_DSYNC) != 0 {
                    fs.fsync(ino.0).await?;
                }
                Ok::<_, Error>(written)
            }
            .await;
            match result {
                Ok(written) => reply.written(written as u32),
                Err(error) => reply.error(errno(error)),
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
                Ok(node) => match fs.open(node.ino, open_flags & libc::O_ACCMODE != libc::O_RDONLY)
                {
                    Ok(()) => {
                        fs.reference(&node);
                        reply.created(
                            &fs.config.stat_cache_ttl,
                            &node.attr(),
                            Generation(0),
                            FileHandle(node.ino),
                            flags,
                        );
                    }
                    Err(error) => reply.error(errno(error)),
                },
                Err(error) => reply.error(errno(error)),
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
                    reply.entry(&fs.config.stat_cache_ttl, &node.attr(), Generation(0));
                }
                Err(error) => reply.error(errno(error)),
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
                    reply.entry(&fs.config.stat_cache_ttl, &node.attr(), Generation(0));
                }
                Err(error) => reply.error(errno(error)),
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
                    reply.entry(&fs.config.stat_cache_ttl, &node.attr(), Generation(0));
                }
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn readlink(&self, _: &Request, ino: INodeNo, reply: ReplyData) {
        match self.fs.node(ino.0) {
            Ok(node) => {
                let state = node.state.lock().unwrap();
                match state.metadata.get(&self.fs.config.symlink_attr) {
                    Some(target) => reply.data(target),
                    None => reply.error(Errno::EINVAL),
                }
            }
            Err(error) => reply.error(errno(error)),
        }
    }
    fn link(&self, _: &Request, ino: INodeNo, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        if !self.fs.config.emulate_hardlinks_as_symlinks {
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
        self.spawn(reply, move |fs, reply| async move {
            match fs.link(ino.0, parent.0, &name).await {
                Ok(node) => {
                    fs.reference(&node);
                    reply.entry(&fs.config.stat_cache_ttl, &node.attr(), Generation(0));
                }
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn unlink(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        self.spawn(reply, move |fs, reply| async move {
            match fs.unlink(parent.0, &name, false).await {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn rmdir(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match text(name) {
            Ok(name) => name,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        self.spawn(reply, move |fs, reply| async move {
            match fs.unlink(parent.0, &name, true).await {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(errno(error)),
            }
        });
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
        let notifier = self.notifier.clone();
        self.spawn(reply, move |fs, reply| async move {
            match fs.rename(parent.0, &name, newparent.0, &newname).await {
                Ok(()) => {
                    reply.ok();
                    if let Some(notifier) = notifier.lock().unwrap().as_ref() {
                        let _ = notifier.inval_entry(parent, OsStr::new(&name));
                        let _ = notifier.inval_entry(newparent, OsStr::new(&newname));
                    }
                }
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn fsync(&self, _: &Request, ino: INodeNo, _: FileHandle, _: bool, reply: ReplyEmpty) {
        self.spawn(reply, move |fs, reply| async move {
            match fs.fsync(ino.0).await {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn fsyncdir(&self, _: &Request, _: INodeNo, _: FileHandle, _: bool, reply: ReplyEmpty) {
        self.spawn(reply, move |fs, reply| async move {
            match fs.flush_all().await {
                Ok(()) => reply.ok(),
                Err(error) => reply.error(errno(error)),
            }
        });
    }
    fn flush(&self, _: &Request, ino: INodeNo, _: FileHandle, _: LockOwner, reply: ReplyEmpty) {
        if self.fs.config.fsync_on_close {
            self.spawn(reply, move |fs, reply| async move {
                match fs.fsync(ino.0).await {
                    Ok(()) => reply.ok(),
                    Err(error) => reply.error(errno(error)),
                }
            });
        } else {
            reply.ok();
        }
    }
    fn release(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        _: OpenFlags,
        _: Option<LockOwner>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        self.fs.release(ino.0);
        reply.ok();
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
        let mut directories = self.directories.lock().unwrap();
        if directories.len() >= self.fs.config.max_requests * 4 {
            reply.error(Errno::EMFILE);
            return;
        }
        if let Err(error) = self.fs.open(ino.0, false) {
            reply.error(errno(error));
            return;
        }
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        directories.insert(
            handle,
            Arc::new(tokio::sync::Mutex::new(Directory {
                last: String::new(),
                cookie: 0,
            })),
        );
        reply.opened(FileHandle(handle), FopenFlags::empty());
    }
    fn readdir(
        &self,
        _: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        reply: ReplyDirectory,
    ) {
        let cursor = match self.directories.lock().unwrap().get(&fh.0).cloned() {
            Some(cursor) => cursor,
            None => {
                reply.error(Errno::EBADF);
                return;
            }
        };
        self.spawn(reply, move |fs, mut reply| async move {
            let mut cursor = cursor.lock().await;
            if offset != cursor.cookie {
                cursor.last.clear();
                cursor.cookie = 0;
            }
            if cursor.cookie == 0 {
                if offset == 0 && reply.add(ino, 1, FileType::Directory, ".") {
                    reply.ok();
                    return;
                }
                cursor.cookie = 1;
            }
            if cursor.cookie == 1 {
                if offset <= 1
                    && reply.add(INodeNo(fs.parent_ino(ino.0)), 2, FileType::Directory, "..")
                {
                    reply.ok();
                    return;
                }
                cursor.cookie = 2;
            }
            loop {
                let entries = match fs.directory_chunk(ino.0, &cursor.last, 128).await {
                    Ok(entries) => entries,
                    Err(error) => {
                        reply.error(errno(error));
                        return;
                    }
                };
                if entries.is_empty() {
                    break;
                }
                for entry in entries {
                    let next = cursor.cookie + 1;
                    if cursor.cookie >= offset
                        && reply.add(INodeNo(entry.ino), next, entry.kind, &entry.name)
                    {
                        reply.ok();
                        return;
                    }
                    cursor.cookie = next;
                    cursor.last = entry.name;
                }
            }
            reply.ok();
        });
    }
    fn releasedir(
        &self,
        _: &Request,
        ino: INodeNo,
        fh: FileHandle,
        _: OpenFlags,
        reply: ReplyEmpty,
    ) {
        self.directories.lock().unwrap().remove(&fh.0);
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
                let mut names = Vec::new();
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
        if mode != 0 && mode != libc::FALLOC_FL_KEEP_SIZE {
            reply.error(Errno::EOPNOTSUPP);
            return;
        }
        let result = (|| {
            self.fs.check_write()?;
            let end = offset.checked_add(length).ok_or(Error::Fs(libc::EFBIG))?;
            let size = self.fs.node(ino.0)?.attr().size;
            if mode == 0 && end > size {
                self.fs.truncate(ino.0, end)?;
            }
            Ok::<_, Error>(())
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(error) => reply.error(errno(error)),
        }
    }
}
