use crate::interrupt;

interrupt!(tlb, || {
    x86::tlb::flush_all();
});

interrupt!(halt, || {
    crate::interrupts::disable_and_halt()
});