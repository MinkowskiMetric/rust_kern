use crate::{interrupt, interrupt_stack};

interrupt_stack!(timer, |_stack| {
    panic!("TIMER INTERRUPT");
});

interrupt!(spurious, || {
    panic!("Spurious interrupt");
});
