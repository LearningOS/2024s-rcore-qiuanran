//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use core::usize::MAX;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
use crate::config::BIG_STRIDE;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }

        // Find the min stride in the ready queue use 
        let (mut min_stride, mut idx) = (MAX, 0);

        for (i,task) in self.ready_queue.iter().enumerate() {
            let inner = task.inner_exclusive_access();
            let stride = inner.stride;
            let status = inner.task_status;
            drop(inner);
            if status != super::TaskStatus::Ready {
                continue;
            }
            if stride < min_stride {
                min_stride = stride;
                idx = i;
            }
        } 

        if min_stride == MAX {
            return None;
        }

        let task = self.ready_queue.remove(idx).unwrap();
        let mut inner = task.inner_exclusive_access();
        inner.stride += BIG_STRIDE / inner.priority as usize;
        drop(inner);
        Some(task)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
