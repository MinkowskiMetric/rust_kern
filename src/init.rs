use crate::acpi;
use crate::allocator;
use crate::devices;
use crate::gdt;
use crate::idt;
use crate::paging;
use crate::physmem;
use crate::println;
use alloc::vec::Vec;
use bootloader::{bootinfo::MemoryRegion, BootInfo};
use core::panic::PanicInfo;

pub unsafe fn kstart(boot_info: &'static BootInfo, func: impl FnOnce() -> ! + 'static) -> ! {
    paging::pre_init(boot_info);

    println!("Starting kernel...");

    gdt::init();
    idt::early_init();

    physmem::early_init(boot_info.memory_map.iter());

    // Initialize the allocator before paging. The allocator uses a small internal buffer which
    // gives us enough working heap to allocate during paging initialization
    allocator::init();

    // Now that we have a functioning heap, we can make a copy of the boot memory map.
    // Eventually we will pass this to the paging manager instead of the one from the bootloader
    let memory_map: Vec<_> = boot_info.memory_map.iter().cloned().collect();

    let tcb_offset = paging::init(0);

    physmem::init_post_paging(memory_map.iter());

    // Once paging is up and running, we can allocate a new kernel stack
    // for what will become our idle thread
    let idle_thread_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate first kernel stack");
    let fault_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate fault stack");
    idle_thread_stack.switch_to_permanent(move |stack| {
        init_post_paging(stack, fault_stack, tcb_offset, memory_map, func);
    });
}

unsafe fn init_post_paging(
    idle_thread_stack: paging::KernelStack,
    fault_stack: paging::KernelStack,
    tcb_offset: usize,
    memory_map: Vec<MemoryRegion>,
    func: impl FnOnce() -> ! + 'static,
) -> ! {
    println!(
        "Running on our own stack! {:?} tcb: {:#x}",
        &idle_thread_stack as *const paging::KernelStack, tcb_offset,
    );

    gdt::init_post_paging(tcb_offset, &idle_thread_stack, &fault_stack);
    idt::init(true);

    physmem::init_reclaim(memory_map.iter());

    acpi::init_bsp();

    // At this point, memory is fully working and in our control. The next thing to do is to bring up
    // the basic hardware
    devices::init_bsp();

    func()
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    loop {}
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::test_panic_handler(info)
}
