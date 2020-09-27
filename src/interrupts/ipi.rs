use crate::interrupt;

interrupt!(tlb, || {
    crate::devices::local_apic::local_apic_access().eoi();
    x86::tlb::flush_all();
});

interrupt!(halt, || {
    crate::devices::local_apic::local_apic_access().eoi();
    crate::interrupts::disable_and_halt()
});

interrupt!(ipi_timer, || {
    crate::devices::local_apic::local_apic_access().eoi();
    crate::println!("AP timer");
});
