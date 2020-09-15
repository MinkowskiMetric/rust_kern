use crate::physmem;
use bootloader::BootInfo;
use core::ops::{Deref, DerefMut};
use spin::{Mutex, MutexGuard};
use x86::{controlregs, tlb};

pub use crate::physmem::{page_align_down, page_align_up, Frame, PAGE_SIZE};

use table::{p1_index, p2_index, p3_index, p4_index};
pub use table::{
    HierarchyLevel, MappedPageTable, MappedPageTableMut, PageTable, PageTableIndex, PageTableLevel,
    L1, L2, L3, L4,
};

pub use heap_region::{allocate_region, Region, RegionFlags};
pub use hyperspace::{map_page, HyperspaceMapping};
pub use mapper::{MappedMutPteReference, MappedPteReference, Mapper, MapperFlush, MapperFlushAll};
pub use stacks::{allocate_kernel_stack, KernelStack, DEFAULT_KERNEL_STACK_PAGES};

mod heap_region;
mod hyperspace;
mod mapper;
mod page_entry;
mod stacks;
mod table;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    OutOfHyperspacePages,
    NotMapped,
    NoRegionAddressSpaceAvailable,
    OutOfMemory,
    InvalidStack,
}

pub type Result<T> = core::result::Result<T, MemoryError>;

pub const FIRST_KERNEL_PML4: PageTableIndex = p4_index(0xffff_8000_0000_0000);
pub const KERNEL_PML4: PageTableIndex = p4_index(0xffff_8000_0000_0000);
pub const KERNEL_DATA_PML4: PageTableIndex = p4_index(HYPERSPACE_BASE);

// Allow 1GB of kernel address space for hyperspace. We don't use all of it.
pub const HYPERSPACE_BASE: u64 = 0xffff_ff80_0000_0000;
pub const HYPERSPACE_LIMIT: u64 = 0xffff_ff80_4000_0000;

// Allow 1GB for stacks
pub const KERNEL_STACKS_BASE: usize = 0xffff_ff80_4000_0000;
pub const KERNEL_STACKS_LIMIT: usize = 0xffff_ff80_8000_0000;

// Allow 1GB for heap regions
pub const KERNEL_HEAP_BASE: u64 = 0xffff_ff80_8000_0000;
pub const KERNEL_HEAP_LIMIT: u64 = 0xffff_ff80_c000_0000;

pub struct ActivePageTable<'a> {
    #[allow(dead_code)]
    guard: MutexGuard<'a, ()>,
    mapper: Mapper,
}

impl<'a> ActivePageTable<'a> {
    pub fn flush(&self, addr: u64) {
        unsafe { tlb::flush(addr as usize) };
    }

    pub fn flush_all(&self) {
        unsafe {
            tlb::flush_all();
        }
    }
}

impl<'a> Deref for ActivePageTable<'a> {
    type Target = Mapper;
    fn deref(&self) -> &Self::Target {
        &self.mapper
    }
}

impl<'a> DerefMut for ActivePageTable<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.mapper
    }
}

pub unsafe fn lock_page_table() -> Result<ActivePageTable<'static>> {
    static PAGE_LOCK: Mutex<()> = Mutex::new(());

    let guard = PAGE_LOCK.lock();

    Mapper::new(Frame::containing_address(controlregs::cr3()))
        .map(|mapper| ActivePageTable { guard, mapper })
}

// The bootloader sets us an amazing challenge - it doesn't tell us where in physical memory
// it has loaded the kernel, or where the bootloader stack is.
// That info is sort of in the memory map, but we need to figure out what is going on.

// For the kernel, we can work it out because we know where the kernel is mapped,
// and we can use that to copy the page table entries.

