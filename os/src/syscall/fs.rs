//! File and filesystem-related syscalls
use crate::fs::{open_file, OpenFlags, Stat, ROOT_INODE};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// Return metadata using the course's C ABI, including when the output crosses a page.
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    let Some(stat) = inner
        .fd_table
        .get(fd)
        .and_then(Option::as_ref)
        .and_then(|file| file.stat())
    else {
        return -1;
    };
    // SAFETY: repr(C) fields fill all 80 bytes, and the reserved words are initialized.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&stat as *const Stat).cast::<u8>(),
            core::mem::size_of::<Stat>(),
        )
    };
    if inner.memory_set.copy_to_user(st as usize, bytes) {
        0
    } else {
        -1
    }
}

/// Give an existing inode an additional name in the root directory.
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    let read_name = |ptr| {
        inner
            .memory_set
            .read_user_string(ptr, easy_fs::NAME_LENGTH_LIMIT)
    };
    let (Some(old), Some(new)) = (read_name(old_name as usize), read_name(new_name as usize))
    else {
        return -1;
    };
    if ROOT_INODE.link(&old, &new).is_some() {
        0
    } else {
        -1
    }
}

/// Remove a root directory entry and release the inode once it has no references.
pub fn sys_unlinkat(name: *const u8) -> isize {
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    let Some(name) = inner
        .memory_set
        .read_user_string(name as usize, easy_fs::NAME_LENGTH_LIMIT)
    else {
        return -1;
    };
    if ROOT_INODE.unlink(&name).is_some() {
        0
    } else {
        -1
    }
}
