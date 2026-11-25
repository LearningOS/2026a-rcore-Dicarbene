//! Additional user-mode checks; copy into ci-user/user/src/bin before running the checker.
//! Implemented with Codex assistance, independently of the official test sources.
#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{mmap, munmap, syscall, trace_read, trace_write, SYSCALL_GETTIMEOFDAY};

const PAGE: usize = 4096;
const START: usize = 0x2000_0000;

fn read(address: usize) -> Option<u8> {
    trace_read(address as *const u8)
}

fn write(address: usize, byte: u8) {
    assert_eq!(trace_write(address as *const u8, byte), 0);
}

#[no_mangle]
fn main() -> i32 {
    assert_eq!(mmap(0, 0, 3), 0);
    assert_eq!(munmap(0, 0), 0);
    assert_eq!(mmap(START + 1, 0, 3), -1);
    assert_eq!(mmap(START, usize::MAX, 3), -1);
    assert_eq!(mmap(1usize << 39, PAGE, 3), -1);

    // Remove the middle of one area, then recreate the hole independently.
    assert_eq!(mmap(START, PAGE * 3, 3), 0);
    write(START, 11);
    write(START + PAGE * 2, 22);
    assert_eq!(munmap(START + PAGE, PAGE), 0);
    assert_eq!(read(START), Some(11));
    assert_eq!(read(START + PAGE), None);
    assert_eq!(read(START + PAGE * 2), Some(22));
    assert_eq!(mmap(START + PAGE, PAGE, 3), 0);
    assert_eq!(read(START + PAGE), Some(0));
    // Remove all three areas in one request.
    assert_eq!(munmap(START, PAGE * 3), 0);
    assert_eq!(munmap(START, PAGE), -1);

    // A failed unmap must preserve its mapped prefix.
    assert_eq!(mmap(START, PAGE, 3), 0);
    write(START, 33);
    assert_eq!(munmap(START, PAGE * 2), -1);
    assert_eq!(read(START), Some(33));
    assert_eq!(munmap(START, PAGE), 0);

    // TimeVal straddles two virtual pages backed by noncontiguous frames.
    assert_eq!(mmap(START, PAGE, 3), 0);
    assert_eq!(mmap(START + PAGE * 4, PAGE, 3), 0);
    assert_eq!(mmap(START + PAGE, PAGE, 3), 0);
    let time = START + PAGE - 8;
    assert_eq!(syscall(SYSCALL_GETTIMEOFDAY, [time, 0, 0]), 0);
    let usec = unsafe { ((START + PAGE) as *const usize).read_volatile() };
    assert!(usec < 1_000_000);
    assert_eq!(munmap(START + PAGE, PAGE), 0);
    for offset in 0..8 {
        write(time + offset, 0x5a);
    }
    assert_eq!(syscall(SYSCALL_GETTIMEOFDAY, [time, 0, 0]), -1);
    for offset in 0..8 {
        assert_eq!(read(time + offset), Some(0x5a));
    }
    assert_eq!(syscall(SYSCALL_GETTIMEOFDAY, [usize::MAX, 0, 0]), -1);
    // High bits must not alias a legitimate mapping after VirtAddr truncation.
    assert_eq!(read(START | (1usize << 39)), None);
    // The supervisor-only trampoline and trap context cannot be removed.
    assert_eq!(munmap(usize::MAX - PAGE + 1, PAGE), -1);
    assert_eq!(munmap(usize::MAX - PAGE * 2 + 1, PAGE), -1);
    assert_eq!(munmap(START, PAGE), 0);
    assert_eq!(munmap(START + PAGE * 4, PAGE), 0);

    assert_eq!(mmap(START, PAGE, 1), 0);
    assert_eq!(syscall(SYSCALL_GETTIMEOFDAY, [START, 0, 0]), -1);
    assert_eq!(trace_write(START as *const u8, 1), -1);
    assert_eq!(munmap(START, PAGE), 0);

    // Exceed the course's 128 MiB RAM, then check failure leaves reusable data frames.
    assert_eq!(mmap(START, 256 * 1024 * 1024, 3), -1);
    assert_eq!(read(START), None);
    for _ in 0..128 {
        assert_eq!(mmap(START, PAGE * 3, 3), 0);
        assert_eq!(read(START), Some(0));
        assert_eq!(munmap(START, PAGE * 3), 0);
    }
    println!("MEMORY EDGE CHECKS COMPLETE");
    0
}