unsafe fn copy_boot_mapping(
    boot_info: &BootInfo,
    boot_p4_table: &PageTable<L4>,
    init_p4_table: &mut PageTable<L4>,
    start: u64,
    end: u64,
    flags: page_entry::PresentPageFlags,
) {
    use table::BootPageTable;

    let mut virt_page = start;
    while virt_page < end {
        let init_p1_table = init_p4_table
            .boot_create_next_table(boot_info, p4_index(virt_page))
            .boot_create_next_table(boot_info, p3_index(virt_page))
            .boot_create_next_table(boot_info, p2_index(virt_page));

        let boot_p1_table = boot_p4_table
            .boot_next_table(boot_info, p4_index(virt_page))
            .unwrap()
            .boot_next_table(boot_info, p3_index(virt_page))
            .unwrap()
            .boot_next_table(boot_info, p2_index(virt_page))
            .unwrap();

        let boot_p1_entry = boot_p1_table[p1_index(virt_page)].present().expect("Expected present page in boot mapping");

        init_p1_table[p1_index(virt_page)] =
            page_entry::RawPresentPte::from_frame_and_flags(boot_p1_entry.frame(), flags).into();

        virt_page += PAGE_SIZE;
    }
}

pub unsafe fn init(boot_info: &BootInfo) {
    extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    };

    let kernel_start = page_align_down((&__kernel_start as *const u8) as u64);
    let kernel_end = page_align_up((&__kernel_end as *const u8) as u64);

    assert_eq!(
        p4_index(kernel_start),
        KERNEL_PML4,
        "Kernel is not in kernel PML4 region"
    );
    assert_eq!(
        p4_index(kernel_end),
        KERNEL_PML4,
        "Kernel is not in kernel PML4 region"
    );

    // Allow for the "guard page" on the stack
    let boot_stack_start = 0x10000000 + PAGE_SIZE;
    let boot_stack_end = boot_stack_start + (PAGE_SIZE * 8);

    let boot_info_start = 0x20000000;
    let boot_info_end = boot_info_start + PAGE_SIZE;

    // How do we get hold of the bootloader page table. Fortunately, the bootloader identity maps
    // enough physical memory that we can access it directly like this.
    let bootloader_page_table =
        &*((controlregs::cr3() + boot_info.physical_memory_offset) as *const PageTable<L4>);

    // Allocate a new page table
    let init_page_table_phys =
        physmem::allocate_frame().expect("cannot allocate early page directory");
    let init_page_table = &mut *((init_page_table_phys.physical_address()
        + boot_info.physical_memory_offset) as *mut PageTable<L4>);

    copy_boot_mapping(
        boot_info,
        bootloader_page_table,
        init_page_table,
        kernel_start,
        kernel_end,
        page_entry::PresentPageFlags::WRITABLE,
    );
    copy_boot_mapping(
        boot_info,
        bootloader_page_table,
        init_page_table,
        boot_stack_start,
        boot_stack_end,
        page_entry::PresentPageFlags::NO_EXECUTE | page_entry::PresentPageFlags::WRITABLE,
    );

    // We need to copy an additional mapping for VGA memory since the logger uses it
    // TODOTODOTODO - should fix this
    copy_boot_mapping(
        boot_info,
        bootloader_page_table,
        init_page_table,
        0xb8000,
        0xb9000,
        page_entry::PresentPageFlags::NO_EXECUTE | page_entry::PresentPageFlags::WRITABLE,
    );

    // Set up hyperspace before switching to the page table
    hyperspace::init(boot_info, init_page_table);

    // Switch to the page table
    controlregs::cr3_write(init_page_table_phys.physical_address());

    // Complete hyperspace setup
    hyperspace::init_post_paging();

    // Initialize the stack and region manager
    stacks::init(KERNEL_STACKS_BASE, KERNEL_STACKS_LIMIT)
        .expect("Failed to initialize kernel stacks");
    heap_region::init(KERNEL_HEAP_BASE, KERNEL_HEAP_LIMIT)
        .expect("Failed to initialize heap regions");
}

// We need a way to manipulate the active page table.
