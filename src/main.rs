#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(rust_kern::test_runner)]
#![reexport_test_harness_main = "test_main"]

use bootloader::BootInfo;

extern crate rust_kern;

#[cfg(not(test))]
fn kmain() -> ! {
    rust_kern::init::idle_loop()
}

#[cfg(test)]
fn run_tests() -> ! {
    test_main();
    rust_kern::init::idle_loop()
}

#[no_mangle]
#[cfg(not(test))]
pub unsafe extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    rust_kern::init::kstart(boot_info, kmain)
}

/// Entry point for `cargo test`
#[cfg(test)]
#[no_mangle]
pub unsafe extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    rust_kern::init::kstart(boot_info, run_tests)
}
