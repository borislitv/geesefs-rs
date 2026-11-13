//! Isolated adapter harness, including crate-private budget ownership checks.
#![allow(dead_code)]

pub use geesefs_rs::error;
#[path = "../../src/budget.rs"]
pub mod budget;
#[path = "../../src/disk_cache/mod.rs"]
pub mod disk_cache;

use budget::{Block, Budget};
use disk_cache::{DiskCache, DiskKey, PreparedDiskCache};
use std::{path::Path, time::Duration};

pub const MIB: usize = 1024 * 1024;
pub const DEADLINE: Duration = Duration::from_secs(10);
pub fn directory() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
pub fn key(offset: u64, length: usize) -> DiskKey {
    DiskKey::new(
        b"public-test-namespace",
        b"packages/example.rpm",
        b"opaque-ETag",
        16 * MIB as u64,
        offset,
        length,
    )
    .unwrap()
}
pub fn block(bytes: &[u8]) -> Block {
    Budget::new(bytes.len().max(1))
        .unwrap()
        .try_reserve(bytes.len())
        .unwrap()
        .copy(bytes)
        .unwrap()
}
pub async fn open(directory: &Path, resources: &Budget) -> DiskCache {
    DiskCache::open(
        PreparedDiskCache::prepare(directory, (16 * MIB) as u64).unwrap(),
        resources,
    )
    .await
    .unwrap()
}

