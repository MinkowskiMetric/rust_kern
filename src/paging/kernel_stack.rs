use super::Region;
use alloc::boxed::Box;

#[derive(Debug)]
pub struct KernelStack {
    region: Region,
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
    pub(super) fn new(region: Region) -> Self {
        Self { region }
    }

    pub fn stack_top(&self) -> usize {
        self.region.limit()
    }

    pub fn switch_to_permanent(self, function: impl FnOnce(KernelStack) -> ! + 'static) -> ! {
        let trampoline = box Trampoline {
            stack: self,
            function,
        };
        switch_to_trampoline(trampoline);
    }
}
