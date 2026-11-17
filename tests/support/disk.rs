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

pub use disk_cache::TestGate as EngineGate;
use std::sync::Arc;

pub async fn gated(directory: &Path, resources: &Budget, gate: Arc<EngineGate>) -> DiskCache {
    DiskCache::open(
        PreparedDiskCache::prepare(directory, (16 * MIB) as u64)
            .unwrap()
            .with_test_gate(gate),
        resources,
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
