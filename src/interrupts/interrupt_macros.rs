#[derive(Default, Debug, Copy, Clone)]
#[repr(packed)]
pub struct ScratchRegisters {
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rax: usize,
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(packed)]
pub struct PreservedRegisters {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbp: usize,
    pub rbx: usize,
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(packed)]
pub struct IretRegisters {
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,

    // ----
    // The following will only be present if interrupt is raised from another
    // privilege ring. Otherwise, they are undefined values.
    // ----
    pub rsp: usize,
    pub ss: usize,
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(packed)]
pub struct InterruptStack {
    pub fs: usize,
    pub preserved: PreservedRegisters,
    pub scratch: ScratchRegisters,
    pub iret: IretRegisters,
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(packed)]
pub struct InterruptErrorStack {
    pub code: usize,
    pub inner: InterruptStack,
}

#[macro_export]
macro_rules! intel_asm {
    ($($strings:expr,)+) => {
        global_asm!(concat!(
            ".intel_syntax noprefix\n",
            $($strings),+,
            ".att_syntax prefix\n",
        ));
    };
}

#[macro_export]
macro_rules! function {
    ($name:ident => { $($body:expr,)+ }) => {
        $crate::intel_asm!(
            ".global ", stringify!($name), "\n",
            ".type ", stringify!($name), ", @function\n",
            ".section .text.", stringify!($name), ", \"ax\", @progbits\n",
            stringify!($name), ":\n",
            $($body),+,
            ".size ", stringify!($name), ", . - ", stringify!($name), "\n",
            ".text\n",
        );
        extern "C" {
            pub fn $name();
        }
    };
}

#[macro_export]
macro_rules! push_scratch {
    () => {
        "
        // Push scratch registers
        push rcx
        push rdx
        push rdi
        push rsi
        push r8
        push r9
        push r10
        push r11
    "
    };
}
#[macro_export]
macro_rules! pop_scratch {
    () => {
        "
        // Pop scratch registers
        pop r11
        pop r10
        pop r9
        pop r8
        pop rsi
        pop rdi
        pop rdx
        pop rcx
        pop rax
    "
    };
}

#[macro_export]
macro_rules! push_preserved {
    () => {
        "
        // Push preserved registers
        push rbx
        push rbp
        push r12
        push r13
        push r14
        push r15
    "
    };
}
#[macro_export]
macro_rules! pop_preserved {
    () => {
        "
        // Pop preserved registers
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbp
        pop rbx
    "
    };
}

#[macro_export]
macro_rules! push_fs {
    () => {
        "
        // Push fs
        push fs

        // Load kernel tls
        //
        // NOTE: We can't load the value directly into `fs`. So we need to use a
        // scratch register (as preserved registers aren't backed up by the
        // interrupt! macro) to store it. We also can't use `rax` as the temporary
        // value, as during errors that's already used for the error code.
        mov rcx, 0x18
        mov fs, cx
    "
    };
}
#[macro_export]
macro_rules! pop_fs {
    () => {
        "
        // Pop fs
        pop fs
    "
    };
}

#[macro_export]
macro_rules! interrupt_stack {
    ($name:ident, |$stack:ident| $code:block) => {
        paste::item! {
            #[no_mangle]
            unsafe extern "C" fn [<__interrupt_ $name>](stack: *mut $crate::interrupts::InterruptStack) {
                // This inner function is needed because macros are buggy:
                // https://github.com/dtolnay/paste/issues/7
                #[inline(always)]
                unsafe fn inner($stack: &mut $crate::interrupts::InterruptStack) {
                    $code
                }
                inner(&mut *stack);
            }

            $crate::function!($name => {
                // Backup all userspace registers to stack
                "push rax\n",
                $crate::push_scratch!(),
                $crate::push_preserved!(),
                $crate::push_fs!(),

                // TODO: Map PTI
                // $crate::arch::x86_64::pti::map();

                // Call inner function with pointer to stack
                "mov rdi, rsp\n",
                "call __interrupt_", stringify!($name), "\n",

                // TODO: Unmap PTI
                // $crate::arch::x86_64::pti::unmap();

                // Restore all userspace registers
                $crate::pop_fs!(),
                $crate::pop_preserved!(),
                $crate::pop_scratch!(),

                "iretq\n",
            });
        }
    };
}

#[macro_export]
macro_rules! interrupt {
    ($name:ident, || $code:block) => {
        paste::item! {
            #[no_mangle]
            unsafe extern "C" fn [<__interrupt_ $name>]() {
                $code
            }

            $crate::function!($name => {
                // Backup all userspace registers to stack
                "push rax\n",
                push_scratch!(),
                push_fs!(),

                // TODO: Map PTI
                // $crate::arch::x86_64::pti::map();

                // Call inner function with pointer to stack
                "call __interrupt_", stringify!($name), "\n",

                // TODO: Unmap PTI
                // $crate::arch::x86_64::pti::unmap();

                // Restore all userspace registers
                pop_fs!(),
                pop_scratch!(),

                "iretq\n",
            });
        }
    };
}

#[macro_export]
macro_rules! interrupt_error {
    ($name:ident, |$stack:ident| $code:block) => {
        paste::item! {
            #[no_mangle]
            unsafe extern "C" fn [<__interrupt_ $name>](stack: *mut $crate::interrupts::InterruptErrorStack) {
                // This inner function is needed because macros are buggy:
                // https://github.com/dtolnay/paste/issues/7
                #[inline(always)]
                unsafe fn inner($stack: &mut $crate::interrupts::InterruptErrorStack) {
                    $code
                }
                inner(&mut *stack);
            }

            $crate::function!($name => {
                // Move rax into code's place, put code in last instead (to be
                // compatible with InterruptStack)
                "xchg [rsp], rax\n",

                // Push all userspace registers
                $crate::push_scratch!(),
                $crate::push_preserved!(),
                $crate::push_fs!(),

                // Put code in, it's now in rax
                "push rax\n",

                // TODO: Map PTI
                // $crate::arch::x86_64::pti::map();

                // Call inner function with pointer to stack
                "mov rdi, rsp\n",
                "call __interrupt_", stringify!($name), "\n",

                // TODO: Unmap PTI
                // $crate::arch::x86_64::pti::unmap();

                // Pop code
                "add rsp, 8\n",

                // Restore all userspace registers
                $crate::pop_fs!(),
                $crate::pop_preserved!(),
                $crate::pop_scratch!(),

                "iretq\n",
            });
        }
    };
}
