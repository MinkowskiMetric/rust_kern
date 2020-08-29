#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(rust_kern::test_runner)]
#![reexport_test_harness_main = "test_main"]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use rust_kern::println;

entry_point!(kernel_main);

#[cfg(test)]
fn run_tests() -> ! {
    test_main();

    loop {}
}

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Starting kernel...");

    #[cfg(test)]
    rust_kern::init::start_cpu0(run_tests);

    #[cfg(not(test))]
    rust_kern::init::start_cpu0(rust_kern::init::idle_proc);
    /*x86_64::instructions::interrupts::int3();

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
    rust_kern::test_panic_handler(info)
}
