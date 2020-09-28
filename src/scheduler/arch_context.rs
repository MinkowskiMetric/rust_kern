use alloc::boxed::Box;

#[derive(Debug)]
#[repr(C)]
pub struct ArchContext {
    cr3: usize,
    rflags: usize,
    rbx: usize,
    r12: usize,
    r13: usize,
    r14: usize,
    r15: usize,
    rsp: usize,
    rbp: usize,
}

impl ArchContext {
    pub const fn new() -> Self {
        Self {
            cr3: 0,
            rflags: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: 0,
            rbp: 0,
        }
    }

    pub fn set_page_table(&mut self, cr3: usize) {
        self.cr3 = cr3;
    }

    pub fn page_table(&self) -> usize {
        self.cr3
    }

    pub fn set_stack(&mut self, rsp: usize) {
        self.rsp = rsp;
    }

    pub fn stack(&self) -> usize {
        self.rsp
    }

    pub unsafe fn push_stack(&mut self, value: usize) {
        self.rsp -= core::mem::size_of::<usize>();
        *(self.rsp as *mut usize) = value;
    }

    pub unsafe fn pop_stack(&mut self) -> usize {
        let value = *(self.rsp as *const usize);
        self.rsp += core::mem::size_of::<usize>();
        value
    }

    pub unsafe fn push_system_task_startup<F: FnOnce() -> !>(&mut self, func: F) {
        let trampoline: Box<dyn TrampolineLaunch> = box LaunchTrampolineStruct::<F> { func };
        let trampoline = box trampoline;

        self.push_stack(Box::into_raw(trampoline) as usize);
        self.push_stack(launch_trampoline as usize);
    }

    pub unsafe fn switch_to(&mut self, next: &mut ArchContext) {
        do_switch(self, next);
    }
}

crate::function!(do_switch(current: &mut ArchContext, next: &mut ArchContext) => {
    "
        // current context is rdi, new context is rsi.
        mov rax, cr3
        mov [rdi+0*8], rax

        mov rcx, [rsi+0*8]
        cmp rcx, rax
        je no_cr3_change
        mov cr3, rcx

    no_cr3_change:
        pushfq
        pop rax
        mov [rdi+1*8], rax

        mov rax, [rsi+1*8]
        push rax
        popfq

        mov [rdi+2*8], rbx
        mov rbx, [rsi+2*8]
        mov [rdi+3*8], r12
        mov r12, [rsi+3*8]
        mov [rdi+4*8], r13
        mov r13, [rsi+4*8]
        mov [rdi+5*8], r14
        mov r14, [rsi+5*8]
        mov [rdi+6*8], r15
        mov r15, [rsi+6*8]

        mov rax, rsp
        mov [rdi+7*8], rax
        mov rcx, [rsi+7*8]
        mov rsp, rcx

        mov [rdi+8*8], rbp
        mov rbp, [rsi+8*8]

        // At this point the context switch is complete, but we need to tell the scheduler to complete its job
        call complete_task_switch

        // And finally, return
        ret
    ",
});

struct LaunchTrampolineStruct<F: FnOnce() -> !> {
    func: F,
}

trait TrampolineLaunch {
    fn do_call(self: Box<Self>) -> !;
}

impl<F: FnOnce() -> !> TrampolineLaunch for LaunchTrampolineStruct<F> {
    fn do_call(self: Box<Self>) -> ! {
        (self.func)()
    }
}

#[no_mangle]
unsafe extern "C" fn do_task_trampoline_launch(trampoline: *mut Box<dyn TrampolineLaunch>) {
    let trampoline = *Box::from_raw(trampoline);
    trampoline.do_call();
}

crate::function!(launch_trampoline() => {
    "
        // We have a special calling convention here - the scheduler actually returns to this function.
        // Next on the stack is the trampoline struct
        pop rdi
        jmp do_task_trampoline_launch
    ",
});
