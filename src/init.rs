use crate::gdt;
use crate::idt;
use crate::paging;
use crate::physmem;
use crate::println;
use bootloader::BootInfo;
use core::panic::PanicInfo;

#[no_mangle]
#[cfg(not(test))]
pub unsafe extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    println!("Starting kernel...");

    gdt::init();
    idt::init();

    physmem::init(boot_info);

    paging::init(boot_info);

    println!("{} used frames", physmem::used_frames());
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
    println!("{} free frames", physmem::free_frames());

    loop {}

    /*println!("Starting kernel...");

    #[cfg(test)]
    rust_kern::init::start_cpu0(run_tests);

    #[cfg(not(test))]
    rust_kern::init::start_cpu0(rust_kern::init::idle_proc);
    x86_64::instructions::interrupts::int3();

    test_main();

    loop {}*/
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
