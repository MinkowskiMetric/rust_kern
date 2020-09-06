use crate::physmem;
use crate::types::VirtualAddress;
use bootloader::BootInfo;
use x86::controlregs;

pub use crate::physmem::PAGE_SIZE;

pub use crate::types::{PageFlags, PageTable, PageTableEntry};

// The bootloader sets us an amazing challenge - it doesn't tell us where in physical memory
// it has loaded the kernel, or where the bootloader stack is.
// That info is sort of in the memory map, but we need to figure out what is going on.

// For the kernel, we can work it out because we know where the kernel is mapped,
// and we can use that to copy the page table entries.

pub fn page_align_down(addr: u64) -> u64 {
    addr & !(PAGE_SIZE - 1)
}

pub fn page_align_up(addr: u64) -> u64 {
    page_align_down(addr + PAGE_SIZE - 1)
}

unsafe fn copy_boot_mapping(
    boot_info: &BootInfo,
    bootloader_page_table: &PageTable,
    init_page_table: &mut PageTable,
    start: VirtualAddress,
    end: VirtualAddress,
) {
    let mut virt_page = start;
    while virt_page < end {
        let p4_index = virt_page.p4_index();
        let init_p3_table: &mut PageTable = if init_page_table[p4_index]
            .flags()
            .contains(PageFlags::PRESENT)
        {
            &mut *(init_page_table[p4_index].addr() + boot_info.physical_memory_offset).as_mut_ptr()
        } else {
            let page_table_phys_addr =
                physmem::allocate_frame().expect("cannot allocate early page directory");
            let page_table = &mut *((page_table_phys_addr + boot_info.physical_memory_offset)
                .as_u64() as *mut PageTable);

            init_page_table[p4_index] = PageTableEntry::from_addr_and_flags(
                page_table_phys_addr,
                PageFlags::PRESENT | PageFlags::WRITABLE,
            );

            page_table
        };

        assert!(bootloader_page_table[p4_index]
            .flags()
            .contains(PageFlags::PRESENT));
        assert!(!bootloader_page_table[p4_index]
            .flags()
            .contains(PageFlags::HUGE_PAGE));
        let bootloader_p3_table: &PageTable =
            &*(bootloader_page_table[p4_index].addr() + boot_info.physical_memory_offset).as_ptr();

        let p3_index = virt_page.p3_index();
        let init_p2_table: &mut PageTable =
            if init_p3_table[p3_index].flags().contains(PageFlags::PRESENT) {
                &mut *(init_p3_table[p3_index].addr() + boot_info.physical_memory_offset)
                    .as_mut_ptr()
            } else {
                let page_table_phys_addr =
                    physmem::allocate_frame().expect("cannot allocate early page directory");
                let page_table = &mut *((page_table_phys_addr + boot_info.physical_memory_offset)
                    .as_u64() as *mut PageTable);

                init_p3_table[p3_index] = PageTableEntry::from_addr_and_flags(
                    page_table_phys_addr,
                    PageFlags::PRESENT | PageFlags::WRITABLE,
                );

                page_table
            };

        assert!(bootloader_p3_table[p3_index]
            .flags()
            .contains(PageFlags::PRESENT));
        assert!(!bootloader_p3_table[p3_index]
            .flags()
            .contains(PageFlags::HUGE_PAGE));
        let bootloader_p2_table: &PageTable =
            &*(bootloader_p3_table[p3_index].addr() + boot_info.physical_memory_offset).as_ptr();

        let p2_index = virt_page.p2_index();
        let init_p1_table: &mut PageTable =
            if init_p2_table[p2_index].flags().contains(PageFlags::PRESENT) {
                &mut *(init_p2_table[p2_index].addr() + boot_info.physical_memory_offset)
                    .as_mut_ptr()
            } else {
                let page_table_phys_addr =
                    physmem::allocate_frame().expect("cannot allocate early page directory");
                let page_table = &mut *((page_table_phys_addr + boot_info.physical_memory_offset)
                    .as_u64() as *mut PageTable);

                init_p2_table[p2_index] = PageTableEntry::from_addr_and_flags(
                    page_table_phys_addr,
                    PageFlags::PRESENT | PageFlags::WRITABLE,
                );

                page_table
            };

        assert!(bootloader_p2_table[p2_index]
            .flags()
            .contains(PageFlags::PRESENT));
        assert!(!bootloader_p2_table[p2_index]
            .flags()
            .contains(PageFlags::HUGE_PAGE));
        let bootloader_p1_table: &PageTable =
            &*(bootloader_p2_table[p2_index].addr() + boot_info.physical_memory_offset).as_ptr();

        let p1_index = virt_page.p1_index();
        assert!(bootloader_p1_table[p1_index]
            .flags()
            .contains(PageFlags::PRESENT));
        assert!(!bootloader_p1_table[p1_index]
            .flags()
            .contains(PageFlags::HUGE_PAGE));
        let p1_addr = bootloader_p1_table[p1_index].addr();

        init_p1_table[p1_index] =
            PageTableEntry::from_addr_and_flags(p1_addr, PageFlags::PRESENT | PageFlags::WRITABLE);

        virt_page += PAGE_SIZE;
    }
}

pub unsafe fn init(boot_info: &BootInfo) {
    extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    };

    let kernel_start =
        VirtualAddress::new((&__kernel_start as *const u8) as u64).align_down(PAGE_SIZE);
    let kernel_end = VirtualAddress::new((&__kernel_end as *const u8) as u64).align_up(PAGE_SIZE);

    // Allow for the "guard page" on the stack
    let boot_stack_start = VirtualAddress::new(0x10000000) + PAGE_SIZE;
    let boot_stack_end = boot_stack_start + (PAGE_SIZE * 8);

    // How do we get hold of the bootloader page table. Fortunately, the bootloader identity maps
    // enough physical memory that we can access it directly like this.
    let bootloader_page_table =
        &*((controlregs::cr3() + boot_info.physical_memory_offset) as *const PageTable);

    // Allocate a new page table
    let init_page_table_phys =
        physmem::allocate_frame().expect("cannot allocate early page directory");
    let init_page_table = &mut *((init_page_table_phys + boot_info.physical_memory_offset).as_u64()
        as *mut PageTable);

    copy_boot_mapping(
        boot_info,
        bootloader_page_table,
        init_page_table,
        kernel_start,
        kernel_end,
    );
    copy_boot_mapping(
        boot_info,
        bootloader_page_table,
        init_page_table,
        boot_stack_start,
        boot_stack_end,
    );

    // We need to copy an additional mapping for VGA memory since the logger uses it
    // TODOTODOTODO - should fix this
    copy_boot_mapping(
        boot_info,
        bootloader_page_table,
        init_page_table,
        VirtualAddress::new(0xb8000),
        VirtualAddress::new(0xb9000),
    );

    // Switch to the page table
    controlregs::cr3_write(init_page_table_phys.as_u64());
}
