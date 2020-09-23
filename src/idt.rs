use crate::interrupts::exceptions;
use bitflags::bitflags;
use x86::dtables::{self, DescriptorTablePointer};
use x86::segmentation::Descriptor as X86IdtEntry;

bitflags! {
    struct IdtFlags: u8 {
        const PRESENT = 1 << 7;
        const RING_0 = 0 << 5;
        const RING_1 = 1 << 5;
        const RING_2 = 2 << 5;
        const RING_3 = 3 << 5;
        const SS = 1 << 4;
        const INTERRUPT = 0xE;
        const TRAP = 0xF;
    }
}

#[derive(Copy, Clone, Debug, Default)]
#[repr(packed)]
struct IdtEntry {
    offsetl: u16,
    selector: u16,
    ist: u8,
    attribute: u8,
    offsetm: u16,
    offseth: u32,
    zero2: u32,
}

impl IdtEntry {
    pub const fn new() -> IdtEntry {
        IdtEntry {
            offsetl: 0,
            selector: 0,
            ist: 0,
            attribute: 0,
            offsetm: 0,
            offseth: 0,
            zero2: 0,
        }
    }

    pub fn set_flags(&mut self, flags: IdtFlags) {
        self.attribute = flags.bits;
    }

    pub fn set_offset(&mut self, selector: u16, base: usize) {
        self.selector = selector;
        self.offsetl = base as u16;
        self.offsetm = (base >> 16) as u16;
        self.offseth = (base >> 32) as u32;
    }

    // A function to set the offset more easily
    pub fn set_func(&mut self, func: unsafe extern "C" fn()) {
        self.set_flags(IdtFlags::PRESENT | IdtFlags::RING_0 | IdtFlags::INTERRUPT);
        self.set_offset(8, func as usize);
    }

    pub fn set_ist(&mut self, ist: u8) {
        assert!(ist < 8, "Invalid IST");
        self.ist = ist + 1;
    }
}

type IdtEntries = [IdtEntry; 256];

#[repr(packed)]
struct Idt {
    entries: IdtEntries,
}

impl Idt {
    pub const fn new() -> Self {
        Self {
            entries: [IdtEntry::new(); 256],
        }
    }
}

static mut INIT_IDTR: DescriptorTablePointer<X86IdtEntry> = DescriptorTablePointer {
    limit: 0,
    base: 0 as *const X86IdtEntry,
};

pub unsafe fn early_init() {
    dtables::lidt(&INIT_IDTR);
}

pub fn init(_is_bsp: bool) {
    let (idt, idtr) = unsafe {
        use core::sync::atomic::{AtomicBool, Ordering};

        #[thread_local]
        static CHECK: AtomicBool = AtomicBool::new(false);
        assert_eq!(
            CHECK.swap(true, Ordering::SeqCst),
            false,
            "IDT for this CPU is already initialized"
        );

        #[thread_local]
        static mut IDT: Idt = Idt::new();

        #[thread_local]
        static mut IDTR: DescriptorTablePointer<X86IdtEntry> = DescriptorTablePointer {
            limit: 0,
            base: 0 as *const X86IdtEntry,
        };

        (&mut IDT, &mut IDTR)
    };

    idtr.limit = (idt.entries.len() * core::mem::size_of::<IdtEntry>() - 1) as u16;
    idtr.base = idt.entries.as_ptr() as *const X86IdtEntry;

    idt.entries[0].set_func(exceptions::divide_by_zero);
    idt.entries[1].set_func(exceptions::debug);
    idt.entries[2].set_func(exceptions::non_maskable);
    idt.entries[2].set_ist(0);
    idt.entries[3].set_func(exceptions::breakpoint);
    idt.entries[3].set_flags(IdtFlags::PRESENT | IdtFlags::RING_3 | IdtFlags::INTERRUPT);
    idt.entries[4].set_func(exceptions::overflow);
    idt.entries[5].set_func(exceptions::bound_range);
    idt.entries[6].set_func(exceptions::invalid_opcode);
    idt.entries[7].set_func(exceptions::device_not_available);
    idt.entries[8].set_func(exceptions::double_fault);
    idt.entries[8].set_ist(0);
    // 9 no longer available
    idt.entries[10].set_func(exceptions::invalid_tss);
    idt.entries[11].set_func(exceptions::segment_not_present);
    idt.entries[12].set_func(exceptions::stack_segment);
    idt.entries[13].set_func(exceptions::protection);
    idt.entries[14].set_func(exceptions::page);
    idt.entries[14].set_ist(0);
    // 15 reserved
    idt.entries[16].set_func(exceptions::fpu_fault);
    idt.entries[17].set_func(exceptions::alignment_check);
    idt.entries[18].set_func(exceptions::machine_check);
    idt.entries[19].set_func(exceptions::simd);
    idt.entries[20].set_func(exceptions::virtualization);
    // 21 through 29 reserved
    idt.entries[30].set_func(exceptions::security);
    // 31 reserved

    unsafe {
        dtables::lidt(idtr);
    }
}
