#![allow(dead_code)]

use arceos_posix_api as api;
use arceos_posix_api::get_file_like;
use axerrno::LinuxError;
use axhal::arch::TrapFrame;
use axhal::mem::MemoryAddr;
use axhal::mem::VirtAddr;
use axhal::paging::MappingFlags;
use axhal::trap::{register_trap_handler, PAGE_FAULT, SYSCALL};
use axmm::kernel_aspace;
use axtask::current;
use axtask::TaskExtRef;
use core::ffi::{c_char, c_int, c_void};
use memory_addr::VirtAddrRange;

const SYS_IOCTL: usize = 29;
const SYS_OPENAT: usize = 56;
const SYS_CLOSE: usize = 57;
const SYS_READ: usize = 63;
const SYS_WRITE: usize = 64;
const SYS_WRITEV: usize = 66;
const SYS_EXIT: usize = 93;
const SYS_EXIT_GROUP: usize = 94;
const SYS_SET_TID_ADDRESS: usize = 96;
const SYS_MMAP: usize = 222;

const AT_FDCWD: i32 = -100;

/// Macro to generate syscall body
///
/// It will receive a function which return Result<_, LinuxError> and convert it to
/// the type which is specified by the caller.
#[macro_export]
macro_rules! syscall_body {
    ($fn: ident, $($stmt: tt)*) => {{
        #[allow(clippy::redundant_closure_call)]
        let res = (|| -> axerrno::LinuxResult<_> { $($stmt)* })();
        match res {
            Ok(_) | Err(axerrno::LinuxError::EAGAIN) => debug!(concat!(stringify!($fn), " => {:?}"),  res),
            Err(_) => info!(concat!(stringify!($fn), " => {:?}"), res),
        }
        match res {
            Ok(v) => v as _,
            Err(e) => {
                -e.code() as _
            }
        }
    }};
}

bitflags::bitflags! {
    #[derive(Debug)]
    /// permissions for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    struct MmapProt: i32 {
        /// Page can be read.
        const PROT_READ = 1 << 0;
        /// Page can be written.
        const PROT_WRITE = 1 << 1;
        /// Page can be executed.
        const PROT_EXEC = 1 << 2;
    }
}

impl From<MmapProt> for MappingFlags {
    fn from(value: MmapProt) -> Self {
        let mut flags = MappingFlags::USER;
        if value.contains(MmapProt::PROT_READ) {
            flags |= MappingFlags::READ;
        }
        if value.contains(MmapProt::PROT_WRITE) {
            flags |= MappingFlags::WRITE;
        }
        if value.contains(MmapProt::PROT_EXEC) {
            flags |= MappingFlags::EXECUTE;
        }
        flags
    }
}

bitflags::bitflags! {
    #[derive(Debug)]
    /// flags for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    struct MmapFlags: i32 {
        /// Share changes
        const MAP_SHARED = 1 << 0;
        /// Changes private; copy pages on write.
        const MAP_PRIVATE = 1 << 1;
        /// Map address must be exactly as requested, no matter whether it is available.
        const MAP_FIXED = 1 << 4;
        /// Don't use a file.
        const MAP_ANONYMOUS = 1 << 5;
        /// Don't check for reservations.
        const MAP_NORESERVE = 1 << 14;
        /// Allocation is for a stack.
        const MAP_STACK = 0x20000;
    }
}

