use std::{ops::Range, sync::Arc};

use tokio::sync::{OwnedRwLockReadGuard, OwnedSemaphorePermit, Semaphore};

use crate::error::{Error, Result};

/// Hard byte admission for payload allocations. Metadata, runtime, SDK and kernel
/// overhead are not covered and must be measured separately, not called bounded RSS.
#[derive(Clone)]
pub struct Budget {
    semaphore: Arc<Semaphore>,
    limit: usize,
    _activity: Option<Arc<OwnedRwLockReadGuard<()>>>,
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
            _activity: None,
        })
    }

    /// Keep upload activity alive in cloned bodies/jobs after caller cancellation.
    pub(crate) fn with_activity(&self, activity: OwnedRwLockReadGuard<()>) -> Self {
        let mut budget = self.clone();
        budget._activity = Some(Arc::new(activity));
        budget
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

/// Clones and immutable views retain the complete allocation and its permits.
/// The visible range never replaces its backing allocation's ownership charge.
#[derive(Clone)]
pub struct Block {
    allocation: Arc<Allocation>,
    range: Range<usize>,
}

struct Allocation {
    data: Box<[u8]>,
    _reservation: Reservation,
    _owner: Option<Reservation>,
    loaned: bool,
}

impl Reservation {
    pub(crate) fn size(&self) -> usize {
        self.size
    }

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
        Block::owned(data, self)
    }

    pub(crate) async fn read_exact(
        self,
        reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
    ) -> std::io::Result<Block> {
        use tokio::io::AsyncReadExt;

        let mut data = Vec::with_capacity(self.size);
        let copied = tokio::io::copy_buf(&mut reader.take(self.size as u64), &mut data).await?;
        if copied != self.size as u64 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        Ok(Block::owned(data.into_boxed_slice(), self))
    }

    pub fn copy(self, bytes: &[u8]) -> Result<Block> {
        if bytes.len() != self.size {
            return Err(Error::Config("reservation length mismatch".into()));
        }
        Ok(Block::owned(bytes.into(), self))
    }
}

impl Block {
    fn owned(data: Box<[u8]>, reservation: Reservation) -> Self {
        let range = 0..data.len();
        Self {
            allocation: Arc::new(Allocation {
                data,
                _reservation: reservation,
                _owner: None,
                loaned: false,
            }),
            range,
        }
    }

    /// Share a visible range while retaining the whole backing allocation charge.
    /// Like ordinary slice indexing, an out-of-bounds range panics.
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(
            self.as_slice().get(range.clone()).is_some(),
            "block view must be within its visible range"
        );
        Self {
            allocation: self.allocation.clone(),
            range: self.range.start + range.start..self.range.start + range.end,
        }
    }

    pub(crate) fn with_owner(mut self, owner: Reservation) -> Self {
        let allocation =
            Arc::get_mut(&mut self.allocation).expect("new allocation is exclusively owned");
        assert_eq!(self.range, 0..allocation.data.len());
        allocation._owner = Some(owner);
        self
    }

    pub(crate) fn is_loaned(&self) -> bool {
        self.allocation.loaned
    }

    /// Transfer a complete, exclusively cache-owned backing; never add capacity.
    pub(crate) fn try_loan(&mut self, lane: &Budget) -> bool {
        let Some(allocation) = Arc::get_mut(&mut self.allocation) else {
            return false;
        };
        let Some(owner) = allocation._owner.as_mut() else {
            return false;
        };
        if allocation.loaned || self.range != (0..allocation.data.len()) {
            return false;
        }
        assert_eq!(owner.size(), allocation.data.len());
        let Ok(loan) = lane.try_reserve(owner.size()) else {
            return false;
        };
        *owner = loan;
        allocation.loaned = true;
        true
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.allocation.data[self.range.clone()]
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut Arc::get_mut(&mut self.allocation)
            .expect("new block must not be shared during fill")
            .data[self.range.clone()]
    }
}

impl AsRef<[u8]> for Block {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}
