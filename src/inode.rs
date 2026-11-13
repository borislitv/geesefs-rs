use std::{
    collections::BTreeMap,
    sync::{Mutex, atomic::AtomicU64},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    backend::{
        metadata::{CachedMetadata, CachedRemote},
        s3::ObjectHead,
    },
    budget::Block,
    config::Config,
};
use crate::{
    error::Result,
    metadata_budget::{LINKS, MetadataBudget, MetadataCharge, head_bytes},
};
use fuser::{FileAttr, FileType, INodeNo};

#[derive(Clone)]
pub struct Extent {
    pub end: u64,
    pub data: Block,
    pub data_offset: usize,
    pub version: u64,
}

#[derive(Default)]
pub struct WriterState {
    /// Backing object after a known copy, before a retryable rename is published.
    pub data_key: Option<crate::metadata_budget::SharedKey>,
    pub extents: BTreeMap<u64, Extent>,
    pub(crate) pending_upload: Option<std::sync::Arc<crate::write::PendingUpload>>,
    pub truncate_version: u64,
    pub cuts: BTreeMap<u64, u64>,
    pub rename_target: Option<String>,
}

// Borrowed reads never allocate, and no mutable reference to this value escapes.
static EMPTY_WRITER: WriterState = WriterState {
    data_key: None,
    extents: BTreeMap::new(),
    pending_upload: None,
    truncate_version: 0,
    cuts: BTreeMap::new(),
    rename_target: None,
};

pub struct State {
    pub(crate) metadata_charge: MetadataCharge,
    pub key: crate::metadata_budget::SharedKey,
    pub size: u64,
    pub remote: Option<CachedRemote>,
    pub visible_remote_size: u64,
    pub version: u64,
    pub durable: u64,
    pub(crate) writer: Option<Box<WriterState>>,
    pub metadata: CachedMetadata,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub mtime: SystemTime,
    pub deleted: bool,
    pub lookups: u64,
    pub opens: u64,
    pub metadata_known: bool,
    pub fetched: std::time::Instant,
    pub negative_lookups: std::collections::BTreeMap<String, std::time::Instant>,
}

impl std::ops::Deref for State {
    type Target = WriterState;
    fn deref(&self) -> &WriterState {
        self.writer.as_deref().unwrap_or(&EMPTY_WRITER)
    }
}

impl State {
    /// Admit the backing before exposing mutable writer fields. Preserve any
    /// earlier prepaid mutation allowance rather than settling it away here.
    pub(crate) fn writer_mut(&mut self) -> Result<&mut WriterState> {
        self.writer_mut_with(0)
    }

    pub(crate) fn writer_mut_with(&mut self, growth: usize) -> Result<&mut WriterState> {
        if growth == 0 && self.writer.is_some() {
            return Ok(self.writer.as_deref_mut().unwrap());
        }
        if self.metadata_budget().enabled() {
            let backing = if self.writer.is_none() {
                std::mem::size_of::<WriterState>()
            } else {
                0
            };
            self.metadata_charge.resize(
                self.metadata_charge
                    .bytes()
                    .max(self.metadata_bytes().saturating_add(growth))
                    .saturating_add(backing),
            )?;
        }
        if self.writer.is_none() {
            self.writer = Some(Box::default());
        }
        Ok(self.writer.as_deref_mut().unwrap())
    }

