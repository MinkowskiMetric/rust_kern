use crate::physmem;
use bootloader::BootInfo;
use core::ops::{Deref, DerefMut};
use spin::{Mutex, MutexGuard};
use x86::{controlregs, tlb};

pub use crate::physmem::{page_align_down, page_align_up, Frame, PAGE_SIZE};

use table::{p1_index, p2_index, p3_index, p4_index};
pub use table::{HierarchyLevel, PageTable, PageTableIndex, PageTableLevel, L1, L2, L3, L4};

pub use heap_region::{allocate_kernel_stack, allocate_region, KernelStack, Region};
pub use mapper::{Mapper, MapperFlush, MapperFlushAll};

mod heap_region;
mod kernel_stack;
mod mapper;
mod page_entry;
mod table;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    NotMapped,
    NoRegionAddressSpaceAvailable,
    OutOfMemory,
    InvalidStack,
    InvalidRegion,
}

pub type Result<T> = core::result::Result<T, MemoryError>;

pub const FIRST_KERNEL_PML4: PageTableIndex = p4_index(0xffff_8000_0000_0000);
pub const KERNEL_PML4: PageTableIndex = p4_index(0xffff_8000_0000_0000);
pub const IDENTITY_MAP_PML4: PageTableIndex = p4_index(IDENTITY_MAP_REGION);
pub const KERNEL_DATA_PML4: PageTableIndex = p4_index(KERNEL_HEAP_BASE);

// We're going to use a whole PML4 entry to identity map memory. For now we will only map the first 4GB
pub const IDENTITY_MAP_REGION: usize = 0xffff_8080_0000_0000;

// Allow 3GB of kernel address space for kernel heap
pub const KERNEL_HEAP_BASE: usize = 0xffff_ff80_0000_0000;
pub const KERNEL_HEAP_LIMIT: usize = 0xffff_ff80_c000_0000;

pub const DEFAULT_KERNEL_STACK_PAGES: usize = 8;

pub struct ActivePageTable<'a> {
    #[allow(dead_code)]
    guard: MutexGuard<'a, ()>,
    mapper: Mapper,
}

