use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

pub struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("Allocator not implemented");
    }
}
