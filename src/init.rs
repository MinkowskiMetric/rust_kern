use crate::allocator;
use crate::gdt;
use crate::idt;
use crate::paging;
use crate::physmem;
use crate::println;
use alloc::vec::Vec;
use bootloader::{bootinfo::MemoryRegion, BootInfo};
use core::panic::PanicInfo;

#[no_mangle]
#[cfg(not(test))]
pub unsafe extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    paging::pre_init(boot_info);

    println!("Starting kernel...");

    gdt::init();
    idt::init();

    physmem::init(boot_info);

    // Initialize the allocator before paging. The allocator uses a small internal buffer which
    // gives us enough working heap to allocate during paging initialization
    allocator::init();

    // Now that we have a functioning heap, we can make a copy of the boot memory map.
    // Eventually we will pass this to the paging manager instead of the one from the bootloader
    let memory_map: Vec<_> = boot_info.memory_map.iter().cloned().collect();

    let tcb_offset = paging::init(0);

    // Once paging is up and running, we can allocate a new kernel stack
    // for what will become our idle thread
    let idle_thread_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate first kernel stack");
    let fault_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate fault stack");
    idle_thread_stack.switch_to_permanent(move |stack| {
        init_post_paging(stack, fault_stack, tcb_offset, memory_map)
    });
}

unsafe fn init_post_paging(
    idle_thread_stack: paging::KernelStack,
    fault_stack: paging::KernelStack,
    tcb_offset: usize,
    _memory_map: Vec<MemoryRegion>,
) -> ! {
    println!(
        "Running on our own stack! {:?} tcb: {:#x}",
        &idle_thread_stack as *const paging::KernelStack, tcb_offset,
    );

    gdt::init_post_paging(tcb_offset, &idle_thread_stack, &fault_stack);

    println!(
        "Before stress - heap size {} bytes - free size {} bytes",
        allocator::allocated_space(),
        allocator::free_space()
    );
    // Stress the heap a bit
    use alloc::vec;
    let mut a = vec![box 17; 1024];
    for i in 0..3 {
        println!(
            "Iteration {} - heap size {} bytes - free size {} bytes",
            i,
            allocator::allocated_space(),
            allocator::free_space()
        );
        let b: Vec<_> = a.iter().cloned().collect();
        a.extend(b);
    }

    println!(
        "Before decimate - heap size {} bytes - free size {} bytes",
        allocator::allocated_space(),
        allocator::free_space()
    );
    a = a.iter().step_by(2).cloned().collect();
    println!(
        "After decimate - heap size {} bytes - free size {} bytes",
        allocator::allocated_space(),
        allocator::free_space()
    );

    core::mem::drop(a);
    println!(
        "After drop - heap size {} bytes - free size {} bytes",
        allocator::allocated_space(),
        allocator::free_space()
    );

    todo!("This would be the idle loop");
    //loop {}
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