impl<'a> ActivePageTable<'a> {
    pub fn flush(&self, addr: usize) {
        unsafe { tlb::flush(addr) };
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

pub unsafe fn lock_page_table() -> ActivePageTable<'static> {
    static PAGE_LOCK: Mutex<()> = Mutex::new(());

    let guard = PAGE_LOCK.lock();

    ActivePageTable {
        guard,
        mapper: Mapper::new(Frame::containing_address(controlregs::cr3() as usize)),
    }
}

// The bootloader sets us an amazing challenge - it doesn't tell us where in physical memory
// it has loaded the kernel, or where the bootloader stack is.
// That info is sort of in the memory map, but we need to figure out what is going on.

// For the kernel, we can work it out because we know where the kernel is mapped,
// and we can use that to copy the page table entries.

unsafe fn copy_boot_mapping(
    boot_p4_table: &PageTable<L4>,
    init_p4_table: &mut PageTable<L4>,
    start: usize,
    end: usize,
    flags: page_entry::PresentPageFlags,
) -> Result<()> {
    let mut virt_page = start;
    while virt_page < end {
        let init_p1_table = init_p4_table
            .create_next_table(p4_index(virt_page))?
            .create_next_table(p3_index(virt_page))?
            .create_next_table(p2_index(virt_page))?;

        let boot_p1_table = boot_p4_table
            .next_table(p4_index(virt_page))
            .unwrap()
            .next_table(p3_index(virt_page))
            .unwrap()
            .next_table(p2_index(virt_page))
            .unwrap();

        let boot_p1_entry = boot_p1_table[p1_index(virt_page)]
            .present()
            .expect("Expected present page in boot mapping");

        init_p1_table[p1_index(virt_page)] =
            page_entry::RawPresentPte::from_frame_and_flags(boot_p1_entry.frame(), flags).into();

        virt_page += PAGE_SIZE;
    }

    Ok(())
}

pub const HUGE_PAGE_SIZE: usize = PAGE_SIZE * 512;
pub const IDENTITY_MAP_SIZE: usize = 0x1_0000_0000;

unsafe fn prepare_identity_mapping(init_p4_table: &mut PageTable<L4>) -> Result<()> {
    use x86::cpuid::*;

    if CpuId::new()
        .get_extended_function_info()
        .unwrap()
        .has_1gib_pages()
    {
        todo!("This would be much easier if we supported 1gib pages");
    } else {
        // Identity map the first 4gib of physical address space. This will take a bunch of pages
        // but should all fit in a single PML4 entry
        assert_eq!(
            p4_index(IDENTITY_MAP_REGION + 0xffff_ffff),
            IDENTITY_MAP_PML4,
            "Identity map region does not fit in a single PML4 entry"
        );

        let p3_table = init_p4_table.create_next_table(p4_index(IDENTITY_MAP_REGION))?;
        let mut va_pos = IDENTITY_MAP_REGION;
        let va_limit = IDENTITY_MAP_REGION + IDENTITY_MAP_SIZE;

        let mut current_p3_index = p3_index(va_pos);
        let mut current_p2_table = p3_table.create_next_table(current_p3_index)?;

        while va_pos < va_limit {
            if p3_index(va_pos) != current_p3_index {
                current_p3_index = p3_index(va_pos);
                current_p2_table = p3_table.create_next_table(current_p3_index)?;
            }

            let phys_pos = va_pos - IDENTITY_MAP_REGION;
            let frame = Frame::containing_address(phys_pos);

            current_p2_table[p2_index(va_pos)] = page_entry::RawPresentPte::from_frame_and_flags(
                frame,
                page_entry::PresentPageFlags::WRITABLE
                    | page_entry::PresentPageFlags::HUGE_PAGE
                    | page_entry::PresentPageFlags::NO_EXECUTE
                    | page_entry::PresentPageFlags::GLOBAL,
            )
            .into();
            va_pos += HUGE_PAGE_SIZE;
        }
    }

    Ok(())
}

pub fn phys_to_virt_addr(phys_addr: usize, length: usize) -> usize {
    assert!(phys_addr + length < IDENTITY_MAP_SIZE);
    phys_addr + IDENTITY_MAP_REGION
}

pub fn phys_to_virt<T>(phys_addr: usize) -> *const T {
    phys_to_virt_addr(phys_addr, core::mem::size_of::<T>()) as *const T
}

pub fn phys_to_virt_mut<T>(phys_addr: usize) -> *mut T {
    phys_to_virt_addr(phys_addr, core::mem::size_of::<T>()) as *mut T
}

pub unsafe fn pre_init(boot_info: &BootInfo) {
    assert_eq!(
        boot_info.physical_memory_offset as usize, IDENTITY_MAP_REGION,
        "Bootloader has not mapped identity memory in the right place"
    );
}

pub unsafe fn init(cpuid: usize) -> usize {
    extern "C" {
        static __kernel_start: u8;
        static __text_start: u8;
        static __text_end: u8;
        static __rodata_start: u8;
        static __rodata_end: u8;
        static __data_start: u8;
        static __data_end: u8;
        static __tdata_start: u8;
        static __tdata_end: u8;
        static __tbss_start: u8;
        static __tbss_end: u8;
        static __bss_start: u8;
        static __bss_end: u8;
        static __kernel_end: u8;
    };

    let kernel_start = page_align_down(&__kernel_start as *const u8 as usize);
    let kernel_end = page_align_up(&__kernel_end as *const u8 as usize);

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

    // How do we get hold of the bootloader page table. Fortunately, the bootloader identity maps
    // enough physical memory that we can access it directly like this.
    let bootloader_page_table = &*phys_to_virt(controlregs::cr3() as usize);

    // Allocate a new page table
    let init_page_table_phys =
        physmem::allocate_kernel_frame().expect("cannot allocate early page directory");
    let init_page_table = &mut *phys_to_virt_mut(init_page_table_phys.physical_address());

    prepare_identity_mapping(init_page_table).expect("Failed to initialize identity mapping");

    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        &__text_start as *const u8 as usize,
        &__text_end as *const u8 as usize,
        page_entry::PresentPageFlags::GLOBAL,
    )
    .expect("Failed to create initial mapping");
    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        &__rodata_start as *const u8 as usize,
        &__rodata_end as *const u8 as usize,
        page_entry::PresentPageFlags::GLOBAL | page_entry::PresentPageFlags::NO_EXECUTE,
    )
    .expect("Failed to create initial mapping");
    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        &__data_start as *const u8 as usize,
        &__data_end as *const u8 as usize,
        page_entry::PresentPageFlags::GLOBAL
            | page_entry::PresentPageFlags::NO_EXECUTE
            | page_entry::PresentPageFlags::WRITABLE,
    )
    .expect("Failed to create initial mapping");
    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        &__tdata_start as *const u8 as usize,
        &__tdata_end as *const u8 as usize,
        page_entry::PresentPageFlags::GLOBAL | page_entry::PresentPageFlags::NO_EXECUTE,
    )
    .expect("Failed to create initial mapping");
    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        &__tbss_start as *const u8 as usize,
        &__tbss_end as *const u8 as usize,
        page_entry::PresentPageFlags::GLOBAL | page_entry::PresentPageFlags::NO_EXECUTE,
    )
    .expect("Failed to create initial mapping");
    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        &__bss_start as *const u8 as usize,
        &__bss_end as *const u8 as usize,
        page_entry::PresentPageFlags::GLOBAL
            | page_entry::PresentPageFlags::NO_EXECUTE
            | page_entry::PresentPageFlags::WRITABLE,
    )
    .expect("Failed to create initial mapping");
    copy_boot_mapping(
        bootloader_page_table,
        init_page_table,
        boot_stack_start,
        boot_stack_end,
        page_entry::PresentPageFlags::NO_EXECUTE | page_entry::PresentPageFlags::WRITABLE,
    )
    .expect("Failed to create initial mapping");

    // Switch to the page table
    controlregs::cr3_write(init_page_table_phys.physical_address() as u64);

    // Initialize the region manager
    heap_region::init(KERNEL_HEAP_BASE, KERNEL_HEAP_LIMIT);

    initialize_tcb(cpuid).expect("Failed to initialize tcb for CPU")
}