use foyer::{IoEngine, IoEngineConfig};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[derive(Debug)]
pub struct IoGate {
    pub hold_reads: AtomicBool,
    pub hold_writes: AtomicBool,
    pub publication_only: AtomicBool,
    armed_data_writes: AtomicUsize,
    pub payload_written: tokio::sync::Semaphore,
    pub log_written: tokio::sync::Semaphore,
    pub fail_write: AtomicBool,
    pub fail_read: AtomicBool,
    pub entered: tokio::sync::Semaphore,
    pub release: tokio::sync::Semaphore,
}
impl IoGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            hold_reads: AtomicBool::new(false),
            hold_writes: AtomicBool::new(false),
            publication_only: AtomicBool::new(false),
            armed_data_writes: AtomicUsize::new(0),
            payload_written: tokio::sync::Semaphore::new(0),
            log_written: tokio::sync::Semaphore::new(0),
            fail_write: AtomicBool::new(false),
            fail_read: AtomicBool::new(false),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        })
    }
}
#[derive(Debug)]
pub struct GateConfig(pub Arc<IoGate>);
#[derive(Debug)]
struct GateEngine {
    real: Arc<dyn IoEngine>,
    gate: Arc<IoGate>,
}
impl IoEngineConfig for GateConfig {
    fn build(
        self: Box<Self>,
        context: foyer::IoEngineBuildContext,
    ) -> futures_util::future::BoxFuture<'static, foyer::Result<Arc<dyn IoEngine>>> {
        Box::pin(async move {
            let real = Box::new(foyer::PsyncIoEngineConfig::new())
                .build(context)
                .await?;
            Ok(Arc::new(GateEngine { real, gate: self.0 }) as Arc<dyn IoEngine>)
        })
    }
}
impl IoEngine for GateEngine {
    fn read(
        &self,
        buffer: foyer_storage::IoReadBuffer,
        partition: &dyn foyer_storage::Partition,
        offset: u64,
    ) -> foyer::IoHandle {
        let held = self.gate.hold_reads.load(Ordering::SeqCst);
        let fail = self.gate.fail_read.swap(false, Ordering::SeqCst);
        let operation = self.real.read(buffer, partition, offset);
        let gate = self.gate.clone();
        foyer::IoHandle::from(Box::pin(async move {
            if held {
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
            let (buffer, result) = operation.await;
            (
                buffer,
                if fail {
                    Err(foyer::Error::io_error(std::io::Error::from_raw_os_error(
                        libc::EIO,
                    )))
                } else {
                    result
                },
            )
        }) as futures_util::future::BoxFuture<'static, _>)
    }
    fn write(
        &self,
        buffer: foyer_storage::IoWriteBuffer,
        partition: &dyn foyer_storage::Partition,
        offset: u64,
    ) -> foyer::IoHandle {
        let fail = self.gate.fail_write.swap(false, Ordering::SeqCst);
        let armed = self.gate.hold_writes.load(Ordering::SeqCst);
        let publication = self.gate.publication_only.load(Ordering::SeqCst);
        let log_write = armed && publication && partition.id() == 0;
        let payload = armed
            && publication
            && partition.id() != 0
            && self.gate.armed_data_writes.fetch_add(1, Ordering::SeqCst) == 0;
        let held = armed && (!publication || (partition.id() != 0 && !payload));
        let operation = self.real.write(buffer, partition, offset);
        let gate = self.gate.clone();
        foyer::IoHandle::from(Box::pin(async move {
            if held {
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
            let (buffer, result) = operation.await;
            if payload && result.is_ok() {
                gate.payload_written.add_permits(1);
            }
            if log_write && result.is_ok() {
                gate.log_written.add_permits(1);
            }
            (
                buffer,
                if fail {
                    Err(foyer::Error::io_error(std::io::Error::from_raw_os_error(
                        libc::ENOSPC,
                    )))
                } else {
                    result
                },
            )
        }) as futures_util::future::BoxFuture<'static, _>)
    }
}

pub async fn gated(directory: &Path, resources: &Budget, gate: Arc<IoGate>) -> DiskCache {
    DiskCache::open_test(
        PreparedDiskCache::prepare(directory, (16 * MIB) as u64).unwrap(),
        resources,
        Some(Box::new(GateConfig(gate))),
    )
    .await
    .unwrap()
}

pub async fn child(mode: &str, directory: &Path) {
    let mut process = tokio::process::Command::new(std::env::current_exe().unwrap());
    process
        .args(["--exact", "disk_cache_child", "--nocapture"])
        .env("GEESEFS_L2_CHILD_MODE", mode)
        .env("GEESEFS_L2_CHILD_DIR", directory)
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(20), process.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "{mode}: stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

/// Native failure tests never mount in the caller's namespace. All commands are
/// bounded, and a missing prerequisite is a failure, not a successful skip.
pub async fn native_command(command: &mut tokio::process::Command) {
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .expect("native fixture command could not start");
    let status = match tokio::time::timeout(Duration::from_secs(120), child.wait()).await {
        Ok(status) => status.expect("native fixture command wait failed"),
        Err(_) => {
            child
                .start_kill()
                .expect("cannot stop timed-out native fixture child");
            tokio::time::timeout(Duration::from_secs(10), child.wait())
                .await
                .expect("timed-out native fixture child did not exit")
                .expect("cannot reap native fixture child");
            panic!("native fixture command exceeded 120s: {command:?}");
        }
    };
    assert!(
        status.success(),
        "{command:?}: status={status} (command output inherited)"
    );
}

pub fn assert_private_mount_namespace() {
    let parent = std::env::var_os("GEESEFS_L2_PARENT_MNT")
        .expect("run the documented native_filesystem_failure_paths entry point");
    assert_ne!(
        std::fs::read_link("/proc/self/ns/mnt").unwrap().as_os_str(),
        parent,
        "native fixtures require a separate private mount namespace"
    );
}

/// The parent owns the temporary directory until the namespace child exits.
/// Unmount before any recursive removal. Even on panic, the namespace child must
/// exit before the owning parent removes the temporary image/directory.
pub struct NativeMount(pub std::path::PathBuf);
impl Drop for NativeMount {
    fn drop(&mut self) {
        let status = std::process::Command::new("timeout")
            .args(["--signal=KILL", "30s", "umount", "--"])
            .arg(&self.0)
            .status()
            .expect("cannot run bounded fixture unmount");
        assert!(status.success(), "owned fixture unmount failed: {status}");
    }
}
