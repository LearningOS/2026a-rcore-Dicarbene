//! Counting semaphore and ownership of acquired permits.

use super::{ResourceState, UPSafeCell};
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
    vec::Vec,
};

/// A counting semaphore with a FIFO queue of blocked callers.
pub struct Semaphore {
    inner: UPSafeCell<SemaphoreInner>,
}

struct SemaphoreInner {
    available: usize,
    owners: Vec<(Weak<TaskControlBlock>, usize)>,
    waiters: VecDeque<Weak<TaskControlBlock>>,
}

impl SemaphoreInner {
    fn grant(&mut self, task: Weak<TaskControlBlock>) {
        self.owners.retain(|(owner, _)| owner.strong_count() != 0);
        if let Some((_, count)) = self
            .owners
            .iter_mut()
            .find(|(owner, _)| owner.ptr_eq(&task))
        {
            *count += 1;
        } else {
            self.owners.push((task, 1));
        }
    }

    fn release(&mut self, task: &Weak<TaskControlBlock>) {
        if let Some(index) = self.owners.iter().position(|(owner, _)| owner.ptr_eq(task)) {
            self.owners[index].1 -= 1;
            if self.owners[index].1 == 0 {
                self.owners.swap_remove(index);
            }
        }
    }
}

impl Semaphore {
    /// Construct a semaphore with initial permits.
    pub fn new(res_count: usize) -> Self {
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    available: res_count,
                    owners: Vec::new(),
                    waiters: VecDeque::new(),
                })
            },
        }
    }

    /// Release or signal a permit, handing it to a waiter if present.
    pub fn up(&self) {
        let mut inner = self.inner.exclusive_access();
        inner.release(&Arc::downgrade(&current_task().unwrap()));
        if let Some(task) =
            core::iter::from_fn(|| inner.waiters.pop_front()).find_map(|waiter| waiter.upgrade())
        {
            inner.grant(Arc::downgrade(&task));
            wakeup_task(task);
        } else {
            inner.available = inner.available.saturating_add(1);
        }
    }

    /// Acquire one permit, blocking if none is available.
    pub fn down(&self) {
        let task = Arc::downgrade(&current_task().unwrap());
        let mut inner = self.inner.exclusive_access();
        if inner.available == 0 {
            inner.waiters.push_back(task);
            drop(inner);
            block_current_and_run_next();
        } else {
            inner.available -= 1;
            inner.grant(task);
        }
    }

    /// Snapshot counts, owners and waiters.
    pub fn state(&self) -> ResourceState {
        let inner = self.inner.exclusive_access();
        ResourceState {
            available: inner.available,
            owners: inner
                .owners
                .iter()
                .map(|(owner, count)| (owner.as_ptr(), *count))
                .collect(),
            waiters: inner.waiters.iter().map(Weak::as_ptr).collect(),
        }
    }
}
