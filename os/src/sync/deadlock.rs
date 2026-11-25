//! Resource snapshots for the per-process deadlock detector.
//!
//! Lab5 implementation assisted by Codex. Algorithm from
//! https://learningos.cn/rCore-Tutorial-Guide/chapter8/5exercise.html

use crate::task::TaskControlBlock;
use alloc::{collections::BTreeMap, sync::Arc, vec, vec::Vec};

type TaskId = *const TaskControlBlock;

#[derive(Default)]
/// State of one resource when an acquisition is proposed.
pub struct ResourceState {
    /// Units not currently granted to a thread.
    pub available: usize,
    /// Units granted to each owner.
    pub owners: Vec<(TaskId, usize)>,
    /// Threads currently requesting one unit.
    pub waiters: Vec<TaskId>,
}

/// Try the new request in a snapshot. Return true if this thread cannot finish.
pub fn would_deadlock(
    tasks: &[Option<Arc<TaskControlBlock>>],
    resources: &[ResourceState],
    current: &Arc<TaskControlBlock>,
    requested: usize,
    permits_can_be_signalled: bool,
) -> bool {
    let mut threads = BTreeMap::new();
    for task in tasks.iter().flatten() {
        if task.inner_exclusive_access().exit_code.is_none() {
            let index = threads.len();
            threads.insert(Arc::as_ptr(task), index);
        }
    }
    let current = threads[&Arc::as_ptr(current)];
    let mut allocation = vec![vec![0; resources.len()]; threads.len()];
    let mut need = vec![None; threads.len()];
    let mut work = Vec::with_capacity(resources.len());
    for (id, resource) in resources.iter().enumerate() {
        // An unowned semaphore may be an event. A producer can signal it
        // without first obtaining a permit, so an empty event is not a cycle.
        let live_owner = resource
            .owners
            .iter()
            .any(|(task, _)| threads.contains_key(task));
        work.push(if permits_can_be_signalled && !live_owner {
            resource.available.max(1)
        } else {
            resource.available
        });
        for (task, held) in &resource.owners {
            if let Some(&index) = threads.get(task) {
                allocation[index][id] = *held;
            }
        }
        for task in &resource.waiters {
            if let Some(&index) = threads.get(task) {
                need[index] = Some(id);
            }
        }
    }
    need[current] = Some(requested);

    let mut finished = vec![false; threads.len()];
    loop {
        let Some(index) = (0..threads.len())
            .find(|&i| !finished[i] && need[i].map_or(true, |resource| work[resource] > 0))
        else {
            return true;
        };
        if index == current {
            return false;
        }
        // The simulated thread takes one unit and returns that same unit
        // when it completes, so only its existing allocation is added.
        finished[index] = true;
        for (count, &held) in work.iter_mut().zip(&allocation[index]) {
            *count = count.saturating_add(held);
        }
    }
}