    pub(crate) fn writer_mut_from(
        &mut self,
        credit: &mut MetadataCharge,
    ) -> Result<&mut WriterState> {
        if self.writer.is_none() && self.metadata_budget().enabled() {
            self.metadata_charge.ensure_from(
                self.metadata_charge
                    .bytes()
                    .max(self.metadata_bytes())
                    .saturating_add(std::mem::size_of::<WriterState>()),
                credit,
            )?;
        }
        // The backing is now fully admitted, including existing prepaid growth.
        // Avoid charging it a second time through writer_mut().
        if self.writer.is_none() {
            self.writer = Some(Box::default());
        }
        Ok(self.writer.as_deref_mut().unwrap())
    }
    pub(crate) fn cache_remote_from(
        &mut self,
        head: ObjectHead,
        credit: &mut MetadataCharge,
    ) -> Result<CachedRemote> {
        let growth = self
            .metadata_budget()
            .estimate(|| Inode::metadata_estimate(&self.key, Some(&head)));
        self.admit_metadata_from(credit, growth)?;
        CachedRemote::decode_admitted(head, &self.metadata, &mut self.metadata_charge)
    }
    pub(crate) fn admit_metadata_from(
        &mut self,
        credit: &mut MetadataCharge,
        growth: usize,
    ) -> Result<()> {
        if !self.metadata_budget().enabled() {
            return Ok(());
        }
        self.metadata_charge
            .ensure_from(self.metadata_bytes().saturating_add(growth), credit)
    }
    pub(crate) fn metadata_budget(&self) -> &MetadataBudget {
        self.metadata_charge.budget()
    }

    pub(crate) fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Inode>()
            + 4 * LINKS
            + self
                .writer
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<WriterState>())
            + self.rename_target.as_ref().map_or(0, String::capacity)
            + self
                .negative_lookups
                .keys()
                .map(|key| {
                    key.capacity() + std::mem::size_of::<(String, std::time::Instant)>() + LINKS
                })
                .sum::<usize>()
            + self.extents.len() * (std::mem::size_of::<(u64, Extent)>() + LINKS)
            + self.cuts.len() * (std::mem::size_of::<(u64, u64)>() + LINKS)
    }

    /// Reserve a conservative growth allowance before mutating authoritative state.
    pub(crate) fn admit_metadata(&mut self, growth: usize) -> Result<()> {
        if self.metadata_budget().usage().limit.is_none() {
            return Ok(());
        }
        self.metadata_charge
            .resize(self.metadata_bytes().saturating_add(growth))
    }

    pub(crate) fn settle_metadata(&mut self) {
        if self.metadata_budget().usage().limit.is_none() {
            return;
        }
        let bytes = self.metadata_bytes();
        // Callers admitted all growth before mutation; settlement only releases.
        debug_assert!(
            self.metadata_budget().usage().limit.is_none() || bytes <= self.metadata_charge.bytes()
        );
        self.metadata_charge
            .resize(bytes)
            .expect("admitted metadata mutation");
    }
    pub(crate) fn can_replace_metadata(&self, observed: std::time::Instant) -> bool {
        !self.deleted
            && self.version == self.durable
            && self.rename_target.is_none()
            && self.data_key.is_none()
            && self.fetched <= observed
    }

    /// Replace clean remote metadata without changing the live inode's owners,
    /// local generation counters or separately validated negative lookups.
    pub(crate) fn replace_metadata(&mut self, mut fresh: Self) -> Result<()> {
        let negatives = self.metadata_budget().estimate(|| {
            self.negative_lookups
                .keys()
                .map(|key| {
                    key.capacity() + std::mem::size_of::<(String, std::time::Instant)>() + LINKS
                })
                .sum()
        });
        fresh.admit_metadata(negatives)?;
        fresh.lookups = self.lookups;
        fresh.opens = self.opens;
        fresh.key = self.key.clone();
        fresh.version = self.version;
        fresh.durable = self.durable;
        fresh.negative_lookups = std::mem::take(&mut self.negative_lookups);
        fresh.settle_metadata();
        *self = fresh;
        Ok(())
    }
}

pub struct Inode {
    pub ino: u64,
    pub state: Mutex<State>,
    /// Serialize commits/rename/delete for this object only, never the namespace.
    pub flush: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub(crate) preload: tokio::sync::Mutex<()>,
    /// Listing invalidation is independent of refreshed object metadata.
    pub(crate) directory_epoch: AtomicU64,
    pub pattern: Mutex<crate::read::Pattern>,
    pub(crate) read_fills: Mutex<BTreeMap<(u64, u64), crate::read::Fill>>,
}

