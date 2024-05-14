//! Process management syscalls
use crate::{
    config::MAX_SYSCALL_NUM, mm::{MapPermission, VirtAddr}, task::{
        change_program_brk, exit_current_and_run_next, suspend_current_and_run_next, TaskStatus, 
    }
};
use crate::mm::translated_byte_buffer;
use crate::task::current_user_token;
use crate::timer::get_time_us;
use alloc::vec::Vec;
use crate::task::{get_syscall_time, get_task_runtime,get_current_tcb};

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
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    // The point is: this _ts is not the physical address used to be
    // So must translate it from virtual to physical
    let us = get_time_us();
    let sec = us / 1_000_000;
    let usec = us % 1_000_000;
    let mut bytes:Vec<u8> = Vec::new();
    bytes.extend_from_slice(&sec.to_le_bytes());
    bytes.extend_from_slice(&usec.to_le_bytes());
    
    let mut buffer = translated_byte_buffer(current_user_token(), _ts as *const u8,core::mem::size_of::<TimeVal>());
    // Due with the situation of splitted over two pages
    let size = buffer[0].len();
    if size < bytes.len() {
        buffer[0].copy_from_slice(&bytes[..size]);
        buffer[1][..(bytes.len() - size)].copy_from_slice(&bytes[size..]);
    } else {
        buffer[0][..bytes.len()].copy_from_slice(&bytes[..]);
    }
    trace!("kernel: sys_get_time");
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

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    let start = VirtAddr::from(_start);
    let end:VirtAddr = VirtAddr::from(_start + _len).ceil().into();

    let current_task = get_current_tcb();
    // illeagal
    if !VirtAddr::from(_start).aligned() 
    || _port & !0b111 != 0 
    || _port & 0x7 == 0 
    || current_task.memory_set.free_in_range(start.floor(), end.ceil()){
        return -1;
    }
    // println!("map!!!*********************************start:{:?} end:{:?}",start.floor(), end.ceil());
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
    let current_task = get_current_tcb();
    current_task.memory_set.insert_framed_area(start, end, permission);

    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    // println!("uuuumap!!!");

    let start = VirtAddr::from(_start).floor();
    let end = VirtAddr::from(_start + _len).ceil();

    if !VirtAddr::from(_start).aligned(){
        return -1;
    }

    let current_task = get_current_tcb();

    // Exit the page that not be uesd
    for vpn in start.0..end.0 {
        if let Some(pte) = current_task.memory_set.translate(vpn.into()) {
            if !pte.is_valid() {
                return -1;
            }
        }
        else{
            return -1;
        } 
    }

    current_task.memory_set.unmap_frame_area(start, end);
    0    
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
