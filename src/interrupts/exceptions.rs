use crate::{interrupt_error, interrupt_stack};

interrupt_stack!(divide_by_zero, |stack| {
    panic!("Divide by zero: {:x?}", stack);
});

interrupt_stack!(debug, |stack| {
    panic!("Debug exception: {:x?}", stack);
});

interrupt_stack!(non_maskable, |stack| {
    panic!("Non maskable exception: {:x?}", stack);
});

interrupt_stack!(breakpoint, |stack| {
    panic!("Breakpoint exception: {:x?}", stack);
});

interrupt_stack!(overflow, |stack| {
    panic!("Overflow exception: {:x?}", stack);
});

interrupt_stack!(bound_range, |stack| {
    panic!("Bound range exception: {:x?}", stack);
});

interrupt_stack!(invalid_opcode, |stack| {
    panic!("Invalid opcode exception: {:x?}", stack);
});

interrupt_stack!(device_not_available, |stack| {
    panic!("Device not available exception: {:x?}", stack);
});

interrupt_error!(double_fault, |stack| {
    panic!("Double fault exception: {:x?}", stack);
});

interrupt_error!(invalid_tss, |stack| {
    panic!("Invalid TSS exception: {:x?}", stack);
});

interrupt_error!(segment_not_present, |stack| {
    panic!("Segment not present exception: {:x?}", stack);
});

interrupt_error!(stack_segment, |stack| {
    panic!("Stack segment exception: {:x?}", stack);
});

interrupt_error!(protection, |stack| {
    panic!("Protection exception: {:x?}", stack);
});

interrupt_error!(page, |stack| {
    let cr2: usize;
    asm!("mov {}, cr2", out(reg) cr2);

    panic!("Page fault: cr2: {:#x} {:x?}", cr2, stack);
});

interrupt_stack!(fpu_fault, |stack| {
    panic!("FPU exception: {:x?}", stack);
});

interrupt_error!(alignment_check, |stack| {
    panic!("Alignment check exception: {:x?}", stack);
});

interrupt_stack!(machine_check, |stack| {
    panic!("Machine check exception: {:x?}", stack);
});

interrupt_stack!(simd, |stack| {
    panic!("SIMD exception: {:x?}", stack);
});

interrupt_stack!(virtualization, |stack| {
    panic!("Virtualization exception: {:x?}", stack);
});

interrupt_error!(security, |stack| {
    panic!("Security exception: {:x?}", stack);
});