pub fn parse_integer(bytes: &[u8]) -> Option<u64> {
    let value = std::str::from_utf8(bytes).ok()?;
    if let Some(value) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(value, 16).ok()
    } else if value.len() > 1 && value.starts_with('0') {
        u64::from_str_radix(&value[1..], 8).ok()
    } else {
        value.parse().ok()
    }
}

impl Inode {
    pub fn new(
        ino: u64,
        key: String,
        directory: bool,
        remote: Option<ObjectHead>,
        config: &Config,
    ) -> Self {
        Self::try_new(
            ino,
            key,
            directory,
            remote,
            config,
            &MetadataBudget::default(),
        )
        .expect("disabled metadata admission")
    }

    pub(crate) fn try_new(
        ino: u64,
        key: String,
        directory: bool,
        remote: Option<ObjectHead>,
        config: &Config,
        budget: &MetadataBudget,
    ) -> Result<Self> {
        let charge = budget.charge_with(|| Self::metadata_estimate(&key, remote.as_ref()))?;
        Self::new_admitted(ino, key, directory, remote, config, charge)
    }

    pub(crate) fn metadata_estimate(key: &str, remote: Option<&ObjectHead>) -> usize {
        // Decode only after admitting an upper bound for decoded strings/tree nodes.
        let decoded = remote.map_or(0, |head| {
            head.metadata
                .iter()
                .map(|(key, value)| {
                    2 * (3 * key.len()).max(8).next_power_of_two()
                        + 2 * value.len().max(8).next_power_of_two()
                        + std::mem::size_of::<(String, Vec<u8>)>()
                        + LINKS
                })
                .sum::<usize>()
        });
        std::mem::size_of::<Inode>()
            + 256
            + 4 * LINKS
            + 3 * key.len()
            + remote.map_or(0, head_bytes)
            + decoded
    }

    pub(crate) fn new_admitted(
        ino: u64,
        key: String,
        directory: bool,
        remote: Option<ObjectHead>,
        config: &Config,
        mut charge: MetadataCharge,
    ) -> Result<Self> {
        let Some(head) = remote else {
            let key = crate::metadata_budget::SharedKey::from_admitted(key, &mut charge)?;
            return Self::cached_admitted(ino, key, directory, None, None, config, charge);
        };
        let last_modified = head.last_modified;
        let metadata = crate::backend::metadata::decode_metadata(
            head.metadata.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        );
        let metadata = CachedMetadata::from_admitted(metadata, &mut charge)?;
        let remote = Some(CachedRemote::from_admitted(head, metadata, &mut charge)?);
        let key = crate::metadata_budget::SharedKey::from_admitted(key, &mut charge)?;
        Self::cached_admitted(ino, key, directory, remote, last_modified, config, charge)
    }

