//! Process management syscalls
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::{
    loader::{get_app_data_by_name, max_app_name_len},
    mm::{translated_refmut, translated_str, MapPermission},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
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
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
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

/// Write the time through the current process's checked user mappings.
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    let us = get_time_us();
    let seconds = (us / 1_000_000).to_ne_bytes();
    let micros = (us % 1_000_000).to_ne_bytes();
    let mut bytes = [0; core::mem::size_of::<TimeVal>()];
    let (sec, usec) = bytes.split_at_mut(seconds.len());
    sec.copy_from_slice(&seconds);
    usec.copy_from_slice(&micros);
    let task = current_task().unwrap();
    if task
        .inner_exclusive_access()
        .memory_set
        .copy_to_user(ts as usize, &bytes)
    {
        0
    } else {
        -1
    }
}

/// Create an anonymous user mapping in the calling process.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    if prot == 0 || prot & !0x7 != 0 {
        return -1;
    }
    let permission = MapPermission::from_bits((prot << 1) as u8).unwrap() | MapPermission::U;
    let task = current_task().unwrap();
    if task
        .inner_exclusive_access()
        .memory_set
        .map_user_range(start, len, permission)
    {
        0
    } else {
        -1
    }
}

/// Remove user mappings from the calling process.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    let task = current_task().unwrap();
    if task
        .inner_exclusive_access()
        .memory_set
        .unmap_user_range(start, len)
    {
        0
    } else {
        -1
    }
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

/// Create a child from an embedded ELF. Lab3 implementation assisted by Codex.
/// https://learningos.cn/rCore-Tutorial-Guide/chapter5/4exercise.html
pub fn sys_spawn(path: *const u8) -> isize {
    let parent = current_task().unwrap();
    let Some(elf) = read_spawn_elf(&parent.inner_exclusive_access().memory_set, path as usize)
    else {
        return -1;
    };
    let child = parent.spawn(elf);
    let pid = child.getpid();
    add_task(child);
    pid as isize
}

fn read_spawn_elf(memory: &crate::mm::MemorySet, path: usize) -> Option<&'static [u8]> {
    let max_len = max_app_name_len();
    let mut name = Vec::with_capacity(max_len);
    // Also read the terminator when the name has the maximum legal length.
    for offset in 0..=max_len {
        let address = memory.translate_user_byte(path.checked_add(offset)?, MapPermission::R)?;
        // SAFETY: the process owns this readable frame throughout the call.
        let byte = unsafe { (address.0 as *const u8).read_volatile() };
        if byte == 0 {
            return get_app_data_by_name(core::str::from_utf8(&name).ok()?);
        }
        if offset == max_len {
            break;
        }
        name.push(byte);
    }
    None
}

/// Set the weight for future stride increments.
pub fn sys_set_priority(prio: isize) -> isize {
    if prio < 2 {
        return -1;
    }
    current_task().unwrap().inner_exclusive_access().priority = prio as usize;
    prio
}
