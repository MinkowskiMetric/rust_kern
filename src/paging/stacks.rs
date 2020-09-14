use super::{lock_page_table, Mapper, Result};
use crate::physmem::{allocate_frame, Frame};
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use spin::Mutex;

pub const DEFAULT_KERNEL_STACK_PAGES: usize = 8;

struct StackManager {
    base_va: usize,
    limit_va: usize,
}

impl StackManager {
    pub fn new(base_va: usize, limit_va: usize) -> Result<Self> {
        let mut ret = Self { base_va, limit_va };

        ret.release_kernel_stack(base_va, limit_va)?;

        Ok(ret)
    }

    pub fn allocate_kernel_stack(&mut self, pages: usize) -> Result<KernelStack> {
        assert!(pages > 1, "Kernel stack allocation includes guard page");

        unsafe { lock_page_table() }.and_then(|page_table| {
            // Locate a VM space for the stack
            let stack_start_va = self.find_stack_location(&page_table, pages)?;

            // Allocate the pages for the stack.
            let mut pages: Vec<_> = (1..pages).map(|_| allocate_frame()).collect();

            todo!()
        })
    }

    pub fn release_kernel_stack(&mut self, start_va: usize, limit_va: usize) -> Result<()> {
        todo!()
    }

    fn find_stack_location(&self, page_table: &Mapper, pages: usize) -> Result<usize> {
        todo!()
    }
}

static STACK_MANAGER: Mutex<Option<&'static mut StackManager>> = Mutex::new(None);

#[repr(transparent)]
struct StackManagerLock<'a> {
    guard: spin::MutexGuard<'a, Option<&'static mut StackManager>>,
}

impl<'a> Deref for StackManagerLock<'a> {
    type Target = StackManager;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("Kernel stacks not initialized")
    }
}

impl<'a> DerefMut for StackManagerLock<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().expect("Kernel stacks not initialized")
    }
}

fn lock_stack_manager<'a>() -> StackManagerLock<'a> {
    StackManagerLock {
        guard: STACK_MANAGER.lock(),
    }
}

pub struct KernelStack {
    start_va: usize,
    limit_va: usize,
}

impl KernelStack {
    pub fn switch_to_permanent(self, f: impl FnOnce(KernelStack) -> !) -> ! {
        todo!()
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        lock_stack_manager()
            .release_kernel_stack(self.start_va, self.limit_va)
            .expect("Failed to release kernel stack");
    }
}

pub fn init(base_va: usize, limit_va: usize) -> Result<()> {
    use core::mem::MaybeUninit;
    use core::sync::atomic::{AtomicBool, Ordering};

    static PASSED: AtomicBool = AtomicBool::new(false);

    if PASSED.swap(true, Ordering::AcqRel) {
        panic!("Kernel stacks already initialized");
    }

    static mut KERNEL_STACK_BUFFER: MaybeUninit<StackManager> = MaybeUninit::uninit();
    unsafe {
        KERNEL_STACK_BUFFER
            .as_mut_ptr()
            .write(StackManager::new(base_va, limit_va)?)
    };

    *STACK_MANAGER.lock() = Some(unsafe { core::mem::transmute(&mut KERNEL_STACK_BUFFER) });

    Ok(())
}

pub fn allocate_kernel_stack(pages: usize) -> Result<KernelStack> {
    lock_stack_manager().allocate_kernel_stack(pages)
}
