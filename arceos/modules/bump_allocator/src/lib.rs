#![no_std]

// extern crate axlog;

use allocator::{AllocError, BaseAllocator, ByteAllocator, PageAllocator};
// use axlog::info;
use core::ptr::NonNull;

/// Early memory allocator
/// Use it before formal bytes-allocator and pages-allocator can work!
/// This is a double-end memory range:
/// - Alloc bytes forward
/// - Alloc pages backward
///
/// [ bytes-used | avail-area | pages-used ]
/// |            | -->    <-- |            |
/// start       b_pos        p_pos       end
///
/// For bytes area, 'count' records number of allocations.
/// When it goes down to ZERO, free bytes-used area.
/// For pages area, it will never be freed!
///
pub struct EarlyAllocator<const PAGE_SIZE: usize> {
    byte_alloc_pos: usize,
    page_alloc_pos: usize,
    allocated_page_count: usize,
    memory_start: usize,
    memory_size: usize,
}

impl<const PAGE_SIZE: usize> EarlyAllocator<PAGE_SIZE> {
    pub const fn new() -> Self {
        Self {
            byte_alloc_pos: 0,
            page_alloc_pos: 0,
            allocated_page_count: 0,
            memory_start: 0,
            memory_size: 0,
        }
    }
}

impl<const PAGE_SIZE: usize> BaseAllocator for EarlyAllocator<PAGE_SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.byte_alloc_pos = start;
        self.page_alloc_pos = start + size;
        self.allocated_page_count = 0;
        self.memory_start = start;
        self.memory_size = size;
    }

    fn add_memory(&mut self, _start: usize, _size: usize) -> allocator::AllocResult {
        unimplemented!("EarlyAllocator does not support adding memory dynamically")
    }
}

impl<const PAGE_SIZE: usize> ByteAllocator for EarlyAllocator<PAGE_SIZE> {
    fn alloc(&mut self, layout: core::alloc::Layout) -> allocator::AllocResult<core::ptr::NonNull<u8>> {
        let aligned_pos = self.byte_alloc_pos.next_multiple_of(layout.align());
        let new_pos = aligned_pos + layout.size();
        
        if new_pos > self.page_alloc_pos {
            return Err(AllocError::NoMemory);
        }
        
        self.byte_alloc_pos = new_pos;
        unsafe { Ok(NonNull::new_unchecked(aligned_pos as *mut u8)) }
    }

    fn dealloc(&mut self, _pos: core::ptr::NonNull<u8>, _layout: core::alloc::Layout) {
        self.allocated_page_count -= 1;
        if self.allocated_page_count == 0 {
            self.byte_alloc_pos = self.memory_start;
        }
    }

    fn total_bytes(&self) -> usize {
        self.page_alloc_pos - self.byte_alloc_pos
    }

    fn used_bytes(&self) -> usize {
        self.page_alloc_pos - self.byte_alloc_pos
    }

    fn available_bytes(&self) -> usize {
        self.page_alloc_pos - self.byte_alloc_pos
    }
}

impl<const PAGE_SIZE: usize> PageAllocator for EarlyAllocator<PAGE_SIZE> {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, _align_pow2: usize) -> allocator::AllocResult<usize> {
        if self.allocated_page_count == 0 {
            self.page_alloc_pos -= num_pages * PAGE_SIZE;
            self.allocated_page_count = num_pages;
        }
        
        if self.allocated_page_count == 0 {
            return Err(AllocError::NoMemory);
        }
        
        self.allocated_page_count -= 1;
        Ok(self.page_alloc_pos)
    }

    fn dealloc_pages(&mut self, _pos: usize, num_pages: usize) {
        self.allocated_page_count += 1;
        if self.allocated_page_count == 0 {
            self.page_alloc_pos += num_pages * PAGE_SIZE;
        }
    }

    fn total_pages(&self) -> usize {
        (self.page_alloc_pos - self.byte_alloc_pos) / PAGE_SIZE
    }

    fn used_pages(&self) -> usize {
        (self.page_alloc_pos - self.byte_alloc_pos) / PAGE_SIZE
    }

    fn available_pages(&self) -> usize {
        (self.page_alloc_pos - self.byte_alloc_pos) / PAGE_SIZE
    }
}