#[register_trap_handler(SYSCALL)]
fn handle_syscall(tf: &TrapFrame, syscall_num: usize) -> isize {
    ax_println!("handle_syscall [{}] ...", syscall_num);
    let ret = match syscall_num {
        SYS_IOCTL => sys_ioctl(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _) as _,
        SYS_SET_TID_ADDRESS => sys_set_tid_address(tf.arg0() as _),
        SYS_OPENAT => sys_openat(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
        ),
        SYS_CLOSE => sys_close(tf.arg0() as _),
        SYS_READ => sys_read(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_WRITE => sys_write(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_WRITEV => sys_writev(tf.arg0() as _, tf.arg1() as _, tf.arg2() as _),
        SYS_EXIT_GROUP => {
            ax_println!("[SYS_EXIT_GROUP]: system is exiting ..");
            axtask::exit(tf.arg0() as _)
        }
        SYS_EXIT => {
            ax_println!("[SYS_EXIT]: system is exiting ..");
            axtask::exit(tf.arg0() as _)
        }
        SYS_MMAP => sys_mmap(
            tf.arg0() as _,
            tf.arg1() as _,
            tf.arg2() as _,
            tf.arg3() as _,
            tf.arg4() as _,
            tf.arg5() as _,
        ),
        _ => {
            ax_println!("Unimplemented syscall: {}", syscall_num);
            -LinuxError::ENOSYS.code() as _
        }
    };
    ret
}

#[register_trap_handler(PAGE_FAULT)]
fn page_fault(vaddr: VirtAddr, access_flags: MappingFlags, is_user: bool) -> bool {
    if !is_user {
        panic!("kernel page fault");
    } else {
        ax_println!("handler user page fault");
        axtask::current()
            .task_ext()
            .aspace
            .lock()
            .handle_page_fault(vaddr, access_flags)
    }
}

#[allow(unused_variables)]
fn sys_mmap(
    addr: *mut usize,
    length: usize,
    prot: i32,
    flags: i32,
    fd: i32,
    _offset: isize,
) -> isize {
    let current = current();
    let mut uspace = current.task_ext().aspace.lock();
    ax_println!("uspace base: {:#x}", uspace.base());
    ax_println!("uspace size: {:#x}", uspace.size());
    let Some(vaddr) = uspace.find_free_area(
        VirtAddr::from(addr as usize + 0x10_0000),
        length.align_up_4k(),
        VirtAddrRange::from_start_size(uspace.base(), uspace.size()),
    ) else {
        ax_println!("find_free_area failed");
        return 0;
    };

    let flags = MmapProt::from_bits_truncate(prot);
    if let Err(e) = uspace.map_alloc(vaddr, length.align_up_4k(), flags.into(), true) {
        ax_println!("map_alloc failed: {}", e);
        return 0;
    }
    ax_println!("map_alloc success: vaddr={:#x}", vaddr);

    let file = match get_file_like(fd) {
        Ok(file) => file,
        Err(e) => {
            ax_println!("get_file_like failed: {}", e);
            return 0;
        }
    };
    ax_println!("file: {}", fd);

    let mut buf = alloc::vec![0; length];
    if let Err(e) = file.read(&mut buf) {
        ax_println!("read failed: {}", e);
        return 0;
    };
    ax_println!("read success: buf.len={}", buf.len());

    if let Err(e) = uspace.write(vaddr, &buf) {
        ax_println!("write failed: {}", e);
        return 0;
    };
    ax_println!("write success: vaddr={:#x}", vaddr);

    vaddr.as_usize() as isize
}

fn sys_openat(dfd: c_int, fname: *const c_char, flags: c_int, mode: api::ctypes::mode_t) -> isize {
    assert_eq!(dfd, AT_FDCWD);
    api::sys_open(fname, flags, mode) as isize
}

fn sys_close(fd: i32) -> isize {
    api::sys_close(fd) as isize
}

fn sys_read(fd: i32, buf: *mut c_void, count: usize) -> isize {
    api::sys_read(fd, buf, count)
}

fn sys_write(fd: i32, buf: *const c_void, count: usize) -> isize {
    api::sys_write(fd, buf, count)
}

fn sys_writev(fd: i32, iov: *const api::ctypes::iovec, iocnt: i32) -> isize {
    unsafe { api::sys_writev(fd, iov, iocnt) }
}

fn sys_set_tid_address(tid_ptd: *const i32) -> isize {
    let curr = current();
    curr.task_ext().set_clear_child_tid(tid_ptd as _);
    curr.id().as_u64() as isize
}

fn sys_ioctl(_fd: i32, _op: usize, _argp: *mut c_void) -> i32 {
    ax_println!("Ignore SYS_IOCTL");
    0
}