    pub(crate) fn cached_admitted(
        ino: u64,
        key: crate::metadata_budget::SharedKey,
        directory: bool,
        remote: Option<CachedRemote>,
        last_modified: Option<SystemTime>,
        config: &Config,
        mut charge: MetadataCharge,
    ) -> Result<Self> {
        let size = remote.as_ref().map_or(0, |r| r.size);
        let metadata = match &remote {
            Some(remote) => remote.metadata.clone(),
            None => CachedMetadata::from_admitted(BTreeMap::new(), &mut charge)?,
        };
        let mut mode = if directory {
            libc::S_IFDIR | u32::from(config.dir_mode)
        } else {
            libc::S_IFREG | u32::from(config.file_mode)
        };
        let specials = config.specials_enabled();
        if let Some(parsed) = metadata
            .get(&config.mode_attr)
            .and_then(parse_integer)
            .and_then(|v| u32::try_from(v).ok())
        {
            if config.enable_perms {
                mode = (mode & !0o7777) | (parsed & 0o7777);
            }
            if specials && !directory && parsed & libc::S_IFMT != 0 {
                mode = (mode & !libc::S_IFMT) | (parsed & libc::S_IFMT);
            }
        }
        if metadata.contains_key(&config.symlink_attr) && !directory {
            mode = (mode & !libc::S_IFMT) | libc::S_IFLNK;
        }
        let uid = if config.enable_perms {
            metadata
                .get(&config.uid_attr)
                .and_then(parse_integer)
                .and_then(|v| u32::try_from(v).ok())
        } else {
            None
        }
        .or(config.uid)
        .unwrap_or_else(|| rustix::process::getuid().as_raw());
        let gid = if config.enable_perms {
            metadata
                .get(&config.gid_attr)
                .and_then(parse_integer)
                .and_then(|v| u32::try_from(v).ok())
        } else {
            None
        }
        .or(config.gid)
        .unwrap_or_else(|| rustix::process::getgid().as_raw());
        let mtime = if config.enable_mtime {
            metadata
                .get(&config.mtime_attr)
                .and_then(parse_integer)
                .and_then(|v| UNIX_EPOCH.checked_add(std::time::Duration::from_secs(v)))
        } else {
            None
        }
        .or(last_modified)
        .unwrap_or_else(SystemTime::now);
        let rdev = metadata
            .get(&config.rdev_attr)
            .and_then(parse_integer)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let result = Self {
            ino,
            directory_epoch: AtomicU64::new(0),
            state: Mutex::new(State {
                metadata_charge: charge,
                key,
                size,
                remote,
                visible_remote_size: size,
                version: 0,
                durable: 0,
                writer: None,
                metadata,
                mode,
                uid,
                gid,
                rdev,
                mtime,
                deleted: false,
                lookups: 0,
                opens: 0,
                metadata_known: true,
                fetched: std::time::Instant::now(),
                negative_lookups: std::collections::BTreeMap::new(),
            }),
            flush: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            preload: tokio::sync::Mutex::new(()),
            pattern: Mutex::new(crate::read::Pattern::default()),
            read_fills: Mutex::new(BTreeMap::new()),
        };
        result.state.lock().unwrap().settle_metadata();
        Ok(result)
    }

    pub fn attr(&self) -> FileAttr {
        let s = self.state.lock().unwrap();
        self.attr_from_state(&s)
    }

    fn attr_from_state(&self, s: &State) -> FileAttr {
        let kind = file_type(s.mode);
        let size = s.size;
        FileAttr {
            ino: INodeNo(self.ino),
            size,
            blocks: size.div_ceil(512),
            atime: s.mtime,
            mtime: s.mtime,
            ctime: s.mtime,
            crtime: UNIX_EPOCH,
            kind,
            perm: (s.mode & 0o7777) as u16,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid: s.uid,
            gid: s.gid,
            rdev: s.rdev,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Returning an old cached value must not restart its remote freshness TTL.
    pub fn cached_attr(&self, ttl: Duration) -> (FileAttr, Duration) {
        let state = self.state.lock().unwrap();
        let remaining = if state.deleted {
            Duration::ZERO
        } else if self.ino == 1
            || state.version > state.durable
            || state.rename_target.is_some()
            || state.data_key.is_some()
        {
            ttl
        } else if state.metadata_known {
            ttl.saturating_sub(state.fetched.elapsed())
        } else {
            Duration::ZERO
        };
        (self.attr_from_state(&state), remaining)
    }
}

pub fn file_type(mode: u32) -> FileType {
    match mode & libc::S_IFMT {
        libc::S_IFDIR => FileType::Directory,
        libc::S_IFLNK => FileType::Symlink,
        libc::S_IFIFO => FileType::NamedPipe,
        libc::S_IFCHR => FileType::CharDevice,
        libc::S_IFBLK => FileType::BlockDevice,
        libc::S_IFSOCK => FileType::Socket,
        _ => FileType::RegularFile,
    }
}
