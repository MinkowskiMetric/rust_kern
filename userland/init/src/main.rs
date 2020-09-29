#![no_std]
#![no_main]
#![feature(asm)]
#![feature(start)]

use core::panic::PanicInfo;

fn hello() -> isize {
    75
}

#[start]
#[no_mangle]
extern "C" fn _start(_argc: isize, _argv: *const *const u8) -> isize {
    unsafe {
        asm!(
            "cli",
            "hlt",
        );
    }

    hello()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop { }
}