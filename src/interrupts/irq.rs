use crate::ipi::{ipi, IpiKind, IpiTarget};
use crate::{interrupt, interrupt_stack};

interrupt_stack!(timer, |_stack| {
    crate::devices::local_apic::local_apic_access().eoi();

    crate::println!("TIMER INTERRUPT");
    ipi(IpiKind::Timer, IpiTarget::Other);
});

interrupt!(spurious, || {
    panic!("Spurious interrupt");
});
