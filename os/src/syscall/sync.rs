use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    // println!("HERE MUTEX CREATE");
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_status.push(1);
        for task in process_inner.tasks.iter() {
            if let Some(task) = task {
                let mut task_inner = task.inner_exclusive_access();
                let task_res = task_inner.res.as_mut().unwrap();
                task_res.mutex_need.push(0);
                task_res.mutex_have.push(0);
            }
        }
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    if process_inner.deadlock_detect_enable {
        // init all 
        {
            let current_task = current_task().unwrap();
            let mut current_task_inner = current_task.inner_exclusive_access();
            let current_task_res = current_task_inner.res.as_mut().unwrap();
            current_task_res.mutex_need[mutex_id] += 1; 
        }
       let tasks_len = process_inner.tasks.len();
        let mut finish = vec![false;tasks_len];
        let mut work = process_inner.mutex_status.clone();

        for _ in 0..tasks_len  {
            let mut flag = false;
            // every round, find unless one task that can finish
            for i in 0..tasks_len {
                if let Some(task) = &process_inner.tasks[i] {
                    if finish[i] {
                        continue;
                    }
                    {
                        let tmp_task_inner = task.inner_exclusive_access();
                        if let Some(tmp_task_res) = tmp_task_inner.res.as_ref() {
                            if  tmp_task_res.mutex_need.iter().zip(work.iter()).all(|(a,b)| a <= b) {
                                finish[i] = true;
                                for (work, have) in work.iter_mut().zip(tmp_task_res.mutex_have.iter()) {
                                    *work += have;
                                } 
                                flag = true;
                                
                        } else {
                            continue;
                        }
                       }
                    }
                }
                
            }
            // if no task can finish, just break
            if flag == false {
                break;
            }
        }
        // println!("{:?}",finish);
        // println!("worck : {:?}",work);
        // println!("mutex : {:?}",process_inner.mutex_status);
        if finish.iter().any(|&x| x == false) {
            let current_task = current_task().unwrap();
            let mut current_task_inner = current_task.inner_exclusive_access();
            let current_task_res = current_task_inner.res.as_mut().unwrap();
            current_task_res.mutex_need[mutex_id] -= 1;
            return -0xDEAD;
        }
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    mutex.lock();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.mutex_status[mutex_id] -= 1;
    let current_task = current_task().unwrap();
    let mut current_task_inner = current_task.inner_exclusive_access();
    let current_task_res = current_task_inner.res.as_mut().unwrap();
    current_task_res.mutex_have[mutex_id] += 1;
    current_task_res.mutex_need[mutex_id] -= 1;

    0
    
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    let current_task = current_task().unwrap();
    let mut current_task_inner = current_task.inner_exclusive_access();
    let current_task_res = current_task_inner.res.as_mut().unwrap();
    process_inner.mutex_status[mutex_id] += 1;
    current_task_res.mutex_have[mutex_id] -= 1;
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
   
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_status.push(res_count as i32);
        for task in process_inner.tasks.iter() {
            if let Some(task) = task {
                let mut task_inner = task.inner_exclusive_access();
                let task_res = task_inner.res.as_mut().unwrap();
                task_res.semaphore_need.push(0);
                task_res.semaphore_have.push(0);
            }
        }
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    // println!("HERE SEM UP");
    let process = current_process();
    let mut  process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    process_inner.semaphore_status[sem_id] += 1;
    let current_task = current_task().unwrap();
    let mut current_task_inner = current_task.inner_exclusive_access();
    let current_task_res = current_task_inner.res.as_mut().unwrap();
    current_task_res.semaphore_have[sem_id] -= 1; 
    drop(current_task_inner);
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    if process_inner.deadlock_detect_enable {
        {
            let current_task = current_task().unwrap();
            let mut current_task_inner = current_task.inner_exclusive_access();
            let current_task_res = current_task_inner.res.as_mut().unwrap();
            current_task_res.semaphore_need[sem_id] += 1; 
        }
        let mut finish = vec![false;process_inner.tasks.len()];
        let mut work = process_inner.semaphore_status.clone();
        let tasks_len = process_inner.tasks.len();
        // println!("sim_id:{}, work:{:?}",sem_id,work);
        for _ in 0..tasks_len  {
            let mut flag = false;
            // every round, find at less one task that can finish
            for i in 0..tasks_len {
                if let Some(task) = &process_inner.tasks[i] {
                    if finish[i] {
                        continue;
                    }
                    {
                        let tmp_task_inner = task.inner_exclusive_access();
                        // if i == 2 && sem_id == 2 && process_inner.semaphore_status==vec![1,1,0] {
                        //     tmp_task_inner.res.as_ref().unwrap();
                        // }
                        if let Some(tmp_task_res) = tmp_task_inner.res.as_ref() {
                            // if sem_id == 2 && process_inner.semaphore_status==vec![1,1,0] {
                            //     println!("here:{:?},{:?}{:?}",work,tmp_task_res.semaphore_need,i);
                            // }
                            if  tmp_task_res.semaphore_need.iter().zip(work.iter()).all(|(a,b)| a <= b) {
                                finish[i] = true;
                                for (work, have) in work.iter_mut().zip(tmp_task_res.semaphore_have.iter()) {
                                    *work += have;
                                } 
                                flag = true;
                        } 
                       } else {
                            finish[i] = true;
                            continue;
                        }
                    }
                }
                
            }
            if flag == false {
                break;
            } 
        } 
        // println!("{:?}",finish);
        if finish.iter().any(|&x| x == false) {
            let current_task = current_task().unwrap();
            let mut current_task_inner = current_task.inner_exclusive_access();
            let current_task_res = current_task_inner.res.as_mut().unwrap();
            current_task_res.semaphore_need[sem_id] -= 1;
            return -0xDEAD;
        }
    }
    
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.down();
    let current_task = current_task().unwrap();
    let mut current_task_inner = current_task.inner_exclusive_access();
    let current_task_res = current_task_inner.res.as_mut().unwrap();
    let mut process_inner = process.inner_exclusive_access();
    current_task_res.semaphore_have[sem_id] += 1;
    current_task_res.semaphore_need[sem_id] -= 1;
    process_inner.semaphore_status[sem_id] -= 1;
    // println!("{:?},{:?}",current_task_res.semaphore_have,current_task_res.semaphore_need);
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(_enabled: usize) -> isize {
    if _enabled == 1 {
        let process = current_process();
        let mut process_inner = process.inner_exclusive_access();
        process_inner.deadlock_detect_enable = true;
        0
    } else if _enabled == 0 {
        let process = current_process();
        let mut process_inner = process.inner_exclusive_access();
        process_inner.deadlock_detect_enable = false;
        0
    } else {
        -1
    }
}
