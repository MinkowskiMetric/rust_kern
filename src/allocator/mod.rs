use crate::init_mutex::InitMutex;
use core::alloc::{GlobalAlloc, Layout};
use simple_allocator::SimpleAllocator;

mod free_list;
mod simple_allocator;

static ALLOCATOR_IMPL: InitMutex<SimpleAllocator> = InitMutex::new();

pub(self) fn align_down(addr: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        addr & !(align - 1)
    } else if align == 0 {
        addr
    } else {
        panic!("`align` must be a power of 2");
    }
}

/// Align upwards. Returns the smallest x with alignment `align`
/// so that x >= addr. The alignment must be a power of 2.
pub(self) fn align_up(addr: usize, align: usize) -> usize {
    align_down(addr + align - 1, align)
}

pub unsafe fn init() {
    ALLOCATOR_IMPL.init(SimpleAllocator::new());
}

pub struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATOR_IMPL.lock().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATOR_IMPL.lock().dealloc(ptr, layout);
    }
}

pub fn allocated_space() -> usize {
    ALLOCATOR_IMPL.lock().allocated_space()
}

pub fn free_space() -> usize {
    ALLOCATOR_IMPL.lock().free_space()
}
