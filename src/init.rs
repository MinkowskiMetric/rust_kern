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

    paging::init(boot_info);

    /*println!("{} used frames", physmem::used_frames());
    println!("{} free frames", physmem::free_frames());
    let _original_free_frames = physmem::free_frames();

    let alloc_frame = physmem::allocate_frame().unwrap();
    println!("Allocated frame {:?}", alloc_frame);

    println!("{} used frames", physmem::used_frames());
    println!("{} free frames", physmem::free_frames());

    let mut total_frames = 1;

    loop {
        if let Some(_frame) = physmem::allocate_frame() {
            total_frames += 1;
        } else {
            break;
        }
    }

    println!("Allocated {} frames", total_frames);
    println!("{} used frames", physmem::used_frames());
    println!("{} free frames", physmem::free_frames());*/

    // Once paging is up and running, we can allocate a new kernel stack
    // for what will become our idle thread
    let idle_thread_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate first kernel stack");
    idle_thread_stack.switch_to_permanent(|stack| init_post_paging(stack, memory_map));

    /*println!("Starting kernel...");

    #[cfg(test)]
    rust_kern::init::start_cpu0(run_tests);

    #[cfg(not(test))]
    rust_kern::init::start_cpu0(rust_kern::init::idle_proc);
    x86_64::instructions::interrupts::int3();

    test_main();

    loop {}*/
}

fn init_post_paging(idle_thread_stack: paging::KernelStack, memory_map: Vec<MemoryRegion>) -> ! {
    println!(
        "Running on our own stack! {:?}",
        &idle_thread_stack as *const paging::KernelStack
    );

    println!("MEM {:?}", memory_map);

    loop {}
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