unsafe fn initialize_tcb(_cpuid: usize) -> Result<usize> {
    extern "C" {
        static mut __tdata_start: u8;
        static mut __tdata_end: u8;
        static mut __tbss_start: u8;
        static mut __tbss_end: u8;
    }

    let tcb_start_addr = &__tdata_start as *const _ as usize;
    let tcb_end_addr = &__tbss_end as *const _ as usize;
    let per_cpu_size = page_align_up(tcb_end_addr - tcb_start_addr);
    let tbss_offset = &__tbss_start as *const _ as usize - tcb_start_addr;

    let slot_region = allocate_region(per_cpu_size / PAGE_SIZE)?;
    let slot_start_addr = slot_region.start();
    // The region may be too big. No matter, just use the start of it
    let slot_limit_addr = slot_region.start() + per_cpu_size;

    {
        // Leak the region - we will never free it
        use core::mem::ManuallyDrop;
        let _ = ManuallyDrop::new(slot_region);
    }

    core::ptr::copy(
        &__tdata_start as *const u8,
        slot_start_addr as *mut u8,
        tbss_offset,
    );
    core::ptr::write_bytes(
        (slot_start_addr + tbss_offset) as *mut u8,
        0,
        per_cpu_size - tbss_offset,
    );

    let tcb_offset = slot_limit_addr - core::mem::size_of::<usize>();
    *(tcb_offset as *mut usize) = slot_limit_addr;

    Ok(tcb_offset)
}
