use core::mem;
use x86::dtables::{self, DescriptorTablePointer};
use x86::segmentation::load_cs;
use x86::segmentation::{self, Descriptor as SegmentDescriptor, SegmentSelector};
use x86::Ring;

#[derive(Copy, Clone, Debug)]
#[repr(packed)]
pub struct GdtEntry {
    pub limitl: u16,
    pub offsetl: u16,
    pub offsetm: u8,
    pub access: u8,
    pub flags_limith: u8,
    pub offseth: u8,
}

impl GdtEntry {
    pub const fn new(offset: u32, limit: u32, access: u8, flags: u8) -> Self {
        GdtEntry {
            limitl: limit as u16,
            offsetl: offset as u16,
            offsetm: (offset >> 16) as u8,
            access,
            flags_limith: flags & 0xF0 | ((limit >> 16) as u8) & 0x0F,
            offseth: (offset >> 24) as u8,
        }
    }

    pub fn set_offset(&mut self, offset: u32) {
        self.offsetl = offset as u16;
        self.offsetm = (offset >> 16) as u8;
        self.offseth = (offset >> 24) as u8;
    }

    pub fn set_limit(&mut self, limit: u32) {
        self.limitl = limit as u16;
        self.flags_limith = self.flags_limith & 0xF0 | ((limit >> 16) as u8) & 0x0F;
    }
}

pub const GDT_NULL: usize = 0;
pub const GDT_KERNEL_CODE: usize = 1;
pub const GDT_KERNEL_DATA: usize = 2;
pub const GDT_KERNEL_TLS: usize = 3;
pub const GDT_USER_CODE: usize = 4;
pub const GDT_USER_DATA: usize = 5;
pub const GDT_USER_TLS: usize = 6;
pub const GDT_TSS: usize = 7;
pub const GDT_TSS_HIGH: usize = 8;

pub const GDT_A_PRESENT: u8 = 1 << 7;
pub const GDT_A_RING_0: u8 = 0 << 5;
pub const GDT_A_RING_1: u8 = 1 << 5;
pub const GDT_A_RING_2: u8 = 2 << 5;
pub const GDT_A_RING_3: u8 = 3 << 5;
pub const GDT_A_SYSTEM: u8 = 1 << 4;
pub const GDT_A_EXECUTABLE: u8 = 1 << 3;
pub const GDT_A_CONFORMING: u8 = 1 << 2;
pub const GDT_A_PRIVILEGE: u8 = 1 << 1;
pub const GDT_A_DIRTY: u8 = 1;

pub const GDT_A_TSS_AVAIL: u8 = 0x9;
pub const GDT_A_TSS_BUSY: u8 = 0xB;

pub const GDT_F_PAGE_SIZE: u8 = 1 << 7;
pub const GDT_F_PROTECTED_MODE: u8 = 1 << 6;
pub const GDT_F_LONG_MODE: u8 = 1 << 5;

static mut INIT_GDTR: DescriptorTablePointer<SegmentDescriptor> = DescriptorTablePointer {
    limit: 0,
    base: 0 as *const SegmentDescriptor,
};

static mut INIT_GDT: [GdtEntry; 4] = [
    // Null
    GdtEntry::new(0, 0, 0, 0),
    // Kernel code
    GdtEntry::new(
        0,
        0,
        GDT_A_PRESENT | GDT_A_RING_0 | GDT_A_SYSTEM | GDT_A_EXECUTABLE | GDT_A_PRIVILEGE,
        GDT_F_LONG_MODE,
    ),
    // Kernel data
    GdtEntry::new(
        0,
        0,
        GDT_A_PRESENT | GDT_A_RING_0 | GDT_A_SYSTEM | GDT_A_PRIVILEGE,
        GDT_F_LONG_MODE,
    ),
    // Kernel TLS
    GdtEntry::new(
        0,
        0,
        GDT_A_PRESENT | GDT_A_RING_3 | GDT_A_SYSTEM | GDT_A_PRIVILEGE,
        GDT_F_LONG_MODE,
    ),
];

// Initialize GDT
pub unsafe fn init() {
    // Setup the initial GDT with TLS, so we can setup the TLS GDT (a little confusing)
    // This means that each CPU will have its own GDT, but we only need to define it once as a thread local
    INIT_GDTR.limit = (INIT_GDT.len() * mem::size_of::<GdtEntry>() - 1) as u16;
    INIT_GDTR.base = INIT_GDT.as_ptr() as *const SegmentDescriptor;

    // Load the initial GDT, before we have access to thread locals
    dtables::lgdt(&INIT_GDTR);

    // Load the segment descriptors
    load_cs(SegmentSelector::new(GDT_KERNEL_CODE as u16, Ring::Ring0));
    segmentation::load_ds(SegmentSelector::new(GDT_KERNEL_DATA as u16, Ring::Ring0));
    segmentation::load_es(SegmentSelector::new(GDT_KERNEL_DATA as u16, Ring::Ring0));
    segmentation::load_fs(SegmentSelector::new(GDT_KERNEL_DATA as u16, Ring::Ring0));
    segmentation::load_gs(SegmentSelector::new(GDT_KERNEL_DATA as u16, Ring::Ring0));
    segmentation::load_ss(SegmentSelector::new(GDT_KERNEL_DATA as u16, Ring::Ring0));
}

#[thread_local]
static HMM: u8 = 42;

pub unsafe fn init_post_paging(tcb_offset: usize) {
    extern "C" {
        static __tdata_start: u8;
    }

    use x86::msr::{wrmsr, IA32_FS_BASE};
    wrmsr(IA32_FS_BASE, tcb_offset as u64);

    use crate::println;
    println!(
        "tcb_offset: {:#x} &HMM: {:#x} HMM: {}",
        tcb_offset, &HMM as *const _ as usize, HMM
    );
    println!(
        "tdata_start: {:#x} {}",
        &__tdata_start as *const _ as usize, __tdata_start
    );
}
