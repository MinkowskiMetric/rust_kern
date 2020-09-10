use crate::physmem::{allocate_frame, Frame};
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use spin::Mutex;

pub const DEFAULT_KERNEL_STACK_PAGES: usize = 8;

struct StackManager {}

impl StackManager {
    pub fn allocate_kernel_stack(&mut self, pages: usize) -> Option<KernelStack> {
        assert!(pages > 1, "Kernel stack allocation includes guard page");

        // Allocate the pages for the stack.
        let pages: Vec<_> = (1..pages).map(|_| allocate_frame()).collect();

        use crate::println;
        println!("Stack pages {:#?}", pages);

        todo!()
    }

    pub fn release_kernel_stack(&mut self, start_va: u64, limit_va: u64) {
        todo!()
    }
}

static STACK_MANAGER: Mutex<MaybeUninit<StackManager>> = Mutex::new(MaybeUninit::uninit());

#[repr(transparent)]
struct StackManagerLock<'a> {
    guard: spin::MutexGuard<'a, MaybeUninit<StackManager>>,
}

impl<'a> Deref for StackManagerLock<'a> {
    type Target = StackManager;
    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.guard.as_ptr() as *const StackManager) }
    }
}

impl<'a> DerefMut for StackManagerLock<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.guard.as_ptr() as *mut StackManager) }
    }
}

fn lock_stack_manager<'a>() -> StackManagerLock<'a> {
    StackManagerLock {
        guard: STACK_MANAGER.lock(),
    }
}

pub struct KernelStack {
    start_va: u64,
    limit_va: u64,
}

impl KernelStack {
    pub fn switch_to_permanent(self, f: impl FnOnce(KernelStack) -> !) -> ! {
        todo!()
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        lock_stack_manager().release_kernel_stack(self.start_va, self.limit_va);
    }
}

pub unsafe fn init() {
    STACK_MANAGER.lock().as_mut_ptr().write(StackManager {});
}

pub fn allocate_kernel_stack(pages: usize) -> Option<KernelStack> {
    lock_stack_manager().allocate_kernel_stack(pages)
}
