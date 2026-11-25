//! Mutexes with yielding or blocking waits.

use super::{ResourceState, UPSafeCell};
use crate::task::{
    block_current_and_run_next, current_task, suspend_current_and_run_next, wakeup_task,
    TaskControlBlock,
};
use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

/// Common interface for both mutex implementations.
pub trait Mutex: Sync + Send {
    /// Acquire the mutex.
    fn lock(&self);
    /// Release the mutex or transfer ownership.
    fn unlock(&self);
    /// Snapshot its owner and wait queue.
    fn state(&self) -> ResourceState;
}

#[derive(Default)]
struct MutexState {
    owner: Option<Weak<TaskControlBlock>>,
    waiters: VecDeque<Weak<TaskControlBlock>>,
}

impl MutexState {
    fn snapshot(&self) -> ResourceState {
        ResourceState {
            available: usize::from(self.owner.is_none()),
            owners: self.owner.iter().map(|owner| (owner.as_ptr(), 1)).collect(),
            waiters: self.waiters.iter().map(Weak::as_ptr).collect(),
        }
    }
}

/// A mutex that yields while waiting for the owner to unlock.
pub struct MutexSpin {
    inner: UPSafeCell<MutexState>,
}

impl MutexSpin {
    /// Construct an unlocked mutex.
    pub fn new() -> Self {
        Self {
            inner: unsafe { UPSafeCell::new(MutexState::default()) },
        }
    }
}

impl Mutex for MutexSpin {
    fn lock(&self) {
        loop {
            let task = Arc::downgrade(&current_task().unwrap());
            let mut inner = self.inner.exclusive_access();
            if inner.owner.is_none() {
                inner
                    .waiters
                    .retain(|waiting| !Weak::ptr_eq(waiting, &task));
                inner.owner = Some(task);
                return;
            }
            if !inner
                .waiters
                .iter()
                .any(|waiting| Weak::ptr_eq(waiting, &task))
            {
                inner.waiters.push_back(task);
            }
            drop(inner);
            suspend_current_and_run_next();
        }
    }

    fn unlock(&self) {
        self.inner.exclusive_access().owner = None;
    }

    fn state(&self) -> ResourceState {
        self.inner.exclusive_access().snapshot()
    }
}

/// A mutex that queues and blocks waiters.
pub struct MutexBlocking {
    inner: UPSafeCell<MutexState>,
}

impl MutexBlocking {
    /// Construct an unlocked mutex.
    pub fn new() -> Self {
        Self {
            inner: unsafe { UPSafeCell::new(MutexState::default()) },
        }
    }
}

impl Mutex for MutexBlocking {
    fn lock(&self) {
        let task = Arc::downgrade(&current_task().unwrap());
        let mut inner = self.inner.exclusive_access();
        if inner.owner.is_some() {
            inner.waiters.push_back(task);
            drop(inner);
            block_current_and_run_next();
        } else {
            inner.owner = Some(task);
        }
    }

    fn unlock(&self) {
        let mut inner = self.inner.exclusive_access();
        if let Some(task) =
            core::iter::from_fn(|| inner.waiters.pop_front()).find_map(|waiter| waiter.upgrade())
        {
            inner.owner = Some(Arc::downgrade(&task));
            wakeup_task(task);
        } else {
            inner.owner = None;
        }
    }

    fn state(&self) -> ResourceState {
        self.inner.exclusive_access().snapshot()
    }
}
