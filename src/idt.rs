use x86::dtables::{self, DescriptorTablePointer};
use x86::segmentation::Descriptor as X86IdtEntry;

pub static mut INIT_IDTR: DescriptorTablePointer<X86IdtEntry> = DescriptorTablePointer {
    limit: 0,
    base: 0 as *const X86IdtEntry,
};

#[thread_local]
pub static mut IDTR: DescriptorTablePointer<X86IdtEntry> = DescriptorTablePointer {
    limit: 0,
    base: 0 as *const X86IdtEntry,
};

pub unsafe fn init() {
    dtables::lidt(&INIT_IDTR);
}
