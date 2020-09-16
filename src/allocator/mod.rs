use crate::init_mutex::InitMutex;
use core::alloc::{GlobalAlloc, Layout};
use simple_allocator::SimpleAllocator;

mod simple_allocator;

static ALLOCATOR_IMPL: InitMutex<SimpleAllocator> = InitMutex::new();

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
