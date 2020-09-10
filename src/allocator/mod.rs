use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::null_mut;
use simple_allocator::SimpleAllocator;
use spin::{Mutex, MutexGuard};

mod simple_allocator;

static ALLOCATOR_IMPL: Mutex<MaybeUninit<SimpleAllocator>> = Mutex::new(MaybeUninit::uninit());

struct AllocatorLock<'a> {
    guard: MutexGuard<'a, MaybeUninit<SimpleAllocator>>,
}

impl<'a> Deref for AllocatorLock<'a> {
    type Target = SimpleAllocator;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.guard.as_ptr() as *const SimpleAllocator) }
    }
}

impl<'a> DerefMut for AllocatorLock<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.guard.as_ptr() as *mut SimpleAllocator) }
    }
}

fn lock_allocator_impl<'a>() -> AllocatorLock<'a> {
    AllocatorLock {
        guard: ALLOCATOR_IMPL.lock(),
    }
}

pub unsafe fn init() {
    ALLOCATOR_IMPL
        .lock()
        .as_mut_ptr()
        .write(SimpleAllocator::new());
}

pub struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        lock_allocator_impl().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        lock_allocator_impl().dealloc(ptr, layout);
    }
}
