use super::page_entry::{self, PresentPageFlags};
use super::{lock_page_table, MapperFlushAll, MemoryError, Result, PAGE_SIZE};
use crate::init_mutex::InitMutex;
use crate::physmem::allocate_frame;
use alloc::boxed::Box;
use alloc::format;
use alloc::{vec, vec::Vec};
use core::fmt;

pub const DEFAULT_KERNEL_STACK_PAGES: usize = 8;

struct StackManagerVmRange {
    base_va: usize,
    limit_va: usize,
    available: bool,
}

impl StackManagerVmRange {
    pub fn size(&self) -> usize {
        self.limit_va - self.base_va
    }
}

impl fmt::Debug for StackManagerVmRange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("StackManagerVmRange");
        f.field("base_va", &format!("{:#x}", self.base_va));
        f.field("limit_va", &format!("{:#x}", self.limit_va));
        f.field("available", &self.available);
        f.finish()
    }
}

struct StackManager {
    ranges: Vec<StackManagerVmRange>,
}

impl StackManager {
    pub fn new(base_va: usize, limit_va: usize) -> Result<Self> {
        Ok(Self {
            ranges: vec![StackManagerVmRange {
                base_va,
                limit_va,
                available: true,
            }],
        })
    }

    pub fn allocate_kernel_stack(&mut self, pages: usize) -> Result<KernelStack> {
        assert!(pages > 1, "Kernel stack allocation includes guard page");
        assert!(pages < 64, "Kernel stack allocation too long");

        let required_size = pages
            .checked_mul(PAGE_SIZE as usize)
            .expect("Kernel stack too large");
        let (index, _) = self
            .ranges
            .iter()
            .enumerate()
            .find(|(_, range)| range.available && range.size() >= required_size)
            .ok_or(MemoryError::NoRegionAddressSpaceAvailable)?;

        if self.ranges[index].size() > required_size {
            self.ranges.insert(
                index + 1,
                StackManagerVmRange {
                    base_va: self.ranges[index].base_va + required_size,
                    limit_va: self.ranges[index].limit_va,
                    available: true,
                },
            );

            self.ranges[index].limit_va = self.ranges[index + 1].base_va;
        }

        let (start_va, limit_va) = {
            let range = &mut self.ranges[index];

            range.available = false;

            (range.base_va, range.limit_va)
        };

        // Now we need to map the pages. The lowest address page is the guard page.
        unsafe { lock_page_table() }
            .and_then(|mut page_table| {
                let mut flusher = MapperFlushAll::new();

                let result = try {
                    flusher.consume(page_table.set_not_present(
                        start_va as u64,
                        page_entry::KernelStackGuardPagePte::new(),
                    )?);

                    for page in 1..pages {
                        let page = start_va + (page * PAGE_SIZE as usize);
                        let frame = allocate_frame().ok_or(MemoryError::OutOfMemory)?;

                        flusher.consume(page_table.map_to(
                            page as u64,
                            frame,
                            PresentPageFlags::WRITABLE | PresentPageFlags::NO_EXECUTE,
                        )?);
                    }

                    KernelStack::new(start_va, limit_va)
                };

                flusher.flush(&page_table);

                result
            })
            .or_else(|err| {
                self.release_kernel_stack(start_va, limit_va)
                    .expect("Failed to release stack");
                Err(err)
            })
    }

    pub fn release_kernel_stack(&mut self, start_va: usize, limit_va: usize) -> Result<()> {
        // We're only interested in an exact match (we could do a binary search here since we know the list is ordered. Later)
        let (range_index, _) = self
            .ranges
            .iter()
            .enumerate()
            .find(|(_, range)| {
                range.base_va == start_va && range.limit_va == limit_va && !range.available
            })
            .ok_or(MemoryError::InvalidStack)?;

        use crate::println;
        println!("BEFORE: {} {:?}", range_index, self.ranges);

        // Unmap the pages before we modify the range table
        unsafe { lock_page_table() }
            .and_then(|mut page_table| {
                let mut flusher = MapperFlushAll::new();

                let result = try {
                    let mut pos = start_va;
                    while pos < limit_va {
                        flusher.consume(page_table.unmap_and_free(pos as u64)?);
                        pos += PAGE_SIZE as usize;
                    }
                };

                flusher.flush(&page_table);

                result
            })
            .expect("Failed to unmap pages"); // Don't propagate this error. If unmapping fails we're in trouble

        let range_index = if range_index > 0 && self.ranges[range_index - 1].available {
            // We can join this range up with the previous range
            self.ranges[range_index - 1].limit_va = limit_va;
            self.ranges.remove(range_index);
            range_index - 1
        } else {
            self.ranges[range_index].available = true;
            range_index
        };

        if range_index < self.ranges.len() - 1 && self.ranges[range_index + 1].available {
            self.ranges[range_index].limit_va = self.ranges[range_index + 1].limit_va;
            self.ranges.remove(range_index + 1);
        }

        println!("AFTER: {:?}", self.ranges);
        Ok(())
    }
}

static STACK_MANAGER: InitMutex<StackManager> = InitMutex::new();

pub struct KernelStack {
    start_va: usize,
    limit_va: usize,
}

trait TrampolineCallable {
    fn get_stack_top(&self) -> usize;
    fn call_on_stack(self: Box<Self>) -> !;
}

struct Trampoline<F: FnOnce(KernelStack) -> !> {
    stack: KernelStack,
    function: F,
}

impl<F: FnOnce(KernelStack) -> !> TrampolineCallable for Trampoline<F> {
    fn get_stack_top(&self) -> usize {
        self.stack.stack_top()
    }

    fn call_on_stack(self: Box<Self>) -> ! {
        // Take the value off the heap
        let local_trampoline = *self;

        (local_trampoline.function)(local_trampoline.stack);
    }
}

#[no_mangle]
extern "C" fn stack_switch_entry(trampoline: *mut Box<dyn TrampolineCallable>) {
    let trampoline = unsafe { *Box::from_raw(trampoline) };
    trampoline.call_on_stack();
}

fn switch_to_trampoline(trampoline: Box<dyn TrampolineCallable>) -> ! {
    // Get the new stack pointer
    let stack_pointer = trampoline.get_stack_top();

    // Take a raw pointer to the trampoline
    let trampoline = box trampoline;
    let trampoline = Box::into_raw(trampoline);

    unsafe {
        asm!(
            "mov rsp, {0}",
            "mov rdi, {1}",
            "jmp stack_switch_entry",
            in(reg) stack_pointer,
            in(reg) trampoline as usize,
            options(noreturn),
        )
    }
}

impl KernelStack {
    pub fn new(start_va: usize, limit_va: usize) -> Self {
        Self { start_va, limit_va }
    }

    pub fn stack_top(&self) -> usize {
        self.limit_va
    }

    pub fn switch_to_permanent(self, function: impl FnOnce(KernelStack) -> ! + 'static) -> ! {
        let trampoline = box Trampoline {
            stack: self,
            function,
        };
        switch_to_trampoline(trampoline);
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        STACK_MANAGER
            .lock()
            .release_kernel_stack(self.start_va, self.limit_va)
            .expect("Failed to release kernel stack");
    }
}

pub fn init(base_va: usize, limit_va: usize) -> Result<()> {
    STACK_MANAGER.init(StackManager::new(base_va, limit_va)?);
    Ok(())
}

pub fn allocate_kernel_stack(pages: usize) -> Result<KernelStack> {
    STACK_MANAGER.lock().allocate_kernel_stack(pages)
}
