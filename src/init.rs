use crate::gdt;
use crate::interrupts;

pub fn start_cpu0(idle_thread_proc: impl Fn() -> !) -> ! {
    gdt::init();
    interrupts::init_idt();

    crate::percpu::whats_going_on();

    idle_thread_proc();
}

pub fn idle_proc() -> ! {
    loop {}
}