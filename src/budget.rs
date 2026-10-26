use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{Error, Result};

/// Hard byte admission for payload allocations. Metadata, runtime, SDK and kernel
/// overhead are not covered and must be measured separately, not called bounded RSS.
#[derive(Clone)]
pub struct Budget {
    semaphore: Arc<Semaphore>,
    limit: usize,
}

impl Budget {
    pub fn try_reserve(&self, size: usize) -> Result<Reservation> {
        if size > self.limit || size > u32::MAX as usize {
            return Err(Error::Budget {
                requested: size,
                limit: self.limit.min(u32::MAX as usize),
            });
        }
        let permit = self
            .semaphore
            .clone()
            .try_acquire_many_owned(size as u32)
            .map_err(|_| Error::Fs(libc::ENOMEM))?;
        Ok(Reservation {
            _permit: permit,
            size,
        })
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn new(limit: usize) -> Result<Self> {
        if limit == 0 || limit > Semaphore::MAX_PERMITS {
            return Err(Error::Config(
                "byte budget must fit positive semaphore capacity".into(),
            ));
        }
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(limit)),
            limit,
        })
    }

    pub async fn reserve(&self, size: usize) -> Result<Reservation> {
        if size > self.limit || size > u32::MAX as usize {
            return Err(Error::Budget {
                requested: size,
                limit: self.limit.min(u32::MAX as usize),
            });
        }
        let permit = self
            .semaphore
            .clone()
            .acquire_many_owned(size as u32)
            .await
            .map_err(|_| Error::Cancelled)?;
        Ok(Reservation {
            _permit: permit,
            size,
        })
    }

    pub fn used(&self) -> usize {
        self.limit - self.semaphore.available_permits()
    }

    pub fn close(&self) {
        self.semaphore.close();
    }
}

pub struct Reservation {
    _permit: OwnedSemaphorePermit,
    size: usize,
}

/// Cloning a block pins the same allocation and its permit. No subslice escapes
/// with a misleading smaller charge; callers copy through as_slice when needed.
#[derive(Clone)]
pub struct Block(Arc<Allocation>);

struct Allocation {
    data: Box<[u8]>,
    _reservation: Reservation,
    _owner: Option<Reservation>,
}

impl Reservation {
    pub(crate) fn split(&mut self, size: usize) -> Self {
        assert!(
            size <= self.size,
            "split is bounded by existing byte admission"
        );
        self.size -= size;
        Self {
            _permit: self
                ._permit
                .split(size)
                .expect("permit count equals reservation"),
            size,
        }
    }
    pub fn zeroed(self) -> Block {
        let data = vec![0; self.size].into_boxed_slice();
        Block(Arc::new(Allocation {
            data,
            _reservation: self,
            _owner: None,
        }))
    }

    pub fn copy(self, bytes: &[u8]) -> Result<Block> {
        if bytes.len() != self.size {
            return Err(Error::Config("reservation length mismatch".into()));
        }
        Ok(Block(Arc::new(Allocation {
            data: bytes.into(),
            _reservation: self,
            _owner: None,
        })))
    }
}

impl Block {
    pub(crate) fn shrink(&mut self, length: usize) {
        let allocation = Arc::get_mut(&mut self.0).expect("unshared output block");
        let mut bytes = std::mem::take(&mut allocation.data).into_vec();
        bytes.truncate(length);
        allocation.data = bytes.into_boxed_slice();
    }
    pub(crate) fn with_owner(mut self, owner: Reservation) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("new allocation is exclusively owned")
            ._owner = Some(owner);
        self
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.0.data
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut Arc::get_mut(&mut self.0)
            .expect("new block must not be shared during fill")
            .data
    }
}

impl AsRef<[u8]> for Block {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}
