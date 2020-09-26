use crate::interrupt_stack;

interrupt_stack!(timer, |_stack| {
    panic!("TIMER INTERRUPT");
});
