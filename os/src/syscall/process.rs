//! Process management syscalls
use alloc::sync::Arc;

use crate::{
    config::MAX_SYSCALL_NUM, mm::{MapPermission,VirtAddr},
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, TaskStatus,
    },
};
use crate::mm::translated_byte_buffer;
use crate::task::{get_syscall_time, get_task_runtime,free_in_page};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    // The point is: this _ts is not the physical address used to be
    // So must translate it from virtual to physical
    let ts = translated_refmut(current_user_token(), _ts);
    let usec = get_time_us();
    (*ts).sec = usec / 1_000_000;
    (*ts).usec = usec % 1_000_000;

    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
      // Careful to cal the size of TaskInfo
      let run_time:usize = get_task_runtime();
      let syscall_times = get_syscall_time();
      // status represent the TaskStatus:Running
      let task_info = TaskInfo {
          status:TaskStatus::Running,
          syscall_times:syscall_times,
          time:run_time,
      };
      let bytes: [u8; core::mem::size_of::<TaskInfo>()] =
          unsafe { core::mem::transmute(task_info) };
  
      let mut buffer = translated_byte_buffer(current_user_token(), _ti as *const u8, core::mem::size_of::<TaskInfo>());
      // Due with the situation of splitted over two pages
      let size = buffer[0].len();
      if size < bytes.len() {
          buffer[0].copy_from_slice(&bytes[..size]);
          buffer[1][..(bytes.len() - size)].copy_from_slice(&bytes[size..]);
      } else {
          buffer[0][..bytes.len()].copy_from_slice(&bytes[..]);
      }
      0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    let start = VirtAddr::from(_start);
    let end:VirtAddr = VirtAddr::from(_start + _len).ceil().into();

    // illeagal
    if !VirtAddr::from(_start).aligned() 
    || _port & !0b111 != 0 
    || _port & 0x7 == 0 
    || free_in_page(start.floor(), end.ceil()){
        return -1;
    }
    let mut permission = MapPermission::U;
    if _port & 0b001 != 0 {
        permission |= MapPermission::R;
    }
    if _port & 0b010 != 0 {
        permission |= MapPermission::W;
    }
    if _port & 0b100 != 0 {
        permission |= MapPermission::X;
    }
    
    // Insert this area
    let current_task = current_task().unwrap();
    current_task.inner_exclusive_access().memory_set.insert_framed_area(start, end, permission);
    drop(current_task);
    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    // println!("uuuumap!!!");

    let start = VirtAddr::from(_start).floor();
    let end = VirtAddr::from(_start + _len).ceil();

    if !VirtAddr::from(_start).aligned(){
        return -1;
    }

    let current_task = current_task().unwrap();

    // Exit the page that not be uesd
    for vpn in start.0..end.0 {
        if let Some(pte) = current_task.inner_exclusive_access().memory_set.translate(vpn.into()) {
            if !pte.is_valid() {
                return -1;
            }
        }
        else{
            return -1;
        } 
    }

    current_task.inner_exclusive_access().memory_set.unmap_frame_area(start, end);
    drop(current_task);
    0    
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    // trace!(
    //     "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
    //     current_task().unwrap().pid.0
    // );
    // trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, _path);
    if let Some(data) = get_app_data_by_name(&path) {
        let task = current_task().unwrap();
        let spawn = task.spawn(data);
        spawn.inner_exclusive_access().parent = Some(Arc::downgrade(&task));
        task.inner_exclusive_access().children.push(spawn.clone());
        let pid = spawn.pid.0;
        add_task(spawn);
        pid as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    // trace!(
    //     "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
    //     current_task().unwrap().pid.0
    // );
    if _prio <= 1 {
        return -1;
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.priority = _prio as usize;
    _prio
}
