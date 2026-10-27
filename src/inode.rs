use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{backend::s3::ObjectHead, budget::Block, config::Config};
use fuser::{FileAttr, FileType, INodeNo};

#[derive(Clone)]
pub struct Extent {
    pub end: u64,
    pub data: Block,
    pub data_offset: usize,
    pub version: u64,
}

pub struct State {
    pub key: String,
    /// Backing object after a known copy, before a retryable rename is published.
    pub data_key: Option<String>,
    pub size: u64,
    pub remote: Option<ObjectHead>,
    pub visible_remote_size: u64,
    pub extents: BTreeMap<u64, Extent>,
    pub version: u64,
    pub durable: u64,
    pub truncate_version: u64,
    pub cuts: BTreeMap<u64, u64>,
    pub metadata: BTreeMap<String, Vec<u8>>,
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
    pub rename_target: Option<String>,
    pub negative_lookups: std::collections::BTreeMap<String, std::time::Instant>,
}

pub struct Inode {
    pub ino: u64,
    pub state: Mutex<State>,
    /// Serialize commits/rename/delete for this object only, never the namespace.
    pub flush: std::sync::Arc<tokio::sync::Mutex<()>>,
    pub(crate) preload: tokio::sync::Mutex<()>,
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
        let size = remote.as_ref().map_or(0, |r| r.size);
        let metadata = remote
            .as_ref()
            .map(|r| {
                crate::backend::metadata::decode_metadata(
                    r.metadata.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                )
            })
            .unwrap_or_default();
        let mut mode = if directory {
            libc::S_IFDIR | u32::from(config.dir_mode)
        } else {
            libc::S_IFREG | u32::from(config.file_mode)
        };
        let specials = config.specials_enabled();
        if let Some(parsed) = metadata
            .get(&config.mode_attr)
            .and_then(|b| parse_integer(b))
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
                .and_then(|b| parse_integer(b))
                .and_then(|v| u32::try_from(v).ok())
        } else {
            None
        }
        .or(config.uid)
        .unwrap_or_else(|| rustix::process::getuid().as_raw());
        let gid = if config.enable_perms {
            metadata
                .get(&config.gid_attr)
                .and_then(|b| parse_integer(b))
                .and_then(|v| u32::try_from(v).ok())
        } else {
            None
        }
        .or(config.gid)
        .unwrap_or_else(|| rustix::process::getgid().as_raw());
        let mtime = if config.enable_mtime {
            metadata
                .get(&config.mtime_attr)
                .and_then(|b| parse_integer(b))
                .and_then(|v| UNIX_EPOCH.checked_add(std::time::Duration::from_secs(v)))
        } else {
            None
        }
        .or_else(|| remote.as_ref().and_then(|r| r.last_modified))
        .unwrap_or_else(SystemTime::now);
        let rdev = metadata
            .get(&config.rdev_attr)
            .and_then(|b| parse_integer(b))
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let size = if mode & libc::S_IFMT == libc::S_IFLNK {
            metadata
                .get(&config.symlink_attr)
                .map_or(size, |v| v.len() as u64)
        } else {
            size
        };
        Self {
            ino,
            state: Mutex::new(State {
                key,
                data_key: None,
                size,
                remote,
                visible_remote_size: size,
                extents: BTreeMap::new(),
                version: 0,
                durable: 0,
                truncate_version: 0,
                cuts: BTreeMap::new(),
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
                rename_target: None,
                negative_lookups: std::collections::BTreeMap::new(),
            }),
            flush: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            preload: tokio::sync::Mutex::new(()),
            pattern: Mutex::new(crate::read::Pattern::default()),
            read_fills: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn attr(&self) -> FileAttr {
        let s = self.state.lock().unwrap();
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
