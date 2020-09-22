#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(rust_kern::test_runner)]
#![reexport_test_harness_main = "test_main"]

use bootloader::BootInfo;
use rust_kern::println;

#[test_case]
fn test_println() {
    println!("test_println output");
}

fn idle_loop() -> ! {
    todo!("BIG IDLE: This would be the idle loop")
}

fn run_tests() -> ! {
    test_main();
    idle_loop();
}

#[no_mangle]
pub unsafe extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    rust_kern::init::kstart(boot_info, run_tests)
}
