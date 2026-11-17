use super::PreparedDiskCache;
use crate::{
    budget::Reservation,
    error::{Error, Result},
};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

/// Shared with the adapter so normal close can release the directory after I/O,
/// while both unwind cleanup and remaining cache handles retain their funding.
pub(super) struct EngineOwners {
    pub(super) prepared: Mutex<Option<PreparedDiskCache>>,
    _funding: Reservation,
}

impl EngineOwners {
    pub(super) fn new(prepared: PreparedDiskCache, funding: Reservation) -> Self {
        Self {
            prepared: Mutex::new(Some(prepared)),
            _funding: funding,
        }
    }
}

/// Foyer receives a Handle, so idle workers cannot retain the runtime owner.
pub(super) struct EngineRuntime {
    runtime: Option<Runtime>,
    device: Arc<dyn foyer::Device>,
    owners: Arc<EngineOwners>,
}

impl EngineRuntime {
    pub(super) fn new(device: Arc<dyn foyer::Device>, owners: Arc<EngineOwners>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(2 * super::funding::JOBS + 2)
            .thread_name("geesefs-l2")
            .enable_all()
            .build()
            .map_err(|error| Error::Config(format!("L2 runtime: {error}")))?;
        Ok(Self {
            runtime: Some(runtime),
            device,
            owners,
        })
    }

    pub(super) fn spawner(&self) -> foyer::Spawner {
        self.runtime.as_ref().unwrap().handle().clone().into()
    }

    pub(super) async fn shutdown(mut self) -> Result<()> {
        let runtime = self.runtime.take().unwrap();
        let device = self.device.clone();
        let owners = self.owners.clone();
        tokio::task::spawn_blocking(move || {
            // Psync tasks borrow raw descriptors. Keep all partitions open until
            // runtime shutdown has joined blocking I/O, including queued work.
            drop(runtime);
            drop(device);
            drop(owners);
        })
        .await
        .map_err(|error| Error::Config(format!("L2 runtime shutdown: {error}")))
    }
}

impl Drop for EngineRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            let device = self.device.clone();
            let owners = self.owners.clone();
            // Covers interrupted construction or process-runtime shutdown. A
            // nested Tokio runtime cannot be dropped on an async executor thread.
            std::thread::spawn(move || {
                drop(runtime);
                drop(device);
                drop(owners);
            });
        }
    }
}
