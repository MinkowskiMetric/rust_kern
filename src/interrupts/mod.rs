pub mod exceptions;
mod interrupt_macros;
pub mod ipi;
pub mod irq;

pub use interrupt_macros::{InterruptErrorStack, InterruptStack};

/// Clear interrupts
#[inline(always)]
pub unsafe fn disable() {
    asm!("cli", options(nomem, nostack));
}

/// Set interrupts
#[inline(always)]
pub unsafe fn enable() {
    asm!("sti", options(nomem, nostack));
}

/// Set interrupts and halt
/// This will atomically wait for the next interrupt
/// Performing enable followed by halt is not guaranteed to be atomic, use this instead!
#[inline(always)]
pub unsafe fn enable_and_halt() {
    asm!("sti; hlt", options(nomem, nostack));
}

/// Set interrupts and nop
/// This will enable interrupts and allow the IF flag to be processed
/// Simply enabling interrupts does not gurantee that they will trigger, use this instead!
#[inline(always)]
pub unsafe fn enable_and_nop() {
    asm!("sti; nop", options(nomem, nostack));
}

/// Halt instruction
#[inline(always)]
pub unsafe fn halt() {
    asm!("hlt", options(nomem, nostack));
}

/// Pause instruction
/// Safe because it is similar to a NOP, and has no memory effects
#[inline(always)]
pub fn pause() {
    unsafe {
        asm!("pause", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn disable_and_halt() -> ! {
    unsafe {
        asm!("cli; hlt", options(noreturn));
    }
}