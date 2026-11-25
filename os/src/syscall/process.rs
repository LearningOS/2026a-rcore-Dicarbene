//! Process management syscalls
use crate::mm::MapPermission;
use crate::task::{
    change_program_brk, current_syscall_count, exit_current_and_run_next,
    suspend_current_and_run_next, with_current_memory_set,
};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
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

/// Copy the time through checked user mappings, including across page boundaries.
/// Lab2 implementation assisted by Codex; interfaces follow:
/// https://learningos.cn/rCore-Tutorial-Guide/chapter4/7exercise.html
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let time = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    // SAFETY: repr(C) with two initialized usize fields has no padding.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            &time as *const TimeVal as *const u8,
            core::mem::size_of::<TimeVal>(),
        )
    };
    if with_current_memory_set(|memory| memory.copy_to_user(ts as usize, bytes)) {
        0
    } else {
        -1
    }
}

/// Trace bytes only through valid user mappings with the requested permission.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        0 | 1 => with_current_memory_set(|memory| {
            let permission = if trace_request == 0 {
                MapPermission::R
            } else {
                MapPermission::W
            };
            match memory.translate_user_byte(id, permission) {
                Some(address) => {
                    // SAFETY: translation checked V, U and R/W; the current task
                    // owns the frame and cannot be scheduled out in this closure.
                    if trace_request == 0 {
                        unsafe { core::ptr::read_volatile(address.0 as *const u8) as isize }
                    } else {
                        unsafe { core::ptr::write_volatile(address.0 as *mut u8, data as u8) };
                        0
                    }
                }
                None => -1,
            }
        }),
        2 => current_syscall_count(id),
        _ => -1,
    }
}

/// Allocate zeroed anonymous pages; prot uses bits 0..2 rather than PTE bits 1..3.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    if prot == 0 || prot & !0x7 != 0 {
        return -1;
    }
    let permission = MapPermission::from_bits((prot << 1) as u8).unwrap() | MapPermission::U;
    if with_current_memory_set(|memory| memory.map_user_range(start, len, permission)) {
        0
    } else {
        -1
    }
}

/// Validate the whole range before removing mappings or releasing frames.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    if with_current_memory_set(|memory| memory.unmap_user_range(start, len)) {
        0
    } else {
        -1
    }
